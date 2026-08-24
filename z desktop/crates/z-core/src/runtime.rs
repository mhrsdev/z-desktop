//! Agent Runtime — the headless heart of Z Desktop.
//!
//! Owns threads and turns. Commands arrive on one channel; events leave on
//! another. Each turn runs on its own worker thread so the command loop stays
//! responsive to cancellation and approvals while a turn streams.

use crate::journal::{Journal, JournalKind, Record, RecordDraft};
use crate::reducer::{TaskStatus, TaskStore, TasksView, TASKS_SEGMENT};
use crate::{provider, repo::RepoIndex, router, tools};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use z_protocol::{Command, Event, Id, ProviderConfig, Risk, ThreadInfo};

// ---------------------------------------------------------------------------
// Conversation model (persisted)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToolCall {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
    pub ok: Option<bool>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub role: z_protocol::Role,
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<StoredToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: Id,
    pub title: String,
    pub messages: Vec<StoredMessage>,
    /// Last message-activity time (ms since epoch). Drives ThreadList recency
    /// ordering. Default keeps older files loadable (ADR-0018 additive).
    #[serde(default)]
    pub updated_ms: u64,
}

impl Thread {
    fn new(id: Id) -> Self {
        Self { id, title: "New chat".into(), messages: Vec::new(), updated_ms: 0 }
    }
}

/// Wall-clock milliseconds since the Unix epoch (best-effort; 0 pre-1970).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Approval gate — workers park here; the UI resolves.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ApprovalGate {
    pending: Mutex<HashMap<Id, bool>>, // call_id -> decision (true = approved)
    signal: Condvar,
}

impl ApprovalGate {
    /// Block until the UI decides or the caller gives up (returns None).
    fn wait(&self, call_id: &str, timeout: std::time::Duration) -> Option<bool> {
        let deadline = std::time::Instant::now() + timeout;
        let mut guard = self.pending.lock().unwrap();
        loop {
            if let Some(decision) = guard.get(call_id) {
                return Some(*decision);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let (g, _) = self.signal.wait_timeout(guard, deadline - now).unwrap();
            guard = g;
        }
    }

    fn resolve(&self, call_id: &str, approved: bool) {
        self.pending.lock().unwrap().insert(call_id.to_string(), approved);
        self.signal.notify_all();
    }

    fn clear(&self, call_id: &str) {
        self.pending.lock().unwrap().remove(call_id);
    }
}

// ---------------------------------------------------------------------------
// Shared runtime state
// ---------------------------------------------------------------------------

struct Shared {
    provider: Mutex<Option<Arc<dyn provider::Provider>>>,
    provider_label: Mutex<String>,
    project_root: Mutex<Option<PathBuf>>,
    index: Mutex<Option<RepoIndex>>,
    gate: ApprovalGate,
    cancelled: Mutex<std::collections::HashSet<Id>>, // thread ids asked to cancel
    // Steering queue per thread: user text enqueued while a turn is running,
    // drained between tool rounds. Bounded so a runaway client cannot grow it
    // without limit; overflow keeps the newest messages.
    steering: Mutex<HashMap<Id, VecDeque<String>>>,
    // set-003 (ADR-0011): snapshot-cached settings. Readers clone the inner
    // Arc once per turn start and never hold this lock during the turn.
    settings: Mutex<Arc<crate::settings::Snapshot>>,
    // edit-016/017 (ADR-0010 §(5c)): exclusive write grants, keyed by
    // canonical path, valued by the owning thread id. Acquired before every
    // Write-risk tool call; overlap by another thread is rejected at acquire
    // time. ponytail: unbounded map OK at personal scale — entries live only
    // for the duration of one call.
    write_grants: Mutex<HashMap<String, String>>,
}

/// Upper bound on queued steering texts per thread. Beyond this, the oldest
/// queued text is dropped to make room (the user's latest intent matters most).
const STEERING_QUEUE_CAP: usize = 16;

pub struct Runtime {
    shared: Arc<Shared>,
    data_dir: PathBuf,
    threads: Mutex<HashMap<Id, Thread>>,
    event_tx: Sender<Event>,
    cmd_rx: Receiver<(u64, Command)>,
    // jour-024/jour-029: the runtime lifecycle journal (`journal/runtime.jsonl`).
    // Deliberately NOT inside `Shared`: only the command loop (and, for
    // MessagePersisted, the turn worker) ever appends to it, so keeping it out
    // of Shared avoids any lock-order coupling with the provider/project locks.
    // It is an `Option<Arc<Mutex<Journal>>>` because (a) run_turn workers need
    // shared ownership across the spawn boundary — same pattern as `shared` —
    // and (b) journaling is best-effort by design: when the file cannot be
    // opened (missing dir, permissions), the runtime runs on without it
    // instead of failing every command.
    journal: Option<Arc<Mutex<Journal>>>,
    // core-025 partial: id of the most recently modified thread file seen by
    // the startup restore loop (by fs mtime). Startup wiring reads this later.
    most_recent_restored: Option<Id>,
    // core-026: thread files that failed to parse at startup, as
    // "{filename}: {error}". Startup-only; surfaced as read-only "[corrupt]"
    // ghosts in ThreadList (ADR-0016 CaptureFailed philosophy).
    corrupt_threads: Vec<String>,
}

/// Open the runtime lifecycle journal at `<data_dir>/journal/runtime.jsonl`
/// (jour-024), resuming the sequence after the highest seq already on disk so
/// a restart never reuses sequence numbers. Returns `None` when the journal
/// cannot be used — journaling must never prevent the runtime from serving,
/// so an unwritable location degrades to "no journal" (warned, not fatal).
fn open_runtime_journal(data_dir: &std::path::Path) -> Option<Arc<Mutex<Journal>>> {
    let dir = data_dir.join("journal");
    let path = dir.join("runtime.jsonl");
    let last_seq = if path.exists() {
        match Journal::replay(&path) {
            Ok(records) => records.last().map(|r| r.seq).unwrap_or(0),
            Err(e) => {
                // A tail we cannot replay would corrupt sequence continuity;
                // fail loud-ish (warn) and disable rather than duplicate seqs.
                log::warn!("journal: disabling runtime journal, replay failed: {e}");
                return None;
            }
        }
    } else {
        0 // no segment yet: start from scratch
    };
    match Journal::open_resuming(&dir, "runtime", last_seq) {
        Ok(journal) => Some(Arc::new(Mutex::new(journal))),
        Err(e) => {
            log::warn!("journal: disabling runtime journal: {e}");
            None
        }
    }
}

/// Where sessions/config live. `Z_DESKTOP_DATA` overrides; default is `data/`
/// beside the working directory so a dev checkout is self-contained.
pub fn data_dir() -> PathBuf {
    std::env::var_os("Z_DESKTOP_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// Load every parseable `<dir>/threads/*.json` into memory. Also reports the
/// id of the most recently modified thread file by fs mtime — core-025
/// partial, so startup wiring can auto-open it. A file whose mtime cannot be
/// read simply does not compete for "most recent"; corrupt JSON is skipped
/// but reported (core-026) as `"{filename}: {error}"` so the UI can surface
/// the gap instead of silently losing data.
fn restore_threads(dir: &std::path::Path) -> (HashMap<Id, Thread>, Option<Id>, Vec<String>) {
    let mut threads = HashMap::new();
    let mut most_recent: Option<Id> = None;
    let mut corrupt: Vec<String> = Vec::new();
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    if let Ok(entries) = std::fs::read_dir(dir.join("threads")) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(entry.path())
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str::<Thread>(&s).map_err(|e| e.to_string()))
            {
                Ok(thread) => {
                    // Only a file we actually restored can be "most recent" —
                    // startup auto-open must never point at corrupt data.
                    if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
                        if mtime > newest {
                            newest = mtime;
                            most_recent =
                                entry.path().file_stem().and_then(|s| s.to_str()).map(Id::from);
                        }
                    }
                    threads.insert(thread.id.clone(), thread);
                }
                Err(e) => {
                    log::warn!("skipping unreadable session {:?}: {e}", entry.path());
                    let fname = entry.file_name().to_string_lossy().into_owned();
                    corrupt.push(format!("{fname}: {e}"));
                }
            }
        }
    }
    (threads, most_recent, corrupt)
}

impl Runtime {
    pub fn new(event_tx: Sender<Event>, cmd_rx: Receiver<(u64, Command)>) -> Self {
        let data_dir = data_dir();
        let _ = std::fs::create_dir_all(data_dir.join("threads"));
        // set-002/003 (ADR-0011): load settings once into the shared snapshot;
        // hand-edited files apply on relaunch, SetSetting swaps the Arc later.
        let settings = Arc::new(crate::settings::Snapshot::new(crate::settings::load(&data_dir)));
        // Restore persisted sessions; a corrupt file is skipped, not fatal.
        // Second return value: id of the most recently modified thread file
        // (core-025 partial, consumed by `most_recent_thread`). Third: files
        // that failed to parse (core-026), warned once here at startup.
        let (threads, most_recent_restored, corrupt_threads) = restore_threads(&data_dir);
        if !corrupt_threads.is_empty() {
            log::warn!("corrupt thread file(s) kept visible as [corrupt] ghosts: {corrupt_threads:?}");
        }
        log::info!("restored {} thread(s)", threads.len());
        // jour-024: resume the lifecycle journal across restarts (best-effort).
        let journal = open_runtime_journal(&data_dir);
        Self {
            shared: Arc::new(Shared {
                provider: Mutex::new(None),
                provider_label: Mutex::new("no provider configured".into()),
                project_root: Mutex::new(None),
                index: Mutex::new(None),
                gate: ApprovalGate::default(),
                cancelled: Mutex::new(std::collections::HashSet::new()),
                steering: Mutex::new(HashMap::new()),
                settings: Mutex::new(settings),
                write_grants: Mutex::new(HashMap::new()),
            }),
            data_dir,
            threads: Mutex::new(threads),
            event_tx,
            cmd_rx,
            journal,
            most_recent_restored,
            corrupt_threads,
        }
    }

    /// core-025 partial (restore-most-recent): id of the most recently
    /// modified thread file at restore time, for app startup auto-open.
    pub fn most_recent_thread(&self) -> Option<String> {
        self.most_recent_restored.clone()
    }

    /// core-026: thread files that failed to parse at startup, as
    /// `"{filename}: {error}"`. Startup-only snapshot; each entry is also
    /// surfaced as a read-only `[corrupt]` ghost in ThreadList until its
    /// file is deleted.
    pub fn corrupt_threads(&self) -> &[String] {
        &self.corrupt_threads
    }

    /// Run the command loop until the channel closes (app shutdown).
    pub fn serve(mut self) {
        while let Ok((command_id, command)) = self.cmd_rx.recv() {
            let _ = self.event_tx.send(Event::Accepted { command_id });
            // jour-024: durable breadcrumb for every inbound command, before
            // dispatch. Never fatal (see journal_command).
            self.journal_command(&command);
            match command {
                Command::ConfigureProvider { config } => self.configure_provider(config),
                Command::OpenProject { path } => self.open_project(path),
                Command::SendMessage { thread_id, text } => self.start_turn(thread_id, text),
                Command::EnqueueMessage { thread_id, text } => self.enqueue_message(thread_id, text),
                Command::CancelTurn { thread_id } => {
                    self.shared.cancelled.lock().unwrap().insert(thread_id.clone());
                    // A cancelled turn must not replay stale steering on its
                    // next turn: drop anything queued for this thread.
                    let mut steering = self.shared.steering.lock().unwrap();
                    if let Some(queue) = steering.get_mut(&thread_id) {
                        queue.clear();
                    }
                }
                Command::ResolveApproval { call_id, approved } => {
                    self.shared.gate.resolve(&call_id, approved);
                }
                // core-021/022: thread management. Every mutation re-emits a
                // fresh ThreadList so the UI never drifts from runtime state.
                Command::ListThreads => self.send_thread_list(),
                Command::RenameThread { thread_id, title } => self.rename_thread(thread_id, title),
                Command::DeleteThread { thread_id } => self.delete_thread(thread_id),
                Command::DuplicateThread { thread_id, new_id } => {
                    self.duplicate_thread(thread_id, new_id)
                }
                // ui-040: fold the journal's evidence for the UI badges.
                Command::GetEvidence { turn_id } => self.send_evidence(turn_id),
            }
        }
        log::info!("command channel closed; runtime stopping");
    }

    /// Queue user text for injection into a running turn (steering). The
    /// queue lives in Shared so the command loop never blocks on a busy
    /// worker; depth is reported back for the UI indicator.
    fn enqueue_message(&self, thread_id: Id, text: String) {
        if text.trim().is_empty() {
            return;
        }
        let depth = {
            let mut steering = self.shared.steering.lock().unwrap();
            let queue = steering.entry(thread_id.clone()).or_default();
            while queue.len() >= STEERING_QUEUE_CAP {
                queue.pop_front(); // keep newest intent under pressure
            }
            queue.push_back(text);
            queue.len() as u64
        };
        let _ = self
            .event_tx
            .send(Event::SteeringQueued { thread_id, depth });
    }

    /// jour-024: append one `CommandReceived` record per inbound command,
    /// before dispatch. Payloads are shape summaries only: message text is
    /// never recorded, and ConfigureProvider contributes field NAMES only —
    /// never API-key values. Failures are warned and swallowed.
    fn journal_command(&self, command: &Command) {
        if self.journal.is_none() {
            return;
        }
        let (thread_id, payload) = match command {
            Command::SendMessage { thread_id, .. } => (
                Some(thread_id.clone()),
                json!({ "command": "send_message", "thread_id": thread_id }),
            ),
            Command::EnqueueMessage { thread_id, .. } => (
                Some(thread_id.clone()),
                json!({ "command": "enqueue_message", "thread_id": thread_id }),
            ),
            Command::CancelTurn { thread_id } => (
                Some(thread_id.clone()),
                json!({ "command": "cancel_turn", "thread_id": thread_id }),
            ),
            Command::ResolveApproval { .. } => (None, json!({ "command": "resolve_approval" })),
            Command::OpenProject { .. } => (None, json!({ "command": "open_project" })),
            // core-021/022 thread management: shape-only breadcrumbs.
            Command::ListThreads => (None, json!({ "command": "list_threads" })),
            Command::RenameThread { thread_id, .. } => (
                Some(thread_id.clone()),
                json!({ "command": "rename_thread", "thread_id": thread_id }),
            ),
            Command::DeleteThread { thread_id } => (
                Some(thread_id.clone()),
                json!({ "command": "delete_thread", "thread_id": thread_id }),
            ),
            Command::DuplicateThread { thread_id, new_id } => (
                Some(thread_id.clone()),
                json!({ "command": "duplicate_thread", "thread_id": thread_id, "new_id": new_id }),
            ),
            Command::GetEvidence { .. } => (None, json!({ "command": "get_evidence" })),
            Command::ConfigureProvider { config } => (
                None,
                // Shape only: which configuration fields were sent, never
                // their values (api_key et al stay out of the journal).
                json!({
                    "command": "configure_provider",
                    "fields": config_field_names(config),
                }),
            ),
        };
        Runtime::journal_record(&self.journal, JournalKind::CommandReceived, thread_id, payload);
    }

    /// Best-effort append of one lifecycle record (jour-024/029). Journal
    /// failures are warned about and dropped: observability must never break
    /// a command or a turn. The lock is held for a single append only, so
    /// critical sections stay narrow even when called from turn workers.
    fn journal_record(
        journal: &Option<Arc<Mutex<Journal>>>,
        kind: JournalKind,
        thread_id: Option<String>,
        payload: serde_json::Value,
    ) {
        let Some(journal) = journal.as_ref() else { return };
        let draft = RecordDraft::new(kind, thread_id, payload);
        if let Err(e) = journal.lock().unwrap().append(draft) {
            log::warn!("journal: append failed: {e}");
        }
    }

    /// Take all queued steering texts for `thread_id` (leaves an empty
    /// queue entry behind). Called by turn workers between tool rounds.
    fn drain_steering(shared: &Shared, thread_id: &str) -> Vec<String> {
        let mut steering = shared.steering.lock().unwrap();
        match steering.get_mut(thread_id) {
            Some(queue) => queue.drain(..).collect(),
            None => Vec::new(),
        }
    }

    /// Combine drained steering texts into one appended user message.
    ///
    /// Combine gate (core-007): consecutive plain texts merge into a single
    /// user turn — one "User steering:" marker per drain, not one per queued
    /// message — so N rapid corrections cost one history entry, not N.
    fn combine_steering(texts: Vec<String>) -> Option<StoredMessage> {
        let joined = texts.join("\n");
        if joined.trim().is_empty() {
            return None;
        }
        Some(StoredMessage {
            role: z_protocol::Role::User,
            text: format!("User steering:\n{joined}"),
            tool_calls: Vec::new(),
        })
    }

    fn configure_provider(&self, config: ProviderConfig) {
        match provider::from_config(config.clone()) {
            Ok(p) => {
                let label = p.describe();
                *self.shared.provider_label.lock().unwrap() = label.clone();
                *self.shared.provider.lock().unwrap() = Some(Arc::from(p));
                let _ = self.event_tx.send(Event::ProviderStatus {
                    ok: true,
                    message: format!("provider ready: {label}"),
                });
            }
            Err(e) => {
                let _ = self.event_tx.send(Event::ProviderStatus { ok: false, message: e });
            }
        }
        // Persist BYOK config locally only.
        let path = self.data_dir.join("config.json");
        if let Ok(json) = serde_json::to_string_pretty(&config) {
            let _ = std::fs::write(path, json);
        }
    }

    fn open_project(&self, path: String) {
        let root = PathBuf::from(&path);
        if !root.is_dir() {
            let _ = self.event_tx.send(Event::ProviderStatus {
                ok: false,
                message: format!("not a directory: {path}"),
            });
            return;
        }
        let index = RepoIndex::open(&root);
        let files = index.file_count();
        let symbols = index.symbol_count();
        *self.shared.index.lock().unwrap() = Some(index);
        *self.shared.project_root.lock().unwrap() = Some(root);
        let _ = self.event_tx.send(Event::ProjectIndexed { path, files, symbols });
    }

    fn start_turn(&self, thread_id: Id, text: String) {
        // Create or reuse the thread; first user message becomes its title.
        let thread = {
            let mut threads = self.threads.lock().unwrap();
            let thread = threads.entry(thread_id.clone()).or_insert_with(|| Thread::new(thread_id.clone()));
            if thread.messages.is_empty() {
                let title: String = text.chars().take(48).collect();
                thread.title = if text.chars().count() > 48 { format!("{title}…") } else { title };
            }
            thread.updated_ms = now_ms(); // core-021 recency marker
            thread.messages.push(StoredMessage { role: z_protocol::Role::User, text, tool_calls: Vec::new() });
            let snapshot = thread.clone();
            self.persist(&snapshot);
            snapshot
        };

        let turn_id = crate::new_id("turn");
        let _ = self.event_tx.send(Event::TurnStarted { thread_id: thread.id.clone(), turn_id: turn_id.clone() });
        // jour-024: durable marker that this turn began executing. Written by
        // the command loop before the worker spawns, so its position relative
        // to CommandReceived/TurnFinished records is deterministic.
        Runtime::journal_record(
            &self.journal,
            JournalKind::TurnStarted,
            Some(thread.id.clone()),
            json!({ "turn_id": turn_id }),
        );

        // The turn runs on a worker so CancelTurn/ResolveApproval stay live.
        let shared = Arc::clone(&self.shared);
        let event_tx = self.event_tx.clone();
        let threads_lock = Arc::new(Mutex::new(())); // serialise history mutation
        let data_dir = self.data_dir.clone();
        // jour-029: the worker needs the SAME journal instance to append
        // MessagePersisted records; hand it an Arc clone exactly like `shared`
        // (a plain borrow cannot cross the spawn boundary).
        let journal = self.journal.clone();
        std::thread::Builder::new()
            .name("z-turn".into())
            .spawn(move || {
                run_turn(shared, event_tx, threads_lock, data_dir, journal, thread, turn_id);
            })
            .expect("could not spawn turn worker");
    }

    fn persist(&self, thread: &Thread) {
        let path = self.data_dir.join("threads").join(format!("{}.json", thread.id));
        if let Ok(json) = serde_json::to_string_pretty(thread) {
            let _ = std::fs::write(path, json);
        }
    }

    /// core-021: project the threads map into a recency-ordered ThreadList
    /// event. Most recent first; id ascending breaks ties deterministically.
    fn send_thread_list(&self) {
        let threads = self.threads.lock().unwrap();
        let mut infos: Vec<ThreadInfo> = threads
            .values()
            .map(|t| ThreadInfo {
                id: t.id.clone(),
                title: t.title.clone(),
                message_count: t.messages.len() as u64,
                updated_ms: t.updated_ms,
            })
            .collect();
        drop(threads);
        // core-026: unreadable startup files stay visible as read-only
        // "[corrupt]" ghost rows (title `[corrupt] {filename}`, 0 messages)
        // so the gap is honest, not silent (ADR-0016 CaptureFailed). Ghosts
        // sort last (updated_ms 0); deleting one just removes the file.
        for c in &self.corrupt_threads {
            let Some((fname, _)) = c.split_once(": ") else { continue };
            infos.push(ThreadInfo {
                id: fname.strip_suffix(".json").unwrap_or(fname).to_string(),
                title: format!("[corrupt] {fname}"),
                message_count: 0,
                updated_ms: 0,
            });
        }
        infos.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(a.id.cmp(&b.id)));
        let _ = self.event_tx.send(Event::ThreadList { threads: infos });
    }

    /// ui-040: fold the journal's evidence records into an EvidenceSummary for
    /// the UI badge strip. Read-only and best-effort: an unreadable journal is
    /// an empty summary, never an error — same degrade philosophy as every
    /// other journal consumer.
    fn send_evidence(&self, turn_id: Option<Id>) {
        let mut items = match crate::evidence::EvidenceView::fold(
            &self.data_dir.join("journal").join("runtime.jsonl"),
        ) {
            Ok(view) => view
                .items
                .into_iter()
                .filter(|e| match &turn_id {
                    Some(t) => e.turn_id == *t,
                    None => true,
                })
                .map(|e| z_protocol::EvidenceInfo {
                    id: e.id,
                    kind: match e.kind {
                        crate::evidence::EvidenceKind::Build => "build",
                        crate::evidence::EvidenceKind::Tests => "tests",
                        crate::evidence::EvidenceKind::Diff => "diff",
                        crate::evidence::EvidenceKind::Bench => "bench",
                        crate::evidence::EvidenceKind::Regression => "regression",
                    }
                    .to_string(),
                    ok: e.ok,
                    summary: e.summary,
                })
                .collect::<Vec<_>>(),
            Err(err) => {
                log::warn!("evidence: fold failed for GetEvidence: {err}");
                Vec::new()
            }
        };
        // Cap to the most recent 50 rows so a long-lived journal can't grow
        // this event without bound.
        if items.len() > 50 {
            items.drain(..items.len() - 50);
        }
        let _ = self.event_tx.send(Event::EvidenceSummary { items });
    }

    /// core-022: set a thread title (clamped to 120 chars), persist it.
    fn rename_thread(&self, thread_id: Id, title: String) {
        {
            let mut threads = self.threads.lock().unwrap();
            let Some(thread) = threads.get_mut(&thread_id) else { return };
            // ponytail: hard char clamp keeps UI rows single-line-ish; no
            // trimming/normalization until a real consumer asks for it.
            thread.title = title.chars().take(120).collect();
            let snapshot = thread.clone();
            self.persist(&snapshot);
        }
        self.send_thread_list();
    }

    /// core-022: remove a thread from memory and delete its file on disk.
    /// core-026: a [corrupt] ghost is not in memory; deleting it removes the
    /// unreadable file itself, which also drops the ghost from future lists.
    fn delete_thread(&mut self, thread_id: Id) {
        let known = self.threads.lock().unwrap().remove(&thread_id).is_some();
        if !known && !self.drop_corrupt_ghost(&thread_id) {
            return;
        }
        let path = self
            .data_dir
            .join("threads")
            .join(format!("{thread_id}.json"));
        if let Err(e) = std::fs::remove_file(&path) {
            log::warn!("delete_thread: could not remove {path:?}: {e}");
        }
        // core-023: tombstone so replay/audit (and ThreadsView-style reducers)
        // can see the deletion after the snapshot file is gone. Best-effort.
        Runtime::journal_record(
            &self.journal,
            JournalKind::ThreadDeleted,
            Some(thread_id.clone()),
            json!({ "thread_id": thread_id }),
        );
        self.send_thread_list();
    }

    /// core-026: drop `thread_id`'s entry from the corrupt list (matched by
    /// filename). False when it was never a corrupt ghost.
    fn drop_corrupt_ghost(&mut self, thread_id: &str) -> bool {
        let fname = format!("{thread_id}.json");
        match self.corrupt_threads.iter().position(|c| {
            c.split_once(": ").map_or(false, |(f, _)| f == fname)
        }) {
            Some(i) => {
                self.corrupt_threads.remove(i);
                true
            }
            None => false,
        }
    }

    /// core-022: deep-copy all messages of `thread_id` under `new_id`.
    fn duplicate_thread(&self, thread_id: Id, new_id: Id) {
        {
            let mut threads = self.threads.lock().unwrap();
            if threads.contains_key(&new_id) || !threads.contains_key(&thread_id) {
                return;
            }
            let mut copy = threads[&thread_id].clone();
            copy.id = new_id.clone();
            copy.title = format!("{} (copy)", copy.title);
            threads.insert(new_id, copy.clone());
            self.persist(&copy);
        }
        self.send_thread_list();
    }
}

// ---------------------------------------------------------------------------
// core-016 partial (ADR-0017 D2): provider error classification
// ---------------------------------------------------------------------------

/// What a provider error string means for retry policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryClass {
    Network,
    RateLimited,
    ServerError,
    Auth,
    Other,
}

/// Classify by substring over lowercase text. Order matters: auth first so
/// nothing shadows a fast-fail class; rate-limit before network so
/// "rate limit exceeded" can't be eaten by a broader transport word later.
/// ponytail: string sniffing is load-bearing until `Provider` returns
/// structured errors (ADR-0017 option e); then only this body changes.
fn classify_provider_error(e: &str) -> RetryClass {
    let l = e.to_lowercase();
    if ["401", "403", "unauthorized", "invalid api key"].iter().any(|s| l.contains(s)) {
        RetryClass::Auth
    } else if l.contains("429") || l.contains("rate limit") {
        RetryClass::RateLimited
    } else if l.contains("5xx") || l.contains("internal") || l.contains("bad gateway") {
        RetryClass::ServerError
    } else if l.contains("timeout")
        || l.contains("timed out")
        || l.contains("connection")
        || l.contains("stream read failed")
    {
        RetryClass::Network
    } else {
        RetryClass::Other
    }
}

/// core-017 (ADR-0017 D3): seconds the provider hinted to wait before
/// retrying — "retry after 5" or "retry-after: 5" in the error text.
/// ponytail: parsed from the error string until providers surface
/// structured errors; then only this body changes.
fn parse_retry_after(e: &str) -> Option<u64> {
    let l = e.to_lowercase();
    for pat in ["retry after", "retry-after"] {
        if let Some(i) = l.find(pat) {
            let rest = &l[i + pat.len()..];
            let digits: String = rest
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}

/// Backoff before a non-network retry: honor the provider's Retry-After
/// hint, capped at 30 s so a hostile hint can't stall a turn; flat 1 s
/// default when no hint is present.
fn retry_backoff_secs(e: &str) -> u64 {
    parse_retry_after(e).map_or(1, |n| n.min(30))
}

/// prov-004/005 (ADR-0011 D2): capabilities of the currently configured
/// model, read from the active provider slot. Nothing configured (or a mock
/// without a model id) resolves to the conservative fallback caps.
fn model_caps(shared: &Shared) -> router::Capabilities {
    match shared.provider.lock().unwrap().as_ref() {
        Some(p) => router::lookup(p.model()),
        None => router::Capabilities::default(),
    }
}

/// One full agent turn: stream → (tool calls → approve → execute)×N → done.
fn run_turn(
    shared: Arc<Shared>,
    event_tx: Sender<Event>,
    _history_lock: Arc<Mutex<()>>,
    data_dir: PathBuf,
    // jour-029: the same journal instance Runtime owns, handed to this worker
    // as an `Option<Arc<..>>` clone (mirrors how `shared`/`data_dir` travel).
    // Choice documented on the Runtime.journal field: it stays OUT of Shared
    // so no other Shared reader has to reason about it; only this worker and
    // the command loop ever append.
    journal: Option<Arc<Mutex<Journal>>>,
    mut thread: Thread,
    turn_id: Id,
) {
    let thread_id = thread.id.clone();
    // sup-008 (ADR-0016): last supervision verdict computed this turn, if any.
    // RefCell because `finish` is called from many exit paths after evaluation
    // may have happened; default stays None for turns that never evaluated.
    let last_verdict =
        std::cell::RefCell::new(None::<z_protocol::SupervisionVerdictInfo>);
    let finish = |ok: bool, error: Option<String>| {
        let _ = event_tx.send(Event::TurnFinished {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            ok,
            error,
            verdict: last_verdict.borrow().clone(),
        });
    };

    // jour-029: baseline = history length at turn entry. Only messages added
    // during THIS turn are counted in MessagePersisted payloads. Every save
    // below goes through `persist`, which journals after writing the snapshot;
    // a failed append is warned and swallowed — it can never fail the turn.
    let turn_start_len = thread.messages.len();
    let persist = |thread: &Thread| {
        save_thread(&data_dir, thread);
        let n_new = thread.messages.len().saturating_sub(turn_start_len);
        if n_new == 0 {
            return;
        }
        let last_role = match thread.messages.last().map(|m| m.role) {
            Some(z_protocol::Role::User) => "user",
            Some(z_protocol::Role::Agent) => "agent",
            None => "unknown",
        };
        Runtime::journal_record(
            &journal,
            JournalKind::MessagePersisted,
            Some(thread.id.clone()),
            json!({ "count": n_new, "last_role": last_role }),
        );
    };

    let Some(provider) = shared.provider.lock().unwrap().clone() else {
        finish(false, Some("no provider configured — set one in Settings".into()));
        return;
    };
    let Some(root) = shared.project_root.lock().unwrap().clone() else {
        finish(false, Some("no project open — open a folder first".into()));
        return;
    };

    // prov-005 (ADR-0011 D2): models whose registry entry lacks tool support
    // run tool-less instead of failing mid-stream. build_request applies the
    // gate every round; say it once here per turn.
    if !model_caps(&shared).supports_tools {
        let name = shared
            .provider
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.model().to_string())
            .unwrap_or_default();
        log::warn!("model {name} lacks tool support; running without tools");
    }

    // core-011/core-012 (ADR-0011): clone the settings Arc ONCE at turn start;
    // a concurrent SetSetting applies to the next turn, never mid-turn.
    let settings = Arc::clone(shared.settings.lock().unwrap().get());
    let max_tool_rounds = settings.max_tool_rounds;
    // core-014 (ADR-0017 D1): per-turn fingerprint counter for the doom-loop
    // breaker. Turn-local lifetime — no Shared field, no reset logic.
    let mut call_counts: HashMap<u64, usize> = HashMap::new();
    let doom_threshold = settings.doom_threshold;
    // sup-007: did this turn execute ANY tool call? A final text claiming
    // success after real execution should have left ok evidence behind.
    let mut turn_had_tool_call = false;
    // core-018 (ADR-0017 D4): when set, the next loop round replays this
    // exact ChatRequest object instead of rebuilding it, so the retry
    // payload is byte-identical to the failed one.
    let mut pending_retry: Option<provider::ChatRequest> = None;

    for round in 0..max_tool_rounds {
        if is_cancelled(&shared, &thread_id) {
            persist(&thread);
            finish(false, Some("cancelled by user".into()));
            return;
        }

        let retrying = pending_retry.is_some();

        // Steering drain (core-006): between tool rounds, queued user text
        // is appended as one combined user message so the next provider
        // round sees it. Round 0 drains only what arrived before the turn's
        // provider call; later rounds pick up mid-turn steering. Skipped on
        // a retry replay so queued steering stays queued (it would not be
        // part of the byte-identical request anyway) rather than dropped.
        if round > 0 && !retrying {
            let drained = Runtime::drain_steering(&shared, &thread_id);
            if !drained.is_empty() {
                if let Some(steering_msg) = Runtime::combine_steering(drained) {
                    log::info!(
                        "steering: injecting {} queued message(s) into thread {thread_id}",
                        steering_msg.text.lines().count().saturating_sub(1)
                    );
                    thread.messages.push(steering_msg);
                    persist(&thread);
                }
            }
        }

        // core-018: a pending retry replays the exact request that failed;
        // only a fresh round builds a new one.
        let request =
            pending_retry.take().unwrap_or_else(|| build_request(provider.as_ref(), &thread, &shared, &root));
        let outcome = {
            let tx = event_tx.clone();
            let result = provider.stream(&request, &mut |item| {
                if let provider::StreamItem::TextDelta(delta) = item {
                    let _ = tx.send(Event::TextDelta {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        delta,
                    });
                }
            });
            match result {
                Ok(o) => o,
                Err(e) => {
                    // core-016 partial (ADR-0017 D2): classified single retry,
                    // round 0 only. Network retries immediately; rate-limit/
                    // server errors back off first — core-017 honors a
                    // provider Retry-After hint (capped at 30 s), else flat
                    // 1 s. Auth/Other fail now — same key, same answer.
                    // Either way the user's message is never lost.
                    let class = classify_provider_error(&e);
                    // core-019 (ADR-0017 D5): every failed attempt leaves one
                    // provider_error breadcrumb. attempt 1 = initial call,
                    // 2 = the replayed retry (the retry executes at round 1).
                    Runtime::journal_record(
                        &journal,
                        JournalKind::ProviderError,
                        Some(thread_id.clone()),
                        json!({
                            "attempt": round + 1,
                            "class": format!("{class:?}").to_lowercase(),
                        }),
                    );
                    let retryable = matches!(
                        class,
                        RetryClass::Network | RetryClass::RateLimited | RetryClass::ServerError
                    );
                    if retryable && round == 0 {
                        if !matches!(class, RetryClass::Network) {
                            std::thread::sleep(std::time::Duration::from_secs(retry_backoff_secs(&e)));
                        }
                        pending_retry = Some(request);
                        continue;
                    }
                    persist(&thread);
                    finish(false, Some(e));
                    return;
                }
            }
        };

        if !outcome.text.trim().is_empty() || outcome.tool_calls.is_empty() {
            thread.messages.push(StoredMessage {
                role: z_protocol::Role::Agent,
                text: outcome.text.clone(),
                tool_calls: Vec::new(),
            });
        }

        if outcome.tool_calls.is_empty() {
            // sup-005/006/007 (ADR-0016): success claims in the final text are
            // linked to same-turn ok evidence. Same-turn window: an earlier
            // turn's green build does not whitewash this claim. sup-007 gates:
            // a fully-unlinked claim set with zero ok same-turn evidence fails
            // the turn when evidence capture was demonstrably operational.
            let mut blocked_reason: Option<String> = None;
            if !outcome.text.trim().is_empty() && !crate::evidence::extract_claims(&outcome.text).is_empty() {
                let claims = crate::evidence::extract_claims(&outcome.text);
                match crate::evidence::EvidenceView::fold(&data_dir.join("journal").join("runtime.jsonl")) {
                    Ok(view) => {
                        let turn_evidence: Vec<_> = view
                            .items
                            .into_iter()
                            .filter(|e| e.turn_id == turn_id)
                            .collect();
                        let report = crate::evidence::link_claims(&claims, &turn_evidence);
                        let verdict = crate::evidence::evaluate_claims(
                            &report,
                            turn_evidence.iter().filter(|e| e.ok).count(),
                        );
                        // sup-009/010/011 (ADR-0016): extra detectors, pure
                        // observability — firing never gates by itself here.
                        let unexecuted_tests =
                            crate::evidence::detect_unexecuted_tests(&claims, &turn_evidence);
                        let unexecuted_build =
                            crate::evidence::detect_unexecuted_build(&claims, &turn_evidence);
                        let ignored_failures =
                            crate::evidence::detect_ignored_failures(&turn_evidence, &outcome.text);
                        let mut fired: Vec<&str> = Vec::new();
                        if unexecuted_tests {
                            fired.push("unexecuted-tests");
                        }
                        if unexecuted_build {
                            fired.push("unexecuted-build");
                        }
                        if ignored_failures {
                            fired.push("ignored-failures");
                        }
                        if !fired.is_empty() {
                            log::warn!(
                                "supervision: detector(s) fired in turn {turn_id}: {}",
                                fired.join(", ")
                            );
                        }
                        // sup-008: an evaluation happened this turn — record
                        // its outcome on TurnFinished regardless of whether
                        // the sup-007 gate promotes it to a turn failure.
                        *last_verdict.borrow_mut() =
                            Some(z_protocol::SupervisionVerdictInfo {
                                blocked: verdict.blocked,
                                reason: verdict.reason.clone(),
                            });
                        // sup-007 gate: fail ONLY when the pipeline was fully
                        // operational — journal handle present, fold succeeded,
                        // and this turn ran at least one tool call (so ok
                        // evidence SHOULD have been captured) — yet every claim
                        // is unlinked with zero ok evidence of any kind. Any
                        // ambiguity (capture path broken, tool-less turn, some
                        // claim linked or some ok evidence) stays warn-only.
                        if verdict.blocked && journal.is_some() && turn_had_tool_call {
                            // sup-009/010: fold the unexecuted-execution
                            // findings into the reason when the sup-007 gate
                            // fires anyway (gating itself unchanged).
                            let mut reason = verdict.reason;
                            let mut extensions: Vec<&str> = Vec::new();
                            if unexecuted_tests {
                                extensions.push("unexecuted-tests");
                            }
                            if unexecuted_build {
                                extensions.push("unexecuted-build");
                            }
                            if !extensions.is_empty() {
                                if let Some(r) = reason.as_mut() {
                                    r.push_str(&format!(" [detectors: {}]", extensions.join(", ")));
                                }
                            }
                            blocked_reason = reason;
                        } else if !report.unlinked.is_empty() {
                            log::warn!(
                                "supervision: {}/{} claim(s) unlinked in turn {turn_id} (kinds: {:?})",
                                report.unlinked.len(),
                                claims.len(),
                                report.unlinked.iter().map(|c| c.kind).collect::<Vec<_>>()
                            );
                        }
                    }
                    Err(err) => log::warn!("supervision: evidence fold failed: {err}"),
                }
            }
            if let Some(reason) = blocked_reason {
                // §8.4: supervision may fail a turn but never edits its output
                // — persist the claimed text verbatim, then fail via the
                // normal TurnFinished(false) path (no TextDone).
                persist(&thread);
                finish(false, Some(reason));
                return;
            }
            let _ = event_tx.send(Event::TextDone { thread_id: thread_id.clone(), turn_id: turn_id.clone() });
            // mem-005 (ADR-0014 D5): best-effort candidate extraction from the
            // final text — provisional-only records, never a turn failure.
            let candidates = crate::memory::extract_candidates(&outcome.text);
            if !candidates.is_empty() {
                if let Some(j) = journal.as_ref() {
                    match crate::memory::promote_candidates(
                        j,
                        &data_dir,
                        &candidates,
                        &thread_id,
                        &turn_id,
                    ) {
                        Ok(0) => {}
                        Ok(n) => log::info!(
                            "memory: {n} extracted candidate(s) recorded in turn {turn_id}"
                        ),
                        Err(e) => log::warn!("memory: candidate recording failed: {e}"),
                    }
                }
            }
            persist(&thread);
            finish(true, None);
            return;
        }

        // Assistant message carrying its tool calls, then results.
        turn_had_tool_call = true;
        let stored_calls: Vec<StoredToolCall> = outcome            .tool_calls
            .iter()
            .map(|c| StoredToolCall {
                id: c.id.clone(),
                name: c.name.clone(),
                arguments_json: c.arguments_json.clone(),
                ok: None,
                summary: String::new(),
            })
            .collect();
        thread.messages.push(StoredMessage {
            role: z_protocol::Role::Agent,
            text: outcome.text,
            tool_calls: stored_calls,
        });

        let mut steer_doom = false;
        for call in &outcome.tool_calls {
            if is_cancelled(&shared, &thread_id) {
                persist(&thread);
                finish(false, Some("cancelled by user".into()));
                return;
            }
            let args: serde_json::Value =
                serde_json::from_str(&call.arguments_json).unwrap_or(serde_json::json!({}));
            let risk = tools::classify(&call.name, &args);
            let detail = tools::describe(&call.name, &args);

            if risk != Risk::ReadOnly {
                let _ = event_tx.send(Event::ApprovalRequested {
                    thread_id: thread_id.clone(),
                    call_id: call.id.clone(),
                    tool: call.name.clone(),
                    detail: detail.clone(),
                    risk,
                });
                match shared.gate.wait(
                    &call.id,
                    std::time::Duration::from_secs(settings.approval_timeout_secs),
                ) {
                    Some(true) => {}
                    Some(false) => {
                        shared.gate.clear(&call.id);
                        record_result(&mut thread, &event_tx, &turn_id, &call.id, false,
                            "denied by user".into());
                        continue;
                    }
                    None => {
                        shared.gate.clear(&call.id);
                        record_result(&mut thread, &event_tx, &turn_id, &call.id, false,
                            "approval timed out".into());
                        continue;
                    }
                }
                shared.gate.clear(&call.id);
            }

            let _ = event_tx.send(Event::StepStarted {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                call_id: call.id.clone(),
                tool: call.name.clone(),
                detail: detail.clone(),
            });

            // edit-016/017: Write-risk calls hold the per-file grant for this
            // thread; a concurrent editor's claim rejects ours and nothing
            // executes. The grant is released once the call completes, on
            // success or failure alike.
            let raw_path = args
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
            let holds_grant = if risk == Risk::Write {
                match grant_write(&shared, &root, &thread_id, &raw_path) {
                    Ok(()) => true,
                    Err(_e) => {
                        record_result(
                            &mut thread,
                            &event_tx,
                            &turn_id,
                            &call.id,
                            false,
                            "blocked by concurrent editor".into(),
                        );
                        continue;
                    }
                }
            } else {
                false
            };

            // core-013/014 (ADR-0017 D1): count identical calls this turn.
            // ponytail: raw arguments_json, not key-order-canonicalized — a
            // real doom loop emits byte-identical JSON in practice.
            let fp = crate::fingerprint::fnv1a64(
                format!("{}{}", call.name, call.arguments_json).as_bytes(),
            );
            let count = {
                let c = call_counts.entry(fp).or_insert(0);
                *c += 1;
                *c
            };
            if count >= 2 * doom_threshold {
                persist(&thread);
                finish(
                    false,
                    Some("Repeated identical tool calls detected — stopping to avoid a loop.".into()),
                );
                return;
            }
            if count == doom_threshold {
                steer_doom = true;
            }

            let output = tools::execute(tools::ToolInvocation {
                name: &call.name,
                args,
                project_root: &root,
                thread_id: &thread_id,
            });
            record_result(
                &mut thread,
                &event_tx,
                &turn_id,
                &call.id,
                output.ok,
                output.text.lines().next().unwrap_or("").chars().take(120).collect(),
            );
            // sup-002/003/004 (ADR-0016): capture-time evidence at the
            // execution chokepoint — terminal_exec lands Build or Tests (by
            // command class), fs_write/edit_patch land Diff. Best-effort like
            // all journal writes; hooks observe results, never gate them.
            let evidence = match call.name.as_str() {
                "terminal_exec" => {
                    let mut e = crate::evidence::Evidence::build(
                        &thread_id,
                        &turn_id,
                        crate::evidence::parse_exit_code(&output.text),
                        output.text.lines().next().unwrap_or("").to_string(),
                    );
                    let args: serde_json::Value =
                        serde_json::from_str(&call.arguments_json).unwrap_or(serde_json::json!({}));
                    e.kind = crate::evidence::classify_command(
                        args.get("command").and_then(serde_json::Value::as_str).unwrap_or(""),
                    );
                    Some(e)
                }
                "fs_write" | "edit_patch" => Some(crate::evidence::Evidence::diff(
                    &thread_id,
                    &turn_id,
                    output.ok,
                    output.text.lines().next().unwrap_or("").to_string(),
                )),
                _ => None,
            };
            if let (Some(evidence), Some(journal)) = (evidence, journal.as_deref()) {
                crate::evidence::record(journal, &evidence);
            }
            // sup-014/015: placeholder/mock detectors over successful
            // write-class calls — observability warnings only, they never
            // gate the tool result.
            if output.ok && matches!(call.name.as_str(), "fs_write" | "edit_patch") {
                if let Ok(args) =
                    serde_json::from_str::<serde_json::Value>(&call.arguments_json)
                {
                    let path = args
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let written = match args.get("content").and_then(serde_json::Value::as_str)
                    {
                        Some(c) => Some(c.to_string()),
                        None => args
                            .get("blocks")
                            .and_then(serde_json::Value::as_array)
                            .map(|blocks| {
                                blocks
                                    .iter()
                                    .filter_map(|b| {
                                        b.get("new").and_then(serde_json::Value::as_str)
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            }),
                    };
                    if let Some(written) = written {
                        if crate::evidence::detect_placeholder_code(&written) {
                            log::warn!("sup-014 placeholder-code marker written to {path}");
                        }
                        if crate::evidence::detect_mock_in_prod(&written) {
                            log::warn!("sup-015 mock-in-prod marker written to {path}");
                        }
                    }
                }
            }
            // Full output goes back to the model as the tool result.
            thread.messages.push(StoredMessage {
                role: z_protocol::Role::User,
                text: String::new(),
                tool_calls: vec![StoredToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments_json: "{}".into(),
                    ok: Some(output.ok),
                    summary: output.text.clone(),
                }],
            });
            if holds_grant {
                release_write(&shared, &root, &thread_id, &raw_path);
            }
        }

        // core-013 (ADR-0017 D1.3): first threshold crossing gets ONE
        // steering note; the turn continues so the model can course-correct.
        if steer_doom {
            log::info!("doom-loop guard: injecting steering note into thread {thread_id}");
            thread.messages.push(StoredMessage {
                role: z_protocol::Role::User,
                text: format!(
                    "[doom-loop guard] You have repeated the identical tool call {doom_threshold} times. Change approach or explain what you are waiting for."
                ),
                tool_calls: Vec::new(),
            });
            persist(&thread);
        }
    }
    persist(&thread);
    finish(false, Some(format!("stopped after {max_tool_rounds} tool rounds")));
}

fn record_result(
    thread: &mut Thread,
    event_tx: &Sender<Event>,
    turn_id: &str,
    call_id: &str,
    ok: bool,
    summary: String,
) {
    if let Some(msg) = thread.messages.iter_mut().rev().find(|m| {
        m.role == z_protocol::Role::Agent && m.tool_calls.iter().any(|c| c.id == call_id)
    }) {
        if let Some(call) = msg.tool_calls.iter_mut().find(|c| c.id == call_id) {
            call.ok = Some(ok);
            call.summary = summary.clone();
        }
    }
    let _ = event_tx.send(Event::StepFinished {
        thread_id: thread.id.clone(),
        turn_id: turn_id.to_string(),
        call_id: call_id.to_string(),
        ok,
        summary,
    });
}

fn is_cancelled(shared: &Shared, thread_id: &str) -> bool {
    shared.cancelled.lock().unwrap().contains(thread_id)
}

/// edit-016/017 (ADR-0010 §(5c)): acquire the exclusive write grant for one
/// canonical path on behalf of `thread_id`. Another thread's live grant is
/// rejected at acquire time; the owner re-enters freely.
fn grant_write(
    shared: &Shared,
    root: &Path,
    thread_id: &str,
    raw_path: &str,
) -> Result<(), String> {
    let key = tools::canonical_key(root, raw_path)?.to_string_lossy().into_owned();
    let mut grants = shared.write_grants.lock().unwrap();
    match grants.get(&key) {
        Some(owner) if owner != thread_id => {
            Err(format!("File is being edited by another task. (held by {owner})"))
        }
        Some(_) => Ok(()), // reentrant for the owner
        None => {
            grants.insert(key, thread_id.to_string());
            Ok(())
        }
    }
}

/// Release the write grant only when the caller owns it; stale releases
/// (never granted, or ownership changed) are deliberate no-ops.
fn release_write(shared: &Shared, root: &Path, thread_id: &str, raw_path: &str) {
    if let Ok(key) = tools::canonical_key(root, raw_path) {
        let key = key.to_string_lossy().into_owned();
        let mut grants = shared.write_grants.lock().unwrap();
        if grants.get(&key).map(String::as_str) == Some(thread_id) {
            grants.remove(&key);
        }
    }
}

/// Test-visible view of the grant registry (edit-016).
#[cfg(test)]
fn debug_grants(shared: &Shared) -> Vec<(String, String)> {
    let mut grants: Vec<(String, String)> = shared
        .write_grants
        .lock()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    grants.sort();
    grants
}

fn save_thread(data_dir: &std::path::Path, thread: &Thread) {
    let path = data_dir.join("threads").join(format!("{}.json", thread.id));
    if let Ok(json) = serde_json::to_string_pretty(thread) {
        let _ = std::fs::write(path, json);
    }
}

/// Shape summary of a ProviderConfig for the journal (jour-024): the NAMES of
/// the fields the client sent, never their values — in particular the API key
/// value must not reach the journal under any circumstances.
fn config_field_names(config: &ProviderConfig) -> Vec<String> {
    match serde_json::to_value(config) {
        Ok(serde_json::Value::Object(map)) => map.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// Context budget defaults. Personal v1 assumes a 128k-class window; the
/// completion plus safety margin are reserved before history is counted.
const CONTEXT_HARD_LIMIT: usize = 128_000;
const COMPLETION_RESERVE: usize = 12_000;

/// Token cost of one stored conversation message, including its tool calls.
fn stored_message_tokens(m: &StoredMessage) -> usize {
    crate::tokens::estimate(&m.text)
        + m.tool_calls
            .iter()
            .map(|c| {
                crate::tokens::estimate(&c.name)
                    + crate::tokens::estimate(&c.arguments_json)
                    + crate::tokens::estimate(&c.summary)
                    + 8
            })
            .sum::<usize>()
}

/// Trim history to `budget` tokens by dropping whole turns from the FRONT.
///
/// Safety rules:
/// - A cut may only happen at a "clean boundary": the start of a real user
///   message (no tool results). This guarantees every kept assistant
///   tool_call still has its result carrier, so the request stays valid.
/// - The newest fitting boundary wins (we keep as much history as possible);
///   if nothing fits we fall back to the most aggressive clean cut.
fn trim_history(msgs: &[StoredMessage], budget: usize) -> Vec<StoredMessage> {
    if msgs.is_empty() {
        return Vec::new();
    }
    let costs: Vec<usize> = msgs.iter().map(stored_message_tokens).collect();
    let total: usize = costs.iter().sum();
    if total <= budget {
        return msgs.to_vec();
    }

    // Walk boundaries oldest→newest; the FIRST clean boundary whose tail
    // fits keeps the maximum history. If none fits, fall back to the LAST
    // clean boundary (most aggressive trim that stays structurally valid).
    let mut fallback: Option<usize> = None;
    let mut suffix = total; // sum(costs[i..])
    for i in 0..msgs.len() {
        let clean = msgs[i].role == z_protocol::Role::User && msgs[i].tool_calls.is_empty();
        if clean {
            if suffix <= budget {
                return msgs[i..].to_vec();
            }
            fallback = Some(i);
        }
        suffix -= costs[i];
    }
    match fallback {
        Some(i) => msgs[i..].to_vec(),
        // No clean boundary exists (conversation started mid-tool-loop);
        // sending untrimmed is safer than producing an invalid request.
        None => msgs.to_vec(),
    }
}

/// ctx-003 (ADR-0013): second compaction gate, after trim_history exhausted
/// whole-turn cuts and we are STILL over budget. Maps history onto context
/// layers and lets the pure allocator (`context::assemble`) drop by priority:
/// tool-result bodies first, then oldest current-turn output, then oldest
/// session history — never the live user message. The system prompt is not a
/// StoredMessage (it is build_request's Prefix), so no Prefix item exists here.
///
/// After assembly the call↔result pairing is repaired atomically: an agent
/// tool_call and its result carrier are dropped together if allocation ever
/// separated them, so the request stays provider-valid.
fn enforce_budget(msgs: Vec<StoredMessage>, budget: usize) -> Vec<StoredMessage> {
    let total: usize = msgs.iter().map(stored_message_tokens).sum();
    if msgs.is_empty() || total <= budget {
        return msgs; // under budget: byte-for-byte unchanged (stability guard)
    }
    let last_user = msgs
        .iter()
        .rposition(|m| m.role == z_protocol::Role::User && m.tool_calls.is_empty());
    let items = msgs
        .iter()
        .enumerate()
        .map(|(i, m)| crate::context::ContextItem {
            layer: if !m.tool_calls.is_empty() {
                // Tool plumbing — an agent's tool_call message AND its result
                // carrier — leaves as one Ephemeral unit before any history.
                crate::context::Layer::Ephemeral
            } else if last_user.map_or(false, |lu| i > lu) {
                crate::context::Layer::Turn // current-turn agent output
            } else {
                // Older history AND the final user message — which is the
                // LAST Session item, i.e. assemble's pinned survivor.
                crate::context::Layer::Session
            },
            // ponytail: ContextItem has no index field; the original index as
            // text maps survivors back unambiguously (est_tokens drives the
            // allocator, text is only carried through).
            text: i.to_string(),
            est_tokens: stored_message_tokens(m),
            stale: false,
        })
        .collect::<Vec<_>>();
    let mut keep: Vec<usize> = crate::context::assemble(items, budget)
        .into_iter()
        .filter_map(|it| it.text.parse::<usize>().ok())
        .collect();
    keep.sort_unstable();

    // Repair pass (until stable): drop a result carrier whose agent call did
    // not survive directly before it, or an agent call whose carrier did not
    // survive directly after it. Never touches plain texts or the final user
    // message (empty tool_calls ⇒ never orphaned).
    loop {
        let mut changed = false;
        let mut next_keep = Vec::with_capacity(keep.len());
        for (pos, &i) in keep.iter().enumerate() {
            let prev = pos.checked_sub(1).map(|p| keep[p]);
            let next = keep.get(pos + 1).copied();
            let m = &msgs[i];
            let orphan = if m.role == z_protocol::Role::Agent && !m.tool_calls.is_empty() {
                !next.map_or(false, |n| {
                    msgs[n].role == z_protocol::Role::User
                        && m.tool_calls
                            .iter()
                            .all(|k| msgs[n].tool_calls.iter().any(|r| r.id == k.id))
                })
            } else if m.role == z_protocol::Role::User && !m.tool_calls.is_empty() {
                !prev.map_or(false, |p| {
                    msgs[p].role == z_protocol::Role::Agent
                        && m.tool_calls
                            .iter()
                            .all(|r| msgs[p].tool_calls.iter().any(|k| k.id == r.id))
                })
            } else {
                false
            };
            if orphan {
                changed = true;
            } else {
                next_keep.push(i);
            }
        }
        keep = next_keep;
        if !changed {
            break;
        }
    }
    keep.into_iter().map(|i| msgs[i].clone()).collect()
}

/// Assemble the chat request. The system prompt + repo map form a stable
/// prefix (provider prompt-cache friendly); only the tail changes per turn.
/// History is budget-checked locally before send and trimmed at clean turn
/// boundaries when it would crowd out the completion.
fn build_request(
    provider: &dyn provider::Provider,
    thread: &Thread,
    shared: &Shared,
    root: &std::path::Path,
) -> provider::ChatRequest {
    let label = shared.provider_label.lock().unwrap().clone();
    let repo_map = shared
        .index
        .lock()
        .unwrap()
        .as_ref()
        .map(|i| i.map_text(160))
        .unwrap_or_default();

    let system = format!(
        "You are Z, the agent of Z Desktop Personal — a local-first developer \
         workspace. You are precise, honest and economical: do not act just to \
         look active. Use tools when they are needed to answer correctly; read \
         real files before editing them. Project root: {}

Repository map:
{repo_map}

Active model: {label}",
        root.display()
    );

    // prov-004/005 (ADR-0011 D2): capability gate — a model registered
    // without tool support never sees tool definitions; attaching them would
    // only teach it to hallucinate tool-call syntax mid-stream.
    let tool_defs = if model_caps(shared).supports_tools {
        tools::definitions()
    } else {
        Vec::new()
    };

    // Budget: fixed parts first, then whatever room is left for history.
    let tools_tokens: usize = tool_defs
        .iter()
        .map(|t| {
            crate::tokens::estimate_tool_def(
                &t.name,
                &t.description,
                &t.parameters.to_string(),
            )
        })
        .sum();
    let fixed = crate::tokens::estimate(&system) + tools_tokens + 16;
    let soft_target = CONTEXT_HARD_LIMIT.saturating_sub(COMPLETION_RESERVE);
    let history_budget = soft_target.saturating_sub(fixed);
    let history = trim_history(&thread.messages, history_budget);
    // ctx-003 (ADR-0013): second gate — when whole-turn trimming alone still
    // leaves us over budget, compact via the context allocator.
    let history = enforce_budget(history, history_budget);
    if history.len() != thread.messages.len() {
        log::info!(
            "context budget: trimmed {} -> {} history messages (~{} tokens fixed)",
            thread.messages.len(),
            history.len(),
            fixed
        );
    }

    let mut messages = vec![provider::ChatMessage::System(system)];
    for m in &history {
        match m.role {
            z_protocol::Role::User => {
                if m.tool_calls.is_empty() {
                    messages.push(provider::ChatMessage::User(m.text.clone()));
                } else {
                    // Tool-result carrier message.
                    for c in &m.tool_calls {
                        messages.push(provider::ChatMessage::ToolResult {
                            call_id: c.id.clone(),
                            output: c.summary.clone(),
                        });
                    }
                }
            }
            z_protocol::Role::Agent => {
                messages.push(provider::ChatMessage::Assistant {
                    text: m.text.clone(),
                    tool_calls: m
                        .tool_calls
                        .iter()
                        .map(|c| provider::ToolCallSpec {
                            id: c.id.clone(),
                            name: c.name.clone(),
                            arguments_json: c.arguments_json.clone(),
                        })
                        .collect(),
                });
            }
        }
    }

    provider::ChatRequest {
        messages,
        tools: tool_defs,
        max_tokens: 8192,
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn user(text: &str) -> StoredMessage {
        StoredMessage { role: z_protocol::Role::User, text: text.into(), tool_calls: Vec::new() }
    }

    fn agent_with_call(id: &str) -> StoredMessage {
        StoredMessage {
            role: z_protocol::Role::Agent,
            text: String::new(),
            tool_calls: vec![StoredToolCall {
                id: id.into(),
                name: "fs_read".into(),
                arguments_json: "{}".into(),
                ok: Some(true),
                summary: "x".repeat(200),
            }],
        }
    }

    fn results(ids: &[&str]) -> StoredMessage {
        StoredMessage {
            role: z_protocol::Role::User,
            text: String::new(),
            tool_calls: ids
                .iter()
                .map(|id| StoredToolCall {
                    id: id.to_string(),
                    name: "fs_read".into(),
                    arguments_json: "{}".into(),
                    ok: Some(true),
                    summary: "y".repeat(200),
                })
                .collect(),
        }
    }

    #[test]
    fn history_within_budget_is_untouched() {
        let msgs = vec![user("hello"), user("world")];
        let out = trim_history(&msgs, 10_000);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn trimming_never_orphans_a_tool_result_carrier() {
        // [user, agent(call a), results(a), user, agent(call b), results(b)]
        let msgs = vec![
            user("first question with plenty of text to push the budget"),
            agent_with_call("a"),
            results(&["a"]),
            user("second question"),
            agent_with_call("b"),
            results(&["b"]),
        ];
        let out = trim_history(&msgs, 1); // force aggressive trim
        // The cut must land on a real user message, never on results(a).
        assert!(!out.is_empty());
        let first = &out[0];
        assert_eq!(first.role, z_protocol::Role::User);
        assert!(first.tool_calls.is_empty(), "cut orphaned tool results");
        // Every kept agent call still has its result carrier.
        for (i, m) in out.iter().enumerate() {
            if m.role == z_protocol::Role::Agent && !m.tool_calls.is_empty() {
                assert!(
                    out[i + 1..].iter().any(|n| n
                        .tool_calls
                        .iter()
                        .any(|c| m.tool_calls.iter().any(|k| k.id == c.id))),
                    "agent call {} lost its result",
                    m.tool_calls[0].id
                );
            }
        }
    }

    #[test]
    fn newest_fitting_boundary_wins_over_aggressive_cut() {
        let big = "word ".repeat(400); // ~100 tokens each
        let msgs = vec![user(&big), user(&big), user("tail")];
        // Budget fits only the last two messages.
        let out = trim_history(&msgs, stored_message_tokens(&msgs[1]) + stored_message_tokens(&msgs[2]));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, big);
    }

    #[test]
    fn empty_history_is_safe() {
        assert!(trim_history(&[], 100).is_empty());
    }

    /// ctx-001/002 core slice (ADR-0013): build_request is not rewired onto
    /// the assembler yet — trim_history stays primary — but this gate proves
    /// its output already fits the budget under assemble() semantics, i.e.
    /// the second gate would be a no-op today. Wiring lands next slice.
    #[test]
    fn built_request_fits_budget_under_assemble_semantics() {
        struct NullProvider;
        impl provider::Provider for NullProvider {
            fn describe(&self) -> String {
                "null".into()
            }
            fn stream(
                &self,
                _req: &provider::ChatRequest,
                _on_item: &mut dyn FnMut(provider::StreamItem),
            ) -> Result<provider::StreamOutcome, String> {
                Ok(provider::StreamOutcome::default())
            }
        }
        let shared = Shared {
            provider: Mutex::new(None),
            provider_label: Mutex::new(String::new()),
            project_root: Mutex::new(None),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        let big = "word ".repeat(400); // ~100 tokens per message
        let thread = Thread {
            id: "t".into(),
            title: "gate".into(),
            messages: (0..40).map(|_| user(&big)).collect(),
            updated_ms: 0,
        };
        let req = build_request(&NullProvider, &thread, &shared, std::path::Path::new("/"));

        // Layer the rendered request per ADR-0013: system = Prefix, all
        // history = Session (Turn/Ephemeral don't exist as distinct sites yet).
        let items: Vec<crate::context::ContextItem> = req
            .messages
            .iter()
            .map(|m| {
                let (layer, text) = match m {
                    provider::ChatMessage::System(s) => (crate::context::Layer::Prefix, s.clone()),
                    provider::ChatMessage::User(t) => (crate::context::Layer::Session, t.clone()),
                    provider::ChatMessage::Assistant { text, .. } => {
                        (crate::context::Layer::Session, text.clone())
                    }
                    provider::ChatMessage::ToolResult { output, .. } => {
                        (crate::context::Layer::Session, output.clone())
                    }
                };
                crate::context::ContextItem {
                    layer,
                    est_tokens: crate::tokens::estimate(&text) + 4,
                    text,
                    stale: false,
                }
            })
            .collect();
        let soft_target = CONTEXT_HARD_LIMIT - COMPLETION_RESERVE;
        let kept = crate::context::assemble(items.clone(), soft_target);
        assert_eq!(
            kept.len(),
            items.len(),
            "assemble dropped something from a freshly built request"
        );
    }

    // -----------------------------------------------------------------
    // ctx-003: enforce_budget (second compaction gate)
    // -----------------------------------------------------------------

    #[test]
    fn enforce_budget_tiny_budget_keeps_the_live_user_message() {
        // The system prompt is build_request's Prefix, not a StoredMessage
        // (z_protocol::Role has no System), so the never-dropped survivor here
        // is assemble's pinned item: the final real user message.
        let big = "word ".repeat(400);
        let msgs = vec![user(&big), user(&big), user(&big), user("live question")];
        let out = enforce_budget(msgs, 10);
        assert_eq!(out.last().unwrap().text, "live question");
        assert_eq!(out.last().unwrap().role, z_protocol::Role::User);
    }

    #[test]
    fn enforce_budget_drops_tool_bodies_before_session_history() {
        // One full tool round whose bodies dwarf everything else.
        let mut call = agent_with_call("a");
        call.tool_calls[0].summary = "z".repeat(4000);
        let mut carrier = results(&["a"]);
        carrier.tool_calls[0].summary = "y".repeat(4000);
        let msgs = vec![
            user("first question"),
            call,
            carrier,
            user("second question"),
        ];
        // Budget fits only the two session texts (+margin): both Ephemeral
        // bodies must go, and their agent-call partner must go with them.
        let budget = stored_message_tokens(&user("first question"))
            + stored_message_tokens(&user("second question"))
            + 4;
        let out = enforce_budget(msgs, budget);
        assert!(
            out.iter().all(|m| m.tool_calls.is_empty()),
            "call/result pair must be dropped atomically"
        );
        assert!(out.iter().any(|m| m.text == "first question"));
        assert!(out.iter().any(|m| m.text == "second question"));
    }

    #[test]
    fn enforce_budget_never_orphans_calls_or_results() {
        let big = "word ".repeat(400); // heavy tail forces deep compaction
        let msgs = vec![
            user("first question"),
            agent_with_call("a"),
            results(&["a"]),
            user("second question"),
            agent_with_call("b"),
            results(&["b"]),
            user(&big),
        ];
        let out = enforce_budget(msgs, 40);
        // The live user message always survives.
        assert_eq!(out.last().unwrap().role, z_protocol::Role::User);
        assert!(out.last().unwrap().tool_calls.is_empty());
        // Whatever survived is provider-valid: every kept agent call still
        // has its result carrier directly after it, and vice versa.
        for (i, m) in out.iter().enumerate() {
            if m.role == z_protocol::Role::Agent && !m.tool_calls.is_empty() {
                assert!(
                    out.get(i + 1).map_or(false, |n| n.role == z_protocol::Role::User
                        && m.tool_calls.iter().all(|k| n.tool_calls.iter().any(|r| r.id == k.id))),
                    "agent call {} lost its result",
                    m.tool_calls[0].id
                );
            }
            if m.role == z_protocol::Role::User && !m.tool_calls.is_empty() {
                assert!(
                    i > 0
                        && out[i - 1].role == z_protocol::Role::Agent
                        && m.tool_calls
                            .iter()
                            .all(|r| out[i - 1].tool_calls.iter().any(|k| k.id == r.id)),
                    "result carrier orphaned from its agent call"
                );
            }
        }
    }

    #[test]
    fn enforce_budget_under_budget_is_byte_identical() {
        let msgs = vec![user("hello"), user("world")];
        let snapshot = msgs.clone();
        let out = enforce_budget(msgs, 10_000);
        assert_eq!(out.len(), snapshot.len());
        assert!(out.iter().zip(snapshot.iter()).all(|(a, b)| a.text == b.text));
    }
}

#[cfg(test)]
mod capability_gate_tests {
    use super::*;

    fn shared_with(model: &str) -> Shared {
        let mut shared = Shared {
            provider: Mutex::new(None),
            provider_label: Mutex::new(String::new()),
            project_root: Mutex::new(None),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        shared.provider =
            Mutex::new(Some(Arc::new(provider::OpenAiProvider {
                config: ProviderConfig {
                    name: "t".into(),
                    kind: z_protocol::ProviderKind::OpenAi,
                    base_url: "http://localhost".into(),
                    model: model.into(),
                    api_key: String::new(),
                },
            }) as Arc<dyn provider::Provider>));
        shared
    }

    #[test]
    fn model_caps_reads_the_configured_model_from_the_slot() {
        let caps = model_caps(&shared_with("GPT-4o"));
        assert_eq!(caps.context_window, 128_000);
        assert!(caps.supports_tools);

        let caps = model_caps(&shared_with("claude-sonnet-4"));
        assert_eq!(caps.context_window, 200_000);
        assert!(caps.supports_tools);
    }

    #[test]
    fn unregistered_or_missing_models_fall_back_conservatively() {
        // Unknown model id.
        assert!(!model_caps(&shared_with("offline-7b")).supports_tools);
        // No provider configured at all.
        let empty = Shared {
            provider: Mutex::new(None),
            provider_label: Mutex::new(String::new()),
            project_root: Mutex::new(None),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        let caps = model_caps(&empty);
        assert!(!caps.supports_tools);
        assert_eq!(caps, router::Capabilities::default());
    }

    /// The skip branch end-to-end: build_request must not attach tool
    /// definitions for models whose registry entry lacks tool support.
    struct NullProvider;
    impl provider::Provider for NullProvider {
        fn describe(&self) -> String {
            "null".into()
        }
        fn stream(
            &self,
            _req: &provider::ChatRequest,
            _on_item: &mut dyn FnMut(provider::StreamItem),
        ) -> Result<provider::StreamOutcome, String> {
            Ok(provider::StreamOutcome::default())
        }
    }

    #[test]
    fn build_request_skips_tools_for_unregistered_models() {
        let thread =
            Thread { id: "t".into(), title: "x".into(), messages: Vec::new(), updated_ms: 0 };
        let root = std::path::Path::new("/");

        let req = build_request(&NullProvider, &thread, &shared_with("offline-7b"), root);
        assert!(req.tools.is_empty(), "tool-less model must not see tool defs");

        let req = build_request(&NullProvider, &thread, &shared_with("gpt-4o"), root);
        assert!(!req.tools.is_empty(), "tool-capable model keeps its tools");
    }
}

#[cfg(test)]
mod steering_tests {
    use super::*;

    /// A scripted provider that returns queued outcomes per call, then a
    /// final text-only outcome. Records every request it receives so tests
    /// can assert exactly what the model saw. (`pub(super)` so the sibling
    /// journal_wiring_tests module can reuse it.)
    pub(super) struct ScriptedProvider {
        pub(super) outcomes: std::sync::Mutex<Vec<provider::StreamOutcome>>,
        pub(super) requests: std::sync::Mutex<Vec<String>>, // user-message texts per request
    }

    impl ScriptedProvider {
        fn tool_call_then_text() -> Vec<provider::StreamOutcome> {
            vec![
                provider::StreamOutcome {
                    text: String::new(),
                    tool_calls: vec![provider::ToolCallSpec {
                        id: "call-1".into(),
                        name: "fs_read".into(),
                        arguments_json: r#"{"path":"README.md"}"#.into(),
                    }],
                },
                provider::StreamOutcome { text: "done".into(), tool_calls: Vec::new() },
            ]
        }

        fn describe_requests(reqs: &[String]) -> String {
            reqs.join("\n---\n")
        }
    }

    impl provider::Provider for ScriptedProvider {
        fn describe(&self) -> String {
            "scripted".into()
        }
        fn stream(
            &self,
            _req: &provider::ChatRequest,
            on_item: &mut dyn FnMut(provider::StreamItem),
        ) -> Result<provider::StreamOutcome, String> {
            let mut outcomes = self.outcomes.lock().unwrap();
            let outcome = if outcomes.is_empty() {
                provider::StreamOutcome::default()
            } else {
                outcomes.remove(0)
            };
            drop(outcomes);
            self.requests.lock().unwrap().push("request".to_string());
            if !outcome.text.is_empty() {
                on_item(provider::StreamItem::TextDelta(outcome.text.clone()));
            }
            Ok(outcome)
        }
    }

    #[test]
    fn combine_steering_merges_texts_under_one_marker() {
        // core-007: consecutive texts merge into ONE user message.
        let combined = Runtime::combine_steering(vec![
            "stop using tabs".to_string(),
            "and add tests".to_string(),
        ])
        .expect("non-empty input combines");
        assert_eq!(combined.role, z_protocol::Role::User);
        assert!(combined.tool_calls.is_empty());
        assert!(combined.text.starts_with("User steering:\n"));
        assert!(combined.text.contains("stop using tabs"));
        assert!(combined.text.contains("and add tests"));
        // One marker, not one per message.
        assert_eq!(combined.text.matches("User steering:").count(), 1);
    }

    #[test]
    fn combine_steering_of_only_whitespace_is_none() {
        assert!(Runtime::combine_steering(vec![]).is_none());
        assert!(Runtime::combine_steering(vec!["   ".to_string()]).is_none());
    }

    #[test]
    fn drain_steering_empties_the_queue_and_preserves_order() {
        let shared = Shared {
            provider: Mutex::new(None),
            provider_label: Mutex::new(String::new()),
            project_root: Mutex::new(None),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        {
            let mut q = shared.steering.lock().unwrap();
            let queue = q.entry("t".into()).or_default();
            queue.push_back("first".into());
            queue.push_back("second".into());
        }
        let drained = Runtime::drain_steering(&shared, "t");
        assert_eq!(drained, vec!["first".to_string(), "second".to_string()]);
        // Second drain sees an empty queue.
        assert!(Runtime::drain_steering(&shared, "t").is_empty());
        // Unknown thread is safe.
        assert!(Runtime::drain_steering(&shared, "nope").is_empty());
    }

    #[test]
    fn enqueue_respects_cap_and_keeps_newest() {
        let shared = Shared {
            provider: Mutex::new(None),
            provider_label: Mutex::new(String::new()),
            project_root: Mutex::new(None),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        let rt = make_runtime_for_tests(Arc::new(shared));
        for i in 0..(STEERING_QUEUE_CAP + 4) {
            rt.enqueue_message("t".into(), format!("m{i}"));
        }
        let drained = Runtime::drain_steering(&rt.shared, "t");
        assert_eq!(drained.len(), STEERING_QUEUE_CAP);
        assert_eq!(drained[0], format!("m{}", 4)); // oldest four dropped
        assert_eq!(drained.last().unwrap(), &format!("m{}", STEERING_QUEUE_CAP + 3));
    }

    #[test]
    fn cancel_clears_pending_steering() {
        // core-008: CancelTurn must not leak stale steering into later turns.
        let shared = Shared {
            provider: Mutex::new(None),
            provider_label: Mutex::new(String::new()),
            project_root: Mutex::new(None),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        let rt = make_runtime_for_tests(Arc::new(shared));
        rt.enqueue_message("t".into(), "stale guidance".into());
        // Simulate what serve() does on CancelTurn.
        rt.shared.cancelled.lock().unwrap().insert("t".into());
        {
            let mut steering = rt.shared.steering.lock().unwrap();
            if let Some(queue) = steering.get_mut("t") {
                queue.clear();
            }
        }
        assert!(Runtime::drain_steering(&rt.shared, "t").is_empty());
    }

    #[test]
    fn mid_turn_steering_lands_before_next_provider_round() {
        // The vertical-slice proof (Exact Next Tasks #1): a tool-calling
        // first round, steering enqueued while round 1 executes, and the
        // second provider request must contain the combined steering text.
        let shared = Shared {
            provider: Mutex::new(Some(Arc::new(ScriptedProvider {
                outcomes: std::sync::Mutex::new(ScriptedProvider::tool_call_then_text()),
                requests: std::sync::Mutex::new(Vec::new()),
            }))),
            provider_label: Mutex::new("scripted".into()),
            project_root: Mutex::new(Some(std::env::current_dir().unwrap())),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        let rt = make_runtime_for_tests(Arc::new(shared));

        // Enqueue BEFORE the turn starts; round 1's drain (round > 0) picks
        // it up before building the second request.
        rt.enqueue_message("steer-thread".into(), "use forward slashes only".into());

        let thread = Thread {
            id: "steer-thread".into(),
            title: "steer test".into(),
            messages: vec![StoredMessage {
                role: z_protocol::Role::User,
                text: "read the readme".into(),
                tool_calls: Vec::new(),
            }],
            updated_ms: 0,
        };

        let (event_tx, event_rx) = std::sync::mpsc::channel();
        run_turn(
            Arc::clone(&rt.shared),
            event_tx,
            Arc::new(Mutex::new(())),
            rt.data_dir.clone(),
            rt.journal.clone(),
            thread,
            "turn-steer".into(),
        );

        // Drain events until TurnFinished to prove lifecycle integrity.
        let mut finished = false;
        while let Ok(event) = event_rx.try_recv() {
            if let Event::TurnFinished { ok, .. } = event {
                finished = ok;
            }
        }
        assert!(finished, "turn did not finish cleanly");

        // The fs_read tool ran inside the sandboxed tool runtime with the
        // Verify the injected user turn landed in the persisted history.
        // Note: serde_json escapes newlines, so match the unescaped form.
        let saved = std::fs::read_to_string(
            rt.data_dir.join("threads").join("steer-thread.json"),
        )
        .expect("thread persisted");
        let unescaped = saved.replace("\\n", "\n");
        assert!(
            unescaped.contains("User steering:\nuse forward slashes only"),
            "steering text missing from persisted history"
        );
    }

    /// Build a Runtime at an explicit `data_dir` WITHOUT serving its command
    /// loop, returning the command sender and event receiver so tests can
    /// drive `serve()` themselves or poke `Shared` directly. The runtime
    /// journal is opened through the same production path (`open_runtime_journal`)
    /// so tests exercise the real resume-on-restart logic.
    pub(super) fn make_runtime_at(
        shared: Arc<Shared>,
        data_dir: PathBuf,
    ) -> (
        Runtime,
        Sender<(u64, Command)>,
        std::sync::mpsc::Receiver<Event>,
    ) {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let _ = std::fs::create_dir_all(data_dir.join("threads"));
        let journal = open_runtime_journal(&data_dir);
        (
            Runtime {
                shared,
                data_dir,
                threads: Mutex::new(HashMap::new()),
                event_tx,
                cmd_rx,
                journal,
                most_recent_restored: None,
                corrupt_threads: Vec::new(),
            },
            cmd_tx,
            event_rx,
        )
    }

    /// Build a Runtime whose command loop is never served, purely so unit
    /// tests can drive `enqueue_message` / inspect `Shared` directly.
    fn make_runtime_for_tests(shared: Arc<Shared>) -> Runtime {
        let data_dir = std::env::temp_dir().join(format!("zdt-steer-{:x}", std::process::id()));
        let (rt, _cmd_tx, event_rx) = make_runtime_at(shared, data_dir);
        // Keep the receiver alive for the duration of the test.
        std::mem::forget(event_rx);
        rt
    }
}

#[cfg(test)]
mod journal_wiring_tests {
    use super::*;
    use super::steering_tests::{make_runtime_at, ScriptedProvider};

    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn unique_data_dir(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "zdt-jour-{}-{tag}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir); // start clean even after a crashed run
        std::fs::create_dir_all(dir.join("threads")).expect("create temp data dir");
        dir
    }

    fn scripted_shared(reply_text: &str) -> Shared {
        Shared {
            provider: Mutex::new(Some(Arc::new(ScriptedProvider {
                outcomes: std::sync::Mutex::new(vec![provider::StreamOutcome {
                    text: reply_text.into(),
                    tool_calls: Vec::new(),
                }]),
                requests: std::sync::Mutex::new(Vec::new()),
            }))),
            provider_label: Mutex::new("scripted".into()),
            project_root: Mutex::new(Some(std::env::current_dir().unwrap())),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        }
    }

    fn empty_shared() -> Shared {
        Shared {
            provider: Mutex::new(None),
            provider_label: Mutex::new(String::new()),
            project_root: Mutex::new(None),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        }
    }

    /// Drive one full SendMessage turn through a real serve() loop on a
    /// background thread; blocks until the turn finishes, then shuts the loop
    /// down. Returns the turn_id from the TurnStarted event.
    fn serve_one_turn(data_dir: &std::path::Path, thread_id: &str, text: &str) -> Id {
        let (rt, cmd_tx, event_rx) =
            make_runtime_at(Arc::new(scripted_shared("journaled answer")), data_dir.to_path_buf());
        let server = std::thread::spawn(move || rt.serve());
        cmd_tx
            .send((
                1,
                Command::SendMessage { thread_id: thread_id.into(), text: text.into() },
            ))
            .expect("send SendMessage");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut turn_id = None;
        loop {
            assert!(std::time::Instant::now() < deadline, "turn did not finish in time");
            match event_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(Event::TurnStarted { thread_id: t, turn_id: tid }) if t == thread_id => {
                    turn_id = Some(tid);
                }
                Ok(Event::TurnFinished { ok: true, thread_id: t, .. }) if t == thread_id => break,
                Ok(_) => {}
                Err(_) => {}
            }
        }
        drop(cmd_tx);
        server.join().expect("serve loop must not panic");
        turn_id.expect("saw TurnStarted event")
    }

    #[test]
    fn serve_pipeline_journals_command_received_turn_started_and_message_persisted() {
        let data_dir = unique_data_dir("pipeline");
        let (rt, cmd_tx, event_rx) =
            make_runtime_at(Arc::new(scripted_shared("journaled answer")), data_dir.clone());
        let server = std::thread::spawn(move || rt.serve());

        cmd_tx.send((1, Command::SendMessage { thread_id: "jour-thread".into(), text: "hello".into() })).expect("send");
        cmd_tx.send((2, Command::CancelTurn { thread_id: "other-thread".into() })).expect("send");
        cmd_tx.send((3, Command::ResolveApproval { call_id: "call-x".into(), approved: true })).expect("send");

        // Wait for the turn itself; later commands are processed by the same
        // serial serve loop before/while it runs.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut started_turn_id = None;
        loop {
            assert!(std::time::Instant::now() < deadline, "turn did not finish in time");
            match event_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(Event::TurnStarted { thread_id: t, turn_id }) if t == "jour-thread" => {
                    started_turn_id = Some(turn_id);
                }
                Ok(Event::TurnFinished { ok: true, thread_id: t, .. }) if t == "jour-thread" => break,
                Ok(_) => {}
                Err(_) => {}
            }
        }
        drop(cmd_tx);
        server.join().expect("serve loop must not panic");

        let path = data_dir.join("journal").join("runtime.jsonl");
        let records = Journal::replay(&path).expect("journal replayable");

        // Sequence is contiguous starting at 1 — no gaps, no reuse.
        let seqs: Vec<u64> = records.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, (1..=records.len() as u64).collect::<Vec<_>>());

        // One CommandReceived per command sent, in dispatch order.
        let received: Vec<&Record> = records.iter()
            .filter(|r| r.kind == JournalKind::CommandReceived)
            .collect();
        assert_eq!(received.len(), 3, "one CommandReceived per command");
        assert_eq!(received[0].payload["command"], "send_message");
        assert_eq!(received[0].payload["thread_id"], "jour-thread");
        assert_eq!(received[1].payload["command"], "cancel_turn");
        assert_eq!(received[1].payload["thread_id"], "other-thread");
        assert_eq!(received[2].payload["command"], "resolve_approval");

        // Exactly one TurnStarted, matching the emitted event's turn_id.
        let starts: Vec<&Record> = records.iter()
            .filter(|r| r.kind == JournalKind::TurnStarted)
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].thread_id.as_deref(), Some("jour-thread"));
        let turn_id = started_turn_id.expect("saw TurnStarted event");
        assert_eq!(starts[0].payload["turn_id"], turn_id);

        // Success path persisted exactly the new assistant message (the user
        // message was appended in start_turn, BEFORE run_turn's baseline).
        let persisted: Vec<&Record> = records.iter()
            .filter(|r| r.kind == JournalKind::MessagePersisted)
            .collect();
        assert_eq!(persisted.len(), 1, "one MessagePersisted on the success path");
        assert_eq!(persisted[0].thread_id.as_deref(), Some("jour-thread"));
        assert_eq!(persisted[0].payload["count"], 1);
        assert_eq!(persisted[0].payload["last_role"], "agent");

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// core-023: deleting a journalled thread appends a shape-only
    /// ThreadDeleted tombstone carrying the deleted id.
    #[test]
    fn serve_delete_thread_journals_thread_deleted_tombstone() {
        let data_dir = unique_data_dir("delete-tombstone");
        let (rt, cmd_tx, event_rx) =
            make_runtime_at(Arc::new(scripted_shared("journaled answer")), data_dir.clone());
        let server = std::thread::spawn(move || rt.serve());

        cmd_tx.send((1, Command::SendMessage { thread_id: "doomed-thread".into(), text: "hi".into() })).expect("send");
        // Wait for the turn so the thread exists in the map before deleting.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            assert!(std::time::Instant::now() < deadline, "turn did not finish in time");
            match event_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(Event::TurnFinished { ok: true, thread_id: t, .. }) if t == "doomed-thread" => break,
                Ok(_) => {}
                Err(_) => {}
            }
        }
        cmd_tx.send((2, Command::DeleteThread { thread_id: "doomed-thread".into() })).expect("send delete");
        drop(cmd_tx);
        server.join().expect("serve loop must not panic");

        assert!(
            !data_dir.join("threads").join("doomed-thread.json").exists(),
            "snapshot file removed"
        );
        let records = Journal::replay(&data_dir.join("journal").join("runtime.jsonl"))
            .expect("journal replayable");
        let deleted: Vec<&Record> = records.iter()
            .filter(|r| r.kind == JournalKind::ThreadDeleted)
            .collect();
        assert_eq!(deleted.len(), 1, "exactly one ThreadDeleted tombstone");
        assert_eq!(deleted[0].thread_id.as_deref(), Some("doomed-thread"));
        assert_eq!(deleted[0].payload["thread_id"], "doomed-thread");
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// core-023: delete still works when the journal is disabled (None) —
    /// no crash, file still removed.
    #[test]
    fn delete_thread_works_without_journal() {
        let data_dir = unique_data_dir("delete-no-journal");
        let (mut rt, cmd_tx, _event_rx) =
            make_runtime_at(Arc::new(empty_shared()), data_dir.clone());
        rt.journal = None;
        rt.threads.lock().unwrap().insert(
            "bare-thread".into(),
            Thread { id: "bare-thread".into(), title: "t".into(), messages: Vec::new(), updated_ms: 1 },
        );
        let server = std::thread::spawn(move || rt.serve());
        cmd_tx.send((1, Command::DeleteThread { thread_id: "bare-thread".into() })).expect("send delete");
        drop(cmd_tx);
        server.join().expect("serve loop must not panic"); // the no-crash assertion

        assert!(
            !data_dir.join("threads").join("bare-thread.json").exists(),
            "deletion still removes the snapshot"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn journal_sequence_continues_after_runtime_restart() {
        let data_dir = unique_data_dir("restart");
        let path = data_dir.join("journal").join("runtime.jsonl");

        serve_one_turn(&data_dir, "restart-thread", "first");
        let first_run_max = Journal::replay(&path)
            .expect("replay first run")
            .last()
            .expect("non-empty journal")
            .seq;
        assert!(first_run_max >= 3);

        // A SECOND Runtime over the same data dir resumes the sequence.
        serve_one_turn(&data_dir, "restart-thread", "second");
        let records = Journal::replay(&path).expect("replay both runs");
        let seqs: Vec<u64> = records.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, (1..=records.len() as u64).collect::<Vec<_>>(), "no seq gap or reuse across restart");
        assert_eq!(records[first_run_max as usize].seq, first_run_max + 1);
        assert_eq!(
            records.iter().filter(|r| r.kind == JournalKind::TurnStarted).count(),
            2,
            "each restart contributed its own TurnStarted"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn configure_provider_is_journalled_as_shape_without_secrets() {
        let data_dir = unique_data_dir("config-shape");
        let (rt, cmd_tx, _event_rx) =
            make_runtime_at(Arc::new(empty_shared()), data_dir.clone());
        let server = std::thread::spawn(move || rt.serve());
        cmd_tx
            .send((1, Command::ConfigureProvider { config: ProviderConfig {
                name: "main".into(),
                kind: z_protocol::ProviderKind::OpenAi,
                base_url: "https://api.example.test/v1".into(),
                model: "model-x".into(),
                api_key: "sk-SUPER-SECRET-VALUE".into(),
            }}))
            .expect("send ConfigureProvider");
        drop(cmd_tx);
        server.join().expect("serve loop must not panic");

        let raw = std::fs::read_to_string(data_dir.join("journal").join("runtime.jsonl"))
            .expect("journal exists");
        assert!(!raw.contains("sk-SUPER-SECRET-VALUE"), "API key leaked into the journal");
        assert!(raw.contains("\"configure_provider\""), "command recorded");
        assert!(raw.contains("\"api_key\""), "field NAME recorded (shape only)");
        assert!(!raw.contains("api.example.test"), "config values stay out of the payload");
        assert!(!raw.contains("model-x"), "config values stay out of the payload");
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    #[cfg(unix)]
    fn unwritable_journal_dir_never_breaks_a_turn() {
        {
            use std::os::unix::fs::PermissionsExt;
            // Root ignores permission bits, so the scenario cannot be
            // simulated — skip gracefully instead of failing.
            if unsafe { libc::geteuid() } == 0 {
                eprintln!("skipped: running as root ignores 0o000 permissions");
                return;
            }
            let data_dir = unique_data_dir("locked");
            let journal_dir = data_dir.join("journal");
            std::fs::create_dir_all(&journal_dir).expect("mkdir journal");
            std::fs::set_permissions(&journal_dir, std::fs::Permissions::from_mode(0o000))
                .expect("chmod journal dir");

            // Construction degrades to "no journal" instead of failing...
            assert!(open_runtime_journal(&data_dir).is_none());

            // ...and a full served turn completes normally without any
            // journal records.
            let (rt, cmd_tx, event_rx) = make_runtime_at(
                Arc::new(scripted_shared("still works")),
                data_dir.clone(),
            );
            assert!(rt.journal.is_none(), "journal must be disabled, not crashing");
            let server = std::thread::spawn(move || rt.serve());
            cmd_tx.send((1, Command::SendMessage { thread_id: "t".into(), text: "hi".into() })).expect("send");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                assert!(std::time::Instant::now() < deadline, "turn did not finish in time");
                match event_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(Event::TurnFinished { ok: true, .. }) => break,
                    Ok(_) => {}
                    Err(_) => {}
                }
            }
            drop(cmd_tx);
            server.join().expect("serve loop must not panic");
            assert!(!journal_dir.join("runtime.jsonl").exists());

            // Restore permissions so cleanup succeeds.
            std::fs::set_permissions(&journal_dir, std::fs::Permissions::from_mode(0o755))
                .expect("restore perms");
            let _ = std::fs::remove_dir_all(&data_dir);
        }
    }
}

#[cfg(test)]
mod settings_wiring_tests {
    use super::*;
    use super::steering_tests::make_runtime_at;

    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn unique_data_dir(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("zdt-setw-{tag}-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("threads")).expect("create temp data dir");
        dir
    }

    /// Always answers with one read-only tool call — never a plain-text
    /// completion — so the turn can only exit via the round cap.
    struct AlwaysToolProvider(std::sync::atomic::AtomicUsize);

    impl provider::Provider for AlwaysToolProvider {
        fn describe(&self) -> String {
            "always-tool".into()
        }
        fn stream(
            &self,
            _req: &provider::ChatRequest,
            _on_item: &mut dyn FnMut(provider::StreamItem),
        ) -> Result<provider::StreamOutcome, String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(provider::StreamOutcome {
                text: String::new(),
                tool_calls: vec![provider::ToolCallSpec {
                    id: "call-loop".into(),
                    name: "fs_read".into(),
                    arguments_json: r#"{"path":"README.md"}"#.into(),
                }],
            })
        }
    }

    #[test]
    fn runtime_honors_configured_round_cap_from_settings() {
        let data_dir = unique_data_dir("cap");
        std::fs::write(
            data_dir.join("settings.json"),
            r#"{"version":1,"values":{"max_tool_rounds":2,"approval_timeout_secs":7}}"#,
        )
        .expect("write settings.json");

        let provider = Arc::new(AlwaysToolProvider(std::sync::atomic::AtomicUsize::new(0)));
        let shared = Shared {
            provider: Mutex::new(Some(provider.clone() as Arc<dyn provider::Provider>)),
            provider_label: Mutex::new("always-tool".into()),
            project_root: Mutex::new(Some(std::env::current_dir().unwrap())),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            // Same production load path Runtime::new uses.
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::new(
                crate::settings::load(&data_dir),
            ))),
            write_grants: Mutex::new(HashMap::new()),
        };
        let (rt, _cmd_tx, _event_rx) = make_runtime_at(Arc::new(shared), data_dir.clone());
        assert_eq!(
            rt.shared.settings.lock().unwrap().get().approval_timeout_secs,
            7,
            "snapshot carries the configured approval timeout"
        );

        let thread = Thread {
            id: "settings-thread".into(),
            title: "settings".into(),
            messages: vec![StoredMessage {
                role: z_protocol::Role::User,
                text: "loop forever".into(),
                tool_calls: Vec::new(),
            }],
            updated_ms: 0,
        };
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        run_turn(
            Arc::clone(&rt.shared),
            event_tx,
            Arc::new(Mutex::new(())),
            data_dir.clone(),
            rt.journal.clone(),
            thread,
            "turn-settings".into(),
        );

        let mut finished = false;
        while let Ok(event) = event_rx.try_recv() {
            if let Event::TurnFinished { ok, error, .. } = event {
                finished = true;
                assert!(!ok, "capped turn must not report success");
                assert!(
                    error.unwrap_or_default().contains("stopped after 2 tool rounds"),
                    "stop message must reflect the configured cap"
                );
            }
        }
        assert!(finished, "turn never emitted TurnFinished");
        assert_eq!(
            provider.0.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "exactly max_tool_rounds provider rounds ran"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}

#[cfg(test)]
mod doom_loop_retry_tests {
    use super::*;
    use super::steering_tests::make_runtime_at;

    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn unique_data_dir(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("zdt-doom-{tag}-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("threads")).expect("create temp data dir");
        dir
    }

    #[test]
    fn classify_provider_error_table() {
        use RetryClass::*;
        let cases: &[(&str, RetryClass)] = &[
            // Transport / network
            ("stream read failed: unexpected EOF during SSE", Network),
            ("request failed: connection refused", Network),
            ("request failed: operation timed out", Network),
            // Rate limited
            ("provider returned HTTP 429: too many requests", RateLimited),
            ("provider returned HTTP 429: rate limit exceeded", RateLimited),
            // Server errors
            ("provider returned HTTP 500: internal server error", ServerError),
            ("provider returned HTTP 502: bad gateway", ServerError),
            ("5xx upstream exploded", ServerError),
            // Auth fails fast
            ("provider returned HTTP 401: unauthorized", Auth),
            ("provider returned HTTP 403: forbidden", Auth),
            ("Invalid API key supplied", Auth),
            // Everything else is fatal
            ("provider returned HTTP 400: bad request", Other),
            ("mystery failure", Other),
        ];
        for (text, expected) in cases {
            assert_eq!(&classify_provider_error(text), expected, "case: {text}");
        }
    }

    /// Always answers with the byte-identical read-only tool call — the
    /// canonical doom loop.
    struct LoopProvider(std::sync::atomic::AtomicUsize);

    impl provider::Provider for LoopProvider {
        fn describe(&self) -> String {
            "loop".into()
        }
        fn stream(
            &self,
            _req: &provider::ChatRequest,
            _on_item: &mut dyn FnMut(provider::StreamItem),
        ) -> Result<provider::StreamOutcome, String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(provider::StreamOutcome {
                text: String::new(),
                tool_calls: vec![provider::ToolCallSpec {
                    id: format!("call-{}", self.0.load(std::sync::atomic::Ordering::Relaxed)),
                    name: "fs_read".into(),
                    arguments_json: r#"{"path":"README.md"}"#.into(),
                }],
            })
        }
    }

    #[test]
    fn doom_loop_steers_once_then_fails_the_turn() {
        let data_dir = unique_data_dir("loop");
        std::fs::write(
            data_dir.join("settings.json"),
            r#"{"version":1,"values":{"doom_threshold":3,"max_tool_rounds":24}}"#,
        )
        .expect("write settings.json");

        let provider = Arc::new(LoopProvider(std::sync::atomic::AtomicUsize::new(0)));
        let shared = Shared {
            provider: Mutex::new(Some(provider.clone() as Arc<dyn provider::Provider>)),
            provider_label: Mutex::new("loop".into()),
            project_root: Mutex::new(Some(std::env::current_dir().unwrap())),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::new(
                crate::settings::load(&data_dir),
            ))),
            write_grants: Mutex::new(HashMap::new()),
        };
        let (rt, _cmd_tx, _runtime_events) = make_runtime_at(Arc::new(shared), data_dir.clone());

        let thread = Thread {
            id: "doom-thread".into(),
            title: "doom".into(),
            messages: vec![StoredMessage {
                role: z_protocol::Role::User,
                text: "read it again and again".into(),
                tool_calls: Vec::new(),
            }],
            updated_ms: 0,
        };
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        run_turn(
            Arc::clone(&rt.shared),
            event_tx,
            Arc::new(Mutex::new(())),
            data_dir.clone(),
            rt.journal.clone(),
            thread,
            "turn-doom".into(),
        );

        let mut finished = false;
        while let Ok(event) = event_rx.try_recv() {
            if let Event::TurnFinished { ok, error, .. } = event {
                finished = true;
                assert!(!ok, "a doom loop must fail the turn");
                assert!(
                    error.unwrap_or_default().contains("identical tool calls"),
                    "stop message must name the doom-loop breaker"
                );
            }
        }
        assert!(finished, "turn never emitted TurnFinished");

        // Steer at N=3 (call 3), hard-fail at 2N=6 (call 6): exactly 6 rounds.
        assert_eq!(
            provider.0.load(std::sync::atomic::Ordering::Relaxed),
            6,
            "steer at threshold N, fail at 2N"
        );

        // Exactly one persisted steering note.
        let saved = std::fs::read_to_string(data_dir.join("threads").join("doom-thread.json"))
            .expect("thread persisted");
        assert_eq!(
            saved.matches("Change approach or explain what you are waiting for.").count(),
            1,
            "exactly one steering message injected"
        );
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn parse_retry_after_table() {
        let cases: &[(&str, Option<u64>)] = &[
            ("HTTP 429 too many requests, retry after 5", Some(5)),
            ("provider returned HTTP 429: rate limit exceeded", None),
            ("slow down; Retry-After: 12 seconds", Some(12)),
            ("RETRY AFTER 90 before trying again", Some(90)),
            ("retry after soon-ish", None), // no digits -> no parse
            ("retry-after:", None),
            ("plain network failure: connection refused", None),
        ];
        for (text, expected) in cases {
            assert_eq!(&parse_retry_after(text), expected, "case: {text}");
        }
    }

    #[test]
    fn retry_backoff_honors_hint_capped_and_defaults_to_one_second() {
        // Hint honored...
        assert_eq!(retry_backoff_secs("retry after 5"), 5);
        // ...capped at 30 s so a hostile hint can't stall a turn...
        assert_eq!(retry_backoff_secs("retry after 3600"), 30);
        // ...flat 1 s default when no hint is present.
        assert_eq!(retry_backoff_secs("no hint here"), 1);
    }

    /// Fails the first call with a retryable rate-limit error, then answers
    /// with final text. Captures every request's Debug form so the retry can
    /// be proven byte-identical.
    struct FailOnceProvider {
        calls: std::sync::atomic::AtomicUsize,
        requests: std::sync::Mutex<Vec<String>>,
    }

    impl provider::Provider for FailOnceProvider {
        fn describe(&self) -> String {
            "fail-once".into()
        }
        fn stream(
            &self,
            req: &provider::ChatRequest,
            on_item: &mut dyn FnMut(provider::StreamItem),
        ) -> Result<provider::StreamOutcome, String> {
            self.requests.lock().unwrap().push(format!("{req:?}")); // full structural snapshot
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n == 0 {
                return Err("provider returned HTTP 429: rate limit exceeded".into());
            }
            let text = "recovered on retry".to_string();
            on_item(provider::StreamItem::TextDelta(text.clone()));
            Ok(provider::StreamOutcome { text, tool_calls: Vec::new() })
        }
    }

    /// core-017/018/019 together: one retryable failure then success means
    /// (a) exactly two provider attempts, (b) the retry passed the SAME
    /// request bytes as the failed first call, (c) exactly one provider_error
    /// breadcrumb carrying {attempt: 1, class: "ratelimited"}, (d) the turn
    /// still finishes ok.
    #[test]
    fn single_retry_replays_identical_request_journals_attempt_breadcrumb() {
        let data_dir = unique_data_dir("retry");
        let provider = Arc::new(FailOnceProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let shared = Shared {
            provider: Mutex::new(Some(provider.clone() as Arc<dyn provider::Provider>)),
            provider_label: Mutex::new("fail-once".into()),
            project_root: Mutex::new(Some(std::env::current_dir().unwrap())),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        let (rt, _cmd_tx, _runtime_events) = make_runtime_at(Arc::new(shared), data_dir.clone());

        let thread = Thread {
            id: "retry-thread".into(),
            title: "retry".into(),
            messages: vec![StoredMessage {
                role: z_protocol::Role::User,
                text: "try me".into(),
                tool_calls: Vec::new(),
            }],
            updated_ms: 0,
        };
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        run_turn(
            Arc::clone(&rt.shared),
            event_tx,
            Arc::new(Mutex::new(())),
            data_dir.clone(),
            rt.journal.clone(),
            thread,
            "turn-retry".into(),
        );

        // (d) turn finished OK despite the transient failure.
        let mut finished_ok = false;
        while let Ok(event) = event_rx.try_recv() {
            if let Event::TurnFinished { ok, .. } = event {
                finished_ok = ok;
            }
        }
        assert!(finished_ok, "transient failure must not fail the turn");

        // (a) exactly two recorded attempts.
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "one failed call plus one successful retry"
        );

        // (b) core-018: the retried request is byte-identical to the first.
        let reqs = provider.requests.lock().unwrap();
        assert_eq!(reqs.len(), 2, "every provider call captured its request");
        assert_eq!(
            reqs[0], reqs[1],
            "retry must replay the same request object byte-for-byte"
        );
        drop(reqs);

        // (c) core-019: exactly one error breadcrumb with the attempt field.
        let records = Journal::replay(&data_dir.join("journal").join("runtime.jsonl"))
            .expect("journal replayable");
        let crumbs: Vec<&Record> = records
            .iter()
            .filter(|r| r.kind == JournalKind::ProviderError)
            .collect();
        assert_eq!(crumbs.len(), 1, "exactly one provider_error breadcrumb");
        assert_eq!(crumbs[0].payload["attempt"], 1, "breadcrumb marks attempt 1");
        assert_eq!(
            crumbs[0].payload["class"], "ratelimited",
            "breadcrumb names the retry class"
        );
        assert_eq!(crumbs[0].thread_id.as_deref(), Some("retry-thread"));

        // Final answer still landed in the persisted history.
        let saved =
            std::fs::read_to_string(data_dir.join("threads").join("retry-thread.json"))
                .expect("thread persisted");
        assert!(saved.contains("recovered on retry"), "final text persisted");
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}

// ---------------------------------------------------------------------------
// edit-016/017: write grants (ADR-0010 §(5c))
// ---------------------------------------------------------------------------

#[cfg(test)]
mod supervision_verdict_tests {
    //! sup-008 (ADR-0016): the last supervision verdict rides on TurnFinished.

    use super::*;
    use super::steering_tests::{make_runtime_at, ScriptedProvider};

    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn unique_data_dir(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("zdt-verdict-{tag}-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("threads")).expect("create temp data dir");
        dir
    }

    /// One read-only tool call (captures NO evidence), then a final text
    /// claiming success — the fake-completion signature of sup-005/007.
    #[test]
    fn blocked_turn_carries_blocked_verdict_on_turn_finished() {
        let data_dir = unique_data_dir("blocked");
        let shared = Shared {
            provider: Mutex::new(Some(Arc::new(ScriptedProvider {
                outcomes: std::sync::Mutex::new(vec![
                    provider::StreamOutcome {
                        text: String::new(),
                        tool_calls: vec![provider::ToolCallSpec {
                            id: "call-1".into(),
                            name: "fs_read".into(),
                            arguments_json: r#"{"path":"README.md"}"#.into(),
                        }],
                    },
                    provider::StreamOutcome {
                        text: "All done, the tests pass.".into(),
                        tool_calls: Vec::new(),
                    },
                ]),
                requests: std::sync::Mutex::new(Vec::new()),
            }) as Arc<dyn provider::Provider>)),
            provider_label: Mutex::new("scripted".into()),
            project_root: Mutex::new(Some(std::env::current_dir().unwrap())),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        let (rt, _cmd_tx, _event_rx) = make_runtime_at(Arc::new(shared), data_dir.clone());
        assert!(rt.journal.is_some(), "sup-007 gate requires a live journal");

        let thread = Thread {
            id: "verdict-thread".into(),
            title: "verdict".into(),
            messages: vec![StoredMessage {
                role: z_protocol::Role::User,
                text: "do work".into(),
                tool_calls: Vec::new(),
            }],
            updated_ms: 0,
        };
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        run_turn(
            Arc::clone(&rt.shared),
            event_tx,
            Arc::new(Mutex::new(())),
            data_dir.clone(),
            rt.journal.clone(),
            thread,
            "turn-verdict".into(),
        );

        let mut finished = false;
        while let Ok(event) = event_rx.try_recv() {
            if let Event::TurnFinished { ok, verdict, .. } = event {
                finished = true;
                assert!(!ok, "unlinked success claim must fail the turn");
                let v = verdict.expect("evaluation happened, so a verdict must ride along");
                assert!(v.blocked, "verdict must be blocked");
                assert!(
                    v.reason.unwrap_or_default().contains("without recorded evidence"),
                    "reason names the missing-evidence failure"
                );
            }
        }
        assert!(finished, "turn never emitted TurnFinished");
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// sup-009 (ADR-0016): when the sup-007 gate blocks anyway and the
    /// unexecuted-tests detector fired (Tests claim, zero Tests evidence this
    /// turn), the turn error names it. Gating unchanged — extension only.
    #[test]
    fn blocked_turn_error_names_fired_unexecuted_detector() {
        let data_dir = unique_data_dir("detector-ext");
        let shared = Shared {
            provider: Mutex::new(Some(Arc::new(ScriptedProvider {
                outcomes: std::sync::Mutex::new(vec![
                    provider::StreamOutcome {
                        text: String::new(),
                        tool_calls: vec![provider::ToolCallSpec {
                            id: "call-1".into(),
                            name: "fs_read".into(),
                            arguments_json: r#"{"path":"README.md"}"#.into(),
                        }],
                    },
                    provider::StreamOutcome {
                        text: "All done, the tests pass.".into(),
                        tool_calls: Vec::new(),
                    },
                ]),
                requests: std::sync::Mutex::new(Vec::new()),
            }) as Arc<dyn provider::Provider>)),
            provider_label: Mutex::new("scripted".into()),
            project_root: Mutex::new(Some(std::env::current_dir().unwrap())),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        let (rt, _cmd_tx, _event_rx) = make_runtime_at(Arc::new(shared), data_dir.clone());

        let thread = Thread {
            id: "detector-thread".into(),
            title: "detector".into(),
            messages: vec![StoredMessage {
                role: z_protocol::Role::User,
                text: "do work".into(),
                tool_calls: Vec::new(),
            }],
            updated_ms: 0,
        };
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        run_turn(
            Arc::clone(&rt.shared),
            event_tx,
            Arc::new(Mutex::new(())),
            data_dir.clone(),
            rt.journal.clone(),
            thread,
            "turn-detector".into(),
        );

        let mut finished = false;
        while let Ok(event) = event_rx.try_recv() {
            if let Event::TurnFinished { ok, error, .. } = event {
                finished = true;
                assert!(!ok);
                // fs_read captures no evidence, so "the tests pass" is an
                // unexecuted Tests claim on top of the unlinked-claim block.
                let err = error.expect("blocked turn carries a reason");
                assert!(
                    err.contains("without recorded evidence [detectors: unexecuted-tests]"),
                    "reason must fold in the fired detector: {err}"
                );
            }
        }
        assert!(finished, "turn never emitted TurnFinished");
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    /// A turn whose final text carries no success claim never evaluates —
    /// TurnFinished goes out with verdict: None even on the happy path.
    #[test]
    fn unevaluated_turn_finishes_with_no_verdict() {
        let data_dir = unique_data_dir("plain");
        let shared = Shared {
            provider: Mutex::new(Some(Arc::new(ScriptedProvider {
                outcomes: std::sync::Mutex::new(vec![provider::StreamOutcome {
                    text: "Here is what you asked for.".into(),
                    tool_calls: Vec::new(),
                }]),
                requests: std::sync::Mutex::new(Vec::new()),
            }) as Arc<dyn provider::Provider>)),
            provider_label: Mutex::new("scripted".into()),
            project_root: Mutex::new(Some(std::env::current_dir().unwrap())),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        };
        let (rt, _cmd_tx, _event_rx) = make_runtime_at(Arc::new(shared), data_dir.clone());

        let thread = Thread {
            id: "plain-thread".into(),
            title: "plain".into(),
            messages: vec![StoredMessage {
                role: z_protocol::Role::User,
                text: "hello".into(),
                tool_calls: Vec::new(),
            }],
            updated_ms: 0,
        };
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        run_turn(
            Arc::clone(&rt.shared),
            event_tx,
            Arc::new(Mutex::new(())),
            data_dir.clone(),
            rt.journal.clone(),
            thread,
            "turn-plain".into(),
        );

        let mut finished = false;
        while let Ok(event) = event_rx.try_recv() {
            if let Event::TurnFinished { ok, verdict, .. } = event {
                finished = true;
                assert!(ok);
                assert!(verdict.is_none(), "no evaluation this turn, no verdict");
            }
        }
        assert!(finished, "turn never emitted TurnFinished");
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}

#[cfg(test)]
mod write_grant_tests {
    use super::*;
    use super::steering_tests::ScriptedProvider;

    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_root(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("zdt-grant-{tag}-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp root");
        dir
    }

    fn empty_shared(root: PathBuf) -> Shared {
        Shared {
            provider: Mutex::new(None),
            provider_label: Mutex::new(String::new()),
            project_root: Mutex::new(Some(root)),
            index: Mutex::new(None),
            gate: ApprovalGate::default(),
            cancelled: Mutex::new(std::collections::HashSet::new()),
            steering: Mutex::new(HashMap::new()),
            settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
            write_grants: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn grant_is_exclusive_across_threads_and_reentrant_for_owner() {
        let root = temp_root("excl");
        std::fs::write(root.join("x.txt"), "x").unwrap();
        let shared = empty_shared(root.clone());

        grant_write(&shared, &root, "A", "x.txt").expect("A acquires");
        let err = grant_write(&shared, &root, "B", "x.txt").unwrap_err();
        assert!(err.contains("another task"), "{err}");
        // Reentrant for the owner.
        grant_write(&shared, &root, "A", "x.txt").expect("A re-enters");
        // B's release is a no-op against A's hold.
        release_write(&shared, &root, "B", "x.txt");
        assert!(grant_write(&shared, &root, "B", "x.txt").is_err(), "B still blocked");
        // A releases; B succeeds.
        release_write(&shared, &root, "A", "x.txt");
        grant_write(&shared, &root, "B", "x.txt").expect("B succeeds after A releases");
    }

    #[test]
    fn canonical_aliases_resolve_to_one_grant_key() {
        let root = temp_root("alias");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let shared = empty_shared(root.clone());

        // ADR-0010 acceptance: alias spellings collapse onto ONE grant key.
        grant_write(&shared, &root, "A", "./sub/../a.txt").expect("aliased spelling acquires");
        assert!(
            grant_write(&shared, &root, "B", "a.txt").is_err(),
            "plain spelling must collide with the aliased hold"
        );
    }

    #[test]
    fn fs_write_turn_acquires_and_releases_the_grant_around_each_call() {
        // Two scripted fs_write calls as thread "tg": both must execute, and
        // the registry must be EMPTY afterwards (per-call acquire→release).
        let root = temp_root("turn");
        let outcomes = vec![
            provider::StreamOutcome {
                text: String::new(),
                tool_calls: vec![provider::ToolCallSpec {
                    id: "w1".into(),
                    name: "fs_write".into(),
                    arguments_json: r#"{"path":"doc.txt","content":"one"}"#.into(),
                }],
            },
            provider::StreamOutcome {
                text: String::new(),
                tool_calls: vec![provider::ToolCallSpec {
                    id: "w2".into(),
                    name: "fs_write".into(),
                    arguments_json: r#"{"path":"doc.txt","content":"two"}"#.into(),
                }],
            },
            provider::StreamOutcome { text: "done".into(), tool_calls: Vec::new() },
        ];
        let mut shared = empty_shared(root.clone());
        shared.provider = Mutex::new(Some(Arc::new(ScriptedProvider {
            outcomes: std::sync::Mutex::new(outcomes),
            requests: std::sync::Mutex::new(Vec::new()),
        })));
        shared.provider_label = Mutex::new("scripted".into());
        // Pre-resolve both approvals so the gate never blocks the worker.
        shared.gate.resolve("w1", true);
        shared.gate.resolve("w2", true);
        let shared = Arc::new(shared);

        let thread = Thread {
            id: "tg".into(),
            title: "grant turn".into(),
            messages: vec![StoredMessage {
                role: z_protocol::Role::User,
                text: "write doc twice".into(),
                tool_calls: Vec::new(),
            }],
            updated_ms: 0,
        };
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        run_turn(
            Arc::clone(&shared),
            event_tx,
            Arc::new(Mutex::new(())),
            temp_root("data"),
            None,
            thread,
            "turn-grant".into(),
        );

        let mut finished_ok = false;
        while let Ok(event) = event_rx.try_recv() {
            if let Event::TurnFinished { ok, .. } = event {
                finished_ok = ok;
            }
        }
        assert!(finished_ok, "turn did not finish cleanly");
        assert_eq!(
            std::fs::read_to_string(root.join("doc.txt")).unwrap(),
            "two",
            "both fs_write calls executed"
        );
        let leftovers = debug_grants(&shared);
        assert!(leftovers.is_empty(), "no leftover grant after completion: {leftovers:?}");
    }
}

// ---------------------------------------------------------------------------
// Orchestrator skeleton (orch-003, ADR-0012): cap-limited nested-task runner.
// ---------------------------------------------------------------------------

/// ADR-0012 caps (orch-012): global default 2 concurrent children; per-parent
/// 1 lands with the full spawn policy. Constants until orch-021 wires
/// dev-mode settings.
pub const ORCH_MAX_CONCURRENT: usize = 2;

/// orch-012: hard max concurrent tasks across ALL parents once per-parent
/// caps land (per-parent is later). Today's global cap must stay below it.
pub const ORCH_CEILING: usize = 4;

/// Nested task work for [`Orchestrator`]. Skeleton stand-in: orch-005 replaces
/// this with a real ChildSpec driving one nested `run_turn` (ADR-0012 §3).
pub type TaskBody = Box<dyn FnOnce() -> Result<(), String> + Send>;

/// Inbox command for the orchestrator thread.
pub enum OrchCommand {
    /// `deadline_ms`: absolute wall-clock budget deadline (ms since the Unix
    /// epoch, same clock as [`now_ms`]); None = no deadline (orch-004).
    EnqueueTask { id: String, body: TaskBody, deadline_ms: Option<u128> },
    Shutdown,
}

/// Minimal orchestrator (ADR-0012 decision 2): one named `z-orchestrator`
/// thread with an mpsc inbox; parks in `recv_timeout(1s)` so the deadline
/// sweep fires without a timer thread. Admits up to [`ORCH_MAX_CONCURRENT`]
/// tasks at once — overflow QUEUES as pending rather than being refused —
/// each on a named `z-subagent` worker that journals Running → Done/Failed
/// through the TaskStore journal segment.
pub struct Orchestrator {
    tx: Sender<OrchCommand>,
}

impl Orchestrator {
    /// Spawns the orchestrator thread over `<tasks_dir>/tasks.jsonl`.
    pub fn spawn(tasks_dir: PathBuf) -> Self {
        // orch-024 (ADR-0012): recover before admitting new work. Best-effort —
        // recovery failure must never prevent startup (same policy as journaling).
        match recover_orphans(&tasks_dir) {
            Ok((failed, pending_ready)) => log::info!(
                "orchestrator: crash recovery: {failed} orphaned task(s) failed, \
                 {pending_ready} pending-ready task(s) await re-submission"
            ),
            Err(e) => log::warn!("orchestrator: crash recovery skipped: {e}"),
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("z-orchestrator".into())
            .spawn(move || orchestrator_loop(tasks_dir, rx))
            .expect("could not spawn orchestrator thread");
        Self { tx }
    }

    pub fn enqueue_task(&self, id: String, body: TaskBody) {
        self.enqueue_task_with_deadline(id, body, None);
    }

    /// orch-004: enqueue with a wall-clock budget deadline (absolute ms since
    /// the Unix epoch). A task still running past it is swept to Failed by
    /// the orchestrator loop's 1s tick.
    pub fn enqueue_task_with_deadline(
        &self,
        id: String,
        body: TaskBody,
        deadline_ms: Option<u128>,
    ) {
        let _ = self.tx.send(OrchCommand::EnqueueTask { id, body, deadline_ms });
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(OrchCommand::Shutdown);
    }
}

fn orchestrator_loop(tasks_dir: PathBuf, inbox: Receiver<OrchCommand>) {
    // id -> its z-subagent worker; presence == currently Running.
    let mut running: HashMap<String, std::thread::JoinHandle<()>> = HashMap::new();
    let mut queued: VecDeque<(String, TaskBody, Option<u128>)> = VecDeque::new();
    // orch-004: deadlines of RUNNING tasks, shared with the workers. Presence
    // here is the right-to-write a final transition: a worker removes its own
    // id BEFORE its final TaskStore write, and the sweep only fails ids it can
    // claim — so exactly one side appends the last event and a Done can never
    // land after a budget-Failed in the journal fold.
    let live: Arc<Mutex<HashMap<String, u128>>> = Arc::new(Mutex::new(HashMap::new()));

    loop {
        match inbox.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(OrchCommand::EnqueueTask { id, body, deadline_ms }) => {
                queued.push_back((id, body, deadline_ms))
            }
            Ok(OrchCommand::Shutdown) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                sweep_deadlines(&tasks_dir, &live);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // Reap finished workers (each already journaled its own Done/Failed),
        // then admit queued tasks while slots remain under the global cap.
        let finished: Vec<String> = running
            .iter()
            .filter(|(_, handle)| handle.is_finished())
            .map(|(id, _)| id.clone())
            .collect();
        for id in finished {
            if let Some(handle) = running.remove(&id) {
                let _ = handle.join();
            }
        }
        while running.len() < ORCH_MAX_CONCURRENT {
            // orch-012: admission stays under both the global cap and the
            // hard ceiling (the ceiling bites once per-parent caps land).
            assert!(
                running.len() < ORCH_CEILING,
                "running ({}) exceeded ORCH_CEILING ({ORCH_CEILING})",
                running.len()
            );
            let Some((id, body, deadline_ms)) = queued.pop_front() else {
                break;
            };
            if let Err(e) = TaskStore::transition(&tasks_dir, &id, TaskStatus::Running) {
                log::warn!("orchestrator: cannot mark {id} running: {e}");
                continue;
            }
            // Register the deadline BEFORE spawning: the worker may finish and
            // look itself up immediately, so the entry must already exist.
            // u128::MAX = no deadline (never expires).
            live.lock()
                .unwrap()
                .insert(id.clone(), deadline_ms.unwrap_or(u128::MAX));
            let spawned = std::thread::Builder::new().name("z-subagent".into()).spawn({
                let tasks_dir = tasks_dir.clone();
                let id = id.clone();
                let live = Arc::clone(&live);
                move || {
                    let status = match body() {
                        Ok(()) => TaskStatus::Done,
                        Err(_) => TaskStatus::Failed,
                    };
                    // Give up our right-to-write FIRST: if the sweep already
                    // claimed us (budget-Failed appended), skip our transition
                    // so Done can never follow Failed in the fold.
                    if live.lock().unwrap().remove(&id).is_none() {
                        return;
                    }
                    if let Err(e) = TaskStore::transition(&tasks_dir, &id, status) {
                        log::warn!("orchestrator: task {id} final transition failed: {e}");
                    }
                }
            });
            match spawned {
                Ok(handle) => {
                    running.insert(id, handle);
                }
                Err(e) => {
                    live.lock().unwrap().remove(&id);
                    log::warn!("orchestrator: could not spawn worker for {id}: {e}");
                    let _ = TaskStore::transition(&tasks_dir, &id, TaskStatus::Failed);
                }
            }
        }
    }
    // Shutdown ordering per ADR-0012: stop admitting (caller drops its sender)
    // → join outstanding children → exit; journal records survive for resume.
    for (_, handle) in running.drain() {
        let _ = handle.join();
    }
}

/// orch-024 crash recovery (ADR-0012): after a restart, task bodies are gone —
/// closures die with the process — so a folded Running task can never complete
/// and cannot be re-run here. Lazy-honest recovery: mark every orphaned Running
/// task Failed ("orphaned by restart") and report how many Pending tasks are
/// ready (deps all Done) so an external caller — app startup or a test — can
/// re-submit them with fresh bodies. Returns `(failed, pending_ready)`.
pub fn recover_orphans(tasks_dir: &Path) -> Result<(usize, usize), String> {
    let path = tasks_dir.join(format!("{TASKS_SEGMENT}.jsonl"));
    if !path.exists() {
        return Ok((0, 0)); // fresh workspace: nothing to recover
    }
    let view = TasksView::fold(&path)?;
    let orphaned: Vec<String> = view
        .tasks
        .values()
        .filter(|t| t.status == TaskStatus::Running)
        .map(|t| t.id.clone())
        .collect();
    let mut failed = 0;
    for id in &orphaned {
        log::warn!("orchestrator: task {id} orphaned by restart; failing");
        match TaskStore::transition(tasks_dir, id, TaskStatus::Failed) {
            Ok(()) => failed += 1,
            Err(e) => log::warn!("orchestrator: could not fail orphan {id}: {e}"),
        }
    }
    let pending_ready = view.ready_set();
    if !pending_ready.is_empty() {
        log::info!(
            "orchestrator: pending-ready task(s) need re-submission: {pending_ready:?}"
        );
    }
    Ok((failed, pending_ready.len()))
}

/// orch-004 backstop sweep (ADR-0012 §5c/§6): on the 1s tick, fail any RUNNING
/// task past its wall-clock deadline. Claims overdue ids out of `live`
/// atomically before appending Failed, so a worker finishing concurrently
/// cannot append Done after the budget-Failed.
/// ponytail: swept bodies are not cancelled mid-flight — a hung body's thread
/// is only joined at shutdown. Acceptable at personal scale; wire
/// Shared.cancelled into bodies when orch-005's ChildSpec lands.
fn sweep_deadlines(tasks_dir: &Path, live: &Mutex<HashMap<String, u128>>) {
    let now = now_ms() as u128;
    let expired: Vec<String> = {
        let mut live = live.lock().unwrap();
        let expired: Vec<String> = live
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            live.remove(id);
        }
        expired
    };
    for id in expired {
        log::warn!("orchestrator: task {id} exceeded its budget deadline; failing");
        if let Err(e) = TaskStore::transition(tasks_dir, &id, TaskStatus::Failed) {
            log::warn!("orchestrator: budget sweep could not fail {id}: {e}");
        }
    }
}

#[cfg(test)]
mod orchestrator_tests {
    use super::*;
    use crate::reducer::TasksView;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_tasks_dir(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("zdt-orch-{tag}-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp tasks dir");
        dir
    }

    fn tasks_path(dir: &Path) -> PathBuf {
        dir.join(format!("{}.jsonl", "tasks"))
    }

    /// Polls `f` until it returns true or the timeout elapses (last try wins).
    fn wait_until(timeout_ms: u64, mut f: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if f() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn three_enqueued_tasks_all_complete_under_global_cap_of_two() {
        let dir = temp_tasks_dir("cap");
        for i in 0..3 {
            TaskStore::create(&dir, &format!("t{i}")).expect("create task");
        }
        let orch = Orchestrator::spawn(dir.clone());

        // Guard: bodies track live concurrency and the max ever observed.
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        for i in 0..3 {
            let concurrent = Arc::clone(&concurrent);
            let max_seen = Arc::clone(&max_seen);
            orch.enqueue_task(
                format!("t{i}"),
                Box::new(move || {
                    let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                    max_seen.fetch_max(now, Ordering::SeqCst);
                    // Long enough that all 3 overlap without a cap; short
                    // enough that the test stays fast under it.
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    concurrent.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                }),
            );
        }

        let path = tasks_path(&dir);
        assert!(
            wait_until(10_000, || TasksView::fold(&path)
                .ok()
                .map_or(false, |v| v.tasks.values().all(|t| t.status == TaskStatus::Done))),
            "all 3 tasks must reach Done"
        );
        orch.shutdown();

        let view = TasksView::fold(&path).expect("fold");
        for i in 0..3 {
            assert_eq!(
                view.tasks[&format!("t{i}")].status,
                TaskStatus::Done,
                "t{i} must be Done"
            );
        }
        assert!(
            max_seen.load(Ordering::SeqCst) <= ORCH_MAX_CONCURRENT,
            "observed {} concurrent workers, cap is {}",
            max_seen.load(Ordering::SeqCst),
            ORCH_MAX_CONCURRENT
        );
        assert_eq!(concurrent.load(Ordering::SeqCst), 0, "no worker left running");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_body_transitions_task_to_failed() {
        let dir = temp_tasks_dir("failed");
        TaskStore::create(&dir, "bad").expect("create bad");

        let orch = Orchestrator::spawn(dir.clone());
        orch.enqueue_task(
            "bad".into(),
            Box::new(|| Err("boom".into())),
        );

        let path = tasks_path(&dir);
        assert!(
            wait_until(10_000, || TasksView::fold(&path)
                .ok()
                .and_then(|v| v.tasks.get("bad").map(|t| t.status == TaskStatus::Failed))
                .unwrap_or(false)),
            "task with failing body must reach Failed"
        );
        orch.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_past_deadline_sweeps_to_failed_and_done_cannot_follow() {
        let dir = temp_tasks_dir("deadline");
        TaskStore::create(&dir, "late").expect("create late");
        let orch = Orchestrator::spawn(dir.clone());

        let body_finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&body_finished);
        // Body must outlive the first 1s sweep tick, else the worker would
        // legitimately win the race (it finished before any sweep ran).
        orch.enqueue_task_with_deadline(
            "late".into(),
            Box::new(move || {
                std::thread::sleep(std::time::Duration::from_millis(2500));
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }),
            Some(1), // deadline in the past (epoch + 1ms)
        );

        let path = tasks_path(&dir);
        assert!(
            wait_until(10_000, || TasksView::fold(&path)
                .ok()
                .and_then(|v| v.tasks.get("late").map(|t| t.status == TaskStatus::Failed))
                .unwrap_or(false)),
            "overdue task must be swept to Failed"
        );
        // Journal evidence: the failed transition is durably on disk.
        let raw = std::fs::read_to_string(&path).expect("tasks journal");
        assert!(raw.contains("\"status\":\"failed\""), "journal carries failed");

        // Once the body definitively finished AND another sweep tick passed,
        // the task must STILL be Failed — no trailing Done in the fold.
        assert!(wait_until(10_000, || body_finished.load(Ordering::SeqCst)));
        std::thread::sleep(std::time::Duration::from_millis(1300));
        let view = TasksView::fold(&path).expect("fold");
        assert_eq!(view.tasks["late"].status, TaskStatus::Failed);
        orch.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_finishing_before_deadline_stays_done() {
        let dir = temp_tasks_dir("inbudget");
        TaskStore::create(&dir, "quick").expect("create quick");
        let orch = Orchestrator::spawn(dir.clone());
        orch.enqueue_task_with_deadline(
            "quick".into(),
            Box::new(|| Ok(())),
            Some(now_ms() as u128 + 60_000),
        );

        let path = tasks_path(&dir);
        assert!(
            wait_until(10_000, || TasksView::fold(&path)
                .ok()
                .and_then(|v| v.tasks.get("quick").map(|t| t.status == TaskStatus::Done))
                .unwrap_or(false)),
            "in-budget task must reach Done"
        );
        // A couple of sweep ticks later the sweeper has not touched it.
        std::thread::sleep(std::time::Duration::from_millis(1300));
        let view = TasksView::fold(&path).expect("fold");
        assert_eq!(view.tasks["quick"].status, TaskStatus::Done);
        orch.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn orch_cap_constants_match_adr_0012() {
        assert_eq!(ORCH_MAX_CONCURRENT, 2, "ADR-0012 §4 global default");
        assert_eq!(ORCH_CEILING, 4, "ADR-0012 §4 hard ceiling");
        assert!(ORCH_MAX_CONCURRENT <= ORCH_CEILING);
    }

    // orch-024 ---------------------------------------------------------------

    #[test]
    fn recover_orphans_fails_running_and_counts_pending_ready() {
        let dir = temp_tasks_dir("recover");
        TaskStore::create(&dir, "orphan").expect("create orphan");
        TaskStore::transition(&dir, "orphan", TaskStatus::Running).expect("orphan running");
        TaskStore::create(&dir, "ready").expect("create ready");
        TaskStore::create(&dir, "finished").expect("create finished");
        TaskStore::transition(&dir, "finished", TaskStatus::Done).expect("finished done");

        assert_eq!(
            recover_orphans(&dir).expect("recover"),
            (1, 1),
            "one Running orphan failed; one dep-less Pending counted ready"
        );

        let view = TasksView::fold(&tasks_path(&dir)).expect("refold");
        assert_eq!(view.tasks["orphan"].status, TaskStatus::Failed, "orphan failed");
        assert_eq!(view.tasks["ready"].status, TaskStatus::Pending);
        assert_eq!(view.tasks["finished"].status, TaskStatus::Done, "Done untouched");
    }

    #[test]
    fn recover_orphans_on_empty_journal_is_noop() {
        let dir = temp_tasks_dir("recover-empty");
        std::fs::write(tasks_path(&dir), "").expect("empty tasks journal");
        assert_eq!(recover_orphans(&dir).expect("recover"), (0, 0));
    }

    #[test]
    fn recover_orphans_does_not_count_blocked_pending_nor_touch_done() {
        let dir = temp_tasks_dir("recover-blocked");
        // Unknown dep id blocks forever (orch-002 safe default).
        TaskStore::create_with_deps(&dir, "blocked", &["ghost".into()]).expect("create blocked");
        TaskStore::create(&dir, "finished").expect("create finished");
        TaskStore::transition(&dir, "finished", TaskStatus::Done).expect("finished done");

        assert_eq!(
            recover_orphans(&dir).expect("recover"),
            (0, 0),
            "blocked Pending is in the fold but not ready"
        );
        let view = TasksView::fold(&tasks_path(&dir)).expect("refold");
        assert_eq!(view.tasks["blocked"].status, TaskStatus::Pending);
        assert_eq!(view.tasks["finished"].status, TaskStatus::Done);
    }
}

// ---------------------------------------------------------------------------
// core-021/022/025: thread list / rename / delete / duplicate / most-recent
// ---------------------------------------------------------------------------

#[cfg(test)]
mod thread_management_tests {
    use super::*;

    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_data_dir(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("zdt-threads-{tag}-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("threads")).expect("create temp threads dir");
        dir
    }

    fn msg(text: &str) -> StoredMessage {
        StoredMessage { role: z_protocol::Role::User, text: text.into(), tool_calls: Vec::new() }
    }

    /// A Runtime over `dir`, pre-seeded with `seed` threads (no restore).
    fn runtime_with(
        dir: PathBuf,
        seed: Vec<Thread>,
    ) -> (
        Runtime,
        Sender<(u64, Command)>,
        std::sync::mpsc::Receiver<Event>,
    ) {
        let journal = open_runtime_journal(&dir);
        // core-026: pick up unreadable thread files on disk so ghosts and
        // `corrupt_threads()` reflect what a real startup restore would see.
        let (_, _, corrupt) = restore_threads(&dir);
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        (
            Runtime {
                shared: Arc::new(Shared {
                    provider: Mutex::new(None),
                    provider_label: Mutex::new(String::new()),
                    project_root: Mutex::new(None),
                    index: Mutex::new(None),
                    gate: ApprovalGate::default(),
                    cancelled: Mutex::new(std::collections::HashSet::new()),
                    steering: Mutex::new(HashMap::new()),
                    settings: Mutex::new(Arc::new(crate::settings::Snapshot::default())),
                    write_grants: Mutex::new(HashMap::new()),
                }),
                data_dir: dir,
                threads: Mutex::new(seed.into_iter().map(|t| (t.id.clone(), t)).collect()),
                event_tx,
                cmd_rx,
                journal,
                most_recent_restored: None,
                corrupt_threads: corrupt,
            },
            cmd_tx,
            event_rx,
        )
    }

    /// Serve the command batch on a real serve() loop, then drain every event.
    /// Dropping cmd_tx shuts the loop down, so this is race-free.
    fn run(
        rt: Runtime,
        cmd_tx: Sender<(u64, Command)>,
        event_rx: std::sync::mpsc::Receiver<Event>,
        cmds: Vec<(u64, Command)>,
    ) -> Vec<Event> {
        let server = std::thread::spawn(move || rt.serve());
        for (id, command) in cmds {
            cmd_tx.send((id, command)).expect("send command");
        }
        drop(cmd_tx);
        server.join().expect("serve loop must not panic");
        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    fn last_thread_list(events: &[Event]) -> &[z_protocol::ThreadInfo] {
        events
            .iter()
            .rev()
            .find_map(|e| match e {
                Event::ThreadList { threads } => Some(threads.as_slice()),
                _ => None,
            })
            .expect("at least one ThreadList event")
    }

    #[test]
    fn rename_updates_title_in_memory_and_persists_to_disk() {
        let dir = temp_data_dir("rename");
        let seed = Thread {
            id: "r1".into(),
            title: "old".into(),
            messages: vec![msg("hi")],
            updated_ms: 10,
        };
        let (rt, cmd_tx, event_rx) = runtime_with(dir.clone(), vec![seed]);
        let events = run(
            rt,
            cmd_tx,
            event_rx,
            vec![(
                1,
                Command::RenameThread { thread_id: "r1".into(), title: "renamed".into() },
            )],
        );

        // Refreshed list reflects the new title...
        let listed = last_thread_list(&events);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "renamed");
        // ...and the change landed on disk.
        let saved = std::fs::read_to_string(dir.join("threads").join("r1.json"))
            .expect("thread persisted");
        let back: Thread = serde_json::from_str(&saved).unwrap();
        assert_eq!(back.title, "renamed");

        // Titles are clamped to 120 chars.
        let long: String = std::iter::repeat('x').take(300).collect();
        let (rt2, cmd_tx2, event_rx2) = runtime_with(dir.clone(), vec![back]);
        let events = run(
            rt2,
            cmd_tx2,
            event_rx2,
            vec![(1, Command::RenameThread { thread_id: "r1".into(), title: long })],
        );
        assert_eq!(last_thread_list(&events)[0].title.chars().count(), 120);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_thread_from_memory_and_disk() {
        let dir = temp_data_dir("delete");
        let seed = Thread {
            id: "gone".into(),
            title: "doomed".into(),
            messages: vec![msg("bye")],
            updated_ms: 5,
        };
        let path = dir.join("threads").join("gone.json");
        std::fs::write(&path, serde_json::to_string(&seed).unwrap()).expect("seed on disk");

        let (rt, cmd_tx, event_rx) = runtime_with(dir.clone(), vec![seed]);
        let events = run(rt, cmd_tx, event_rx, vec![(1, Command::DeleteThread { thread_id: "gone".into() })]);

        let listed = last_thread_list(&events);
        assert!(listed.is_empty(), "memory entry removed");
        assert!(!path.exists(), "disk file removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_copies_all_messages_under_new_id_and_both_exist() {
        let dir = temp_data_dir("dup");
        let source = Thread {
            id: "src".into(),
            title: "original".into(),
            messages: vec![msg("one"), msg("two"), msg("three")],
            updated_ms: 7,
        };
        // Seed on disk too, so "both exist" covers the durable state.
        std::fs::write(
            dir.join("threads").join("src.json"),
            serde_json::to_string(&source).unwrap(),
        )
        .expect("seed source on disk");
        let (rt, cmd_tx, event_rx) = runtime_with(dir.clone(), vec![source]);
        let events = run(
            rt,
            cmd_tx,
            event_rx,
            vec![(
                1,
                Command::DuplicateThread { thread_id: "src".into(), new_id: "dst".into() },
            )],
        );

        // Both exist in the refreshed list; the copy carries every message.
        let listed = last_thread_list(&events);
        assert_eq!(listed.len(), 2, "source and copy both listed");
        let dst = listed.iter().find(|t| t.id == "dst").expect("copy listed");
        assert_eq!(dst.message_count, 3);
        assert!(dst.title.ends_with("(copy)"));
        assert!(listed.iter().any(|t| t.id == "src" && t.message_count == 3));

        // Copy persisted under its own id; original untouched on disk.
        let saved: Thread =
            serde_json::from_str(&std::fs::read_to_string(dir.join("threads").join("dst.json")).expect("copy persisted")).unwrap();
        assert_eq!(saved.id, "dst");
        assert_eq!(saved.messages.len(), 3);
        assert_eq!(saved.messages[2].text, "three");
        assert!(dir.join("threads").join("src.json").exists(), "source still on disk");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_threads_returns_recency_order_and_correct_counts_after_mixed_operations() {
        let dir = temp_data_dir("mixed");
        let mk = |id: &str, updated_ms: u64| Thread {
            id: id.into(),
            title: format!("thread {id}"),
            messages: vec![msg("m1"), msg("m2")],
            updated_ms,
        };
        let (rt, cmd_tx, event_rx) = runtime_with(
            dir.clone(),
            vec![mk("aaa", 100), mk("bbb", 200), mk("ccc", 300)],
        );
        let events = run(
            rt,
            cmd_tx,
            event_rx,
            vec![
                (1, Command::ListThreads),
                (2, Command::RenameThread { thread_id: "bbb".into(), title: "bee".into() }),
                (3, Command::DeleteThread { thread_id: "ccc".into() }),
                (4, Command::ListThreads),
            ],
        );

        // First list: three rows, newest activity first.
        let first = events
            .iter()
            .find_map(|e| match e {
                Event::ThreadList { threads } => Some(threads),
                _ => None,
            })
            .expect("initial ThreadList");
        let ids: Vec<&str> = first.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["ccc", "bbb", "aaa"], "sorted by updated_ms desc");
        assert!(first.iter().all(|t| t.message_count == 2));

        // After rename + delete: two rows, order preserved by stored recency.
        let second = last_thread_list(&events);
        let ids: Vec<&str> = second.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["bbb", "aaa"]);
        assert_eq!(second[0].title, "bee");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_reports_the_most_recently_modified_thread_file() {
        let dir = temp_data_dir("recent");
        let older = Thread {
            id: "older".into(),
            title: "older".into(),
            messages: Vec::new(),
            updated_ms: 1,
        };
        let newer = Thread {
            id: "newer".into(),
            title: "newer".into(),
            messages: Vec::new(),
            updated_ms: 2,
        };
        std::fs::write(dir.join("threads").join("older.json"), serde_json::to_string(&older).unwrap())
            .expect("write older");
        std::thread::sleep(std::time::Duration::from_millis(80));
        std::fs::write(dir.join("threads").join("newer.json"), serde_json::to_string(&newer).unwrap())
            .expect("write newer");

        let (threads, most_recent, corrupt) = restore_threads(&dir);
        assert_eq!(threads.len(), 2, "both valid files restored");
        assert!(corrupt.is_empty(), "no corrupt files yet");

        // A later corrupt file must not steal "most recent" nor break restore.
        std::thread::sleep(std::time::Duration::from_millis(80));
        std::fs::write(dir.join("threads").join("junk.json"), "{ not json").expect("write junk");
        let (threads, most_recent, corrupt) = restore_threads(&dir);
        assert_eq!(threads.len(), 2, "corrupt file skipped");
        assert_eq!(most_recent.as_deref(), Some("newer"));
        assert_eq!(corrupt.len(), 1, "corrupt file reported");
        assert!(corrupt[0].starts_with("junk.json: "));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_thread_files_surface_as_ghosts_and_accessor_reports_them() {
        let dir = temp_data_dir("corrupt");
        let good = Thread {
            id: "ok".into(),
            title: "good".into(),
            messages: vec![msg("hi")],
            updated_ms: 9,
        };
        std::fs::write(dir.join("threads").join("ok.json"), serde_json::to_string(&good).unwrap())
            .expect("write valid thread");
        std::fs::write(dir.join("threads").join("junk.json"), "{ not json").expect("write junk");

        let (rt, cmd_tx, event_rx) = runtime_with(dir.clone(), vec![good]);
        assert_eq!(rt.corrupt_threads().len(), 1, "one corrupt file reported");
        let events = run(
            rt,
            cmd_tx,
            event_rx,
            vec![
                (1, Command::ListThreads),
                (2, Command::DeleteThread { thread_id: "junk".into() }),
            ],
        );

        // First list shows both the real thread and the read-only ghost.
        let first = events
            .iter()
            .find_map(|e| match e {
                Event::ThreadList { threads } => Some(threads),
                _ => None,
            })
            .expect("initial ThreadList");
        let ghost = first.iter().find(|t| t.id == "junk").expect("ghost listed");
        assert_eq!(ghost.title, "[corrupt] junk.json");
        assert_eq!(ghost.message_count, 0);
        assert_eq!(
            first.iter().find(|t| t.id == "ok").map(|t| t.message_count),
            Some(1),
            "valid thread listed normally"
        );

        // Deleting the ghost removes the file and the gap from later lists.
        let final_listed = last_thread_list(&events);
        assert!(!final_listed.iter().any(|t| t.id == "junk"), "ghost gone after delete");
        assert!(
            !dir.join("threads").join("junk.json").exists(),
            "corrupt file removed from disk"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_data_dir_reports_no_corrupt_threads() {
        let dir = temp_data_dir("clean");
        let (rt, _cmd_tx, _event_rx) = runtime_with(dir.clone(), Vec::new());
        assert!(rt.corrupt_threads().is_empty(), "clean dir has no corrupt files");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ui-040: GetEvidence folds the journal's evidence records; the optional
    // turn_id narrows to one turn.
    #[test]
    fn get_evidence_folds_journal_evidence_into_a_summary() {
        let dir = temp_data_dir("evidence");
        // Seed the journal through the same record path the execution
        // chokepoint uses, so the fold sees real EvidenceRecorded records.
        let journal = std::sync::Mutex::new(
            Journal::open(&dir.join("journal"), "runtime").expect("open journal"),
        );
        crate::evidence::record(
            &journal,
            &crate::evidence::Evidence::tests("t", "u1", 5, 0, "cargo test"),
        );
        crate::evidence::record(
            &journal,
            &crate::evidence::Evidence::build("t", "u2", Some(1), "make"),
        );
        drop(journal); // release the handle before the runtime reopens it

        let (rt, cmd_tx, event_rx) = runtime_with(dir.clone(), Vec::new());
        let events = run(
            rt,
            cmd_tx,
            event_rx,
            vec![
                (1, Command::GetEvidence { turn_id: None }),
                (2, Command::GetEvidence { turn_id: Some("u2".into()) }),
            ],
        );

        let mut summaries = events.iter().filter_map(|e| match e {
            Event::EvidenceSummary { items } => Some(items.as_slice()),
            _ => None,
        });
        let all = summaries.next().expect("an EvidenceSummary for GetEvidence");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].kind, "tests");
        assert!(all[0].ok);
        assert_eq!(all[0].summary, "cargo test");
        assert_eq!(all[1].kind, "build");
        assert!(!all[1].ok);

        let filtered =
            summaries.next().expect("one summary per GetEvidence command");
        assert_eq!(filtered.len(), 1, "turn_id filter keeps only that turn");
        assert_eq!(filtered[0].kind, "build");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
