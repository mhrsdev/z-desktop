//! Agent Runtime — the headless heart of Z Desktop.
//!
//! Owns threads and turns. Commands arrive on one channel; events leave on
//! another. Each turn runs on its own worker thread so the command loop stays
//! responsive to cancellation and approvals while a turn streams.

use crate::journal::{Journal, JournalKind, Record, RecordDraft};
use crate::reducer::{TaskStatus, TaskStore};
use crate::{provider, repo::RepoIndex, tools};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use z_protocol::{Command, Event, Id, ProviderConfig, Risk};

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
}

impl Thread {
    fn new(id: Id) -> Self {
        Self { id, title: "New chat".into(), messages: Vec::new() }
    }
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

impl Runtime {
    pub fn new(event_tx: Sender<Event>, cmd_rx: Receiver<(u64, Command)>) -> Self {
        let data_dir = data_dir();
        let _ = std::fs::create_dir_all(data_dir.join("threads"));
        // set-002/003 (ADR-0011): load settings once into the shared snapshot;
        // hand-edited files apply on relaunch, SetSetting swaps the Arc later.
        let settings = Arc::new(crate::settings::Snapshot::new(crate::settings::load(&data_dir)));
        let mut threads = HashMap::new();
        // Restore persisted sessions; a corrupt file is skipped, not fatal.
        if let Ok(entries) = std::fs::read_dir(data_dir.join("threads")) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                match std::fs::read_to_string(entry.path())
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str::<Thread>(&s).map_err(|e| e.to_string()))
                {
                    Ok(thread) => {
                        threads.insert(thread.id.clone(), thread);
                    }
                    Err(e) => log::warn!("skipping unreadable session {:?}: {e}", entry.path()),
                }
            }
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
        }
    }

    /// Run the command loop until the channel closes (app shutdown).
    pub fn serve(self) {
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
    let finish = |ok: bool, error: Option<String>| {
        let _ = event_tx.send(Event::TurnFinished {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            ok,
            error,
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

    // core-011/core-012 (ADR-0011): clone the settings Arc ONCE at turn start;
    // a concurrent SetSetting applies to the next turn, never mid-turn.
    let settings = Arc::clone(shared.settings.lock().unwrap().get());
    let max_tool_rounds = settings.max_tool_rounds;
    // core-014 (ADR-0017 D1): per-turn fingerprint counter for the doom-loop
    // breaker. Turn-local lifetime — no Shared field, no reset logic.
    let mut call_counts: HashMap<u64, usize> = HashMap::new();
    let doom_threshold = settings.doom_threshold;

    for round in 0..max_tool_rounds {
        if is_cancelled(&shared, &thread_id) {
            persist(&thread);
            finish(false, Some("cancelled by user".into()));
            return;
        }

        // Steering drain (core-006): between tool rounds, queued user text
        // is appended as one combined user message so the next provider
        // round sees it. Round 0 drains only what arrived before the turn's
        // provider call; later rounds pick up mid-turn steering.
        if round > 0 {
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

        let request = build_request(provider.as_ref(), &thread, &shared, &root);
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
                    // server errors back off 1 s first. Auth/Other fail now —
                    // same key, same answer. Either way the user's message
                    // is never lost.
                    let class = classify_provider_error(&e);
                    let retryable = matches!(
                        class,
                        RetryClass::Network | RetryClass::RateLimited | RetryClass::ServerError
                    );
                    if retryable && round == 0 {
                        if !matches!(class, RetryClass::Network) {
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
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
            let _ = event_tx.send(Event::TextDone { thread_id: thread_id.clone(), turn_id: turn_id.clone() });
            persist(&thread);
            finish(true, None);
            return;
        }

        // Assistant message carrying its tool calls, then results.
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

    // Budget: fixed parts first, then whatever room is left for history.
    let tools_tokens: usize = tools::definitions()
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
        tools: tools::definitions(),
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
}

// ---------------------------------------------------------------------------
// edit-016/017: write grants (ADR-0010 §(5c))
// ---------------------------------------------------------------------------

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

/// ADR-0012 caps (orch-012 fixes the numbers): global 2 concurrent children,
/// per-parent 1 and hard ceiling 4 land with the full spawn policy. Constants
/// until orch-021 wires dev-mode settings.
pub const ORCH_MAX_CONCURRENT: usize = 2;

/// Nested task work for [`Orchestrator`]. Skeleton stand-in: orch-005 replaces
/// this with a real ChildSpec driving one nested `run_turn` (ADR-0012 §3).
pub type TaskBody = Box<dyn FnOnce() -> Result<(), String> + Send>;

/// Inbox command for the orchestrator thread.
pub enum OrchCommand {
    EnqueueTask { id: String, body: TaskBody },
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
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("z-orchestrator".into())
            .spawn(move || orchestrator_loop(tasks_dir, rx))
            .expect("could not spawn orchestrator thread");
        Self { tx }
    }

    pub fn enqueue_task(&self, id: String, body: TaskBody) {
        let _ = self.tx.send(OrchCommand::EnqueueTask { id, body });
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(OrchCommand::Shutdown);
    }
}

fn orchestrator_loop(tasks_dir: PathBuf, inbox: Receiver<OrchCommand>) {
    // id -> its z-subagent worker; presence == currently Running.
    let mut running: HashMap<String, std::thread::JoinHandle<()>> = HashMap::new();
    let mut queued: VecDeque<(String, TaskBody)> = VecDeque::new();

    loop {
        match inbox.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(OrchCommand::EnqueueTask { id, body }) => queued.push_back((id, body)),
            Ok(OrchCommand::Shutdown) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // TODO(orch-004, ADR-0012 §5c): sweep tasks past their
                // wall-clock budget deadline here — set cancelled (same path
                // as CancelTurn) and wait for the worker to exit. No budgets
                // exist yet, so nothing to sweep.
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
            let Some((id, body)) = queued.pop_front() else { break };
            if let Err(e) = TaskStore::transition(&tasks_dir, &id, TaskStatus::Running) {
                log::warn!("orchestrator: cannot mark {id} running: {e}");
                continue;
            }
            let spawned = std::thread::Builder::new().name("z-subagent".into()).spawn({
                let tasks_dir = tasks_dir.clone();
                let id = id.clone();
                move || {
                    let status = match body() {
                        Ok(()) => TaskStatus::Done,
                        Err(_) => TaskStatus::Failed,
                    };
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
}
