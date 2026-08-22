//! Agent Runtime — the headless heart of Z Desktop.
//!
//! Owns threads and turns. Commands arrive on one channel; events leave on
//! another. Each turn runs on its own worker thread so the command loop stays
//! responsive to cancellation and approvals while a turn streams.

use crate::{provider, repo::RepoIndex, tools};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
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
}

pub struct Runtime {
    shared: Arc<Shared>,
    data_dir: PathBuf,
    threads: Mutex<HashMap<Id, Thread>>,
    event_tx: Sender<Event>,
    cmd_rx: Receiver<(u64, Command)>,
}

/// Where sessions/config live. `Z_DESKTOP_DATA` overrides; default is `data/`
/// beside the working directory so a dev checkout is self-contained.
pub fn data_dir() -> PathBuf {
    std::env::var_os("Z_DESKTOP_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"))
}

const MAX_TOOL_ROUNDS: usize = 24;
const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

impl Runtime {
    pub fn new(event_tx: Sender<Event>, cmd_rx: Receiver<(u64, Command)>) -> Self {
        let data_dir = data_dir();
        let _ = std::fs::create_dir_all(data_dir.join("threads"));
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
        Self {
            shared: Arc::new(Shared {
                provider: Mutex::new(None),
                provider_label: Mutex::new("no provider configured".into()),
                project_root: Mutex::new(None),
                index: Mutex::new(None),
                gate: ApprovalGate::default(),
                cancelled: Mutex::new(std::collections::HashSet::new()),
            }),
            data_dir,
            threads: Mutex::new(threads),
            event_tx,
            cmd_rx,
        }
    }

    /// Run the command loop until the channel closes (app shutdown).
    pub fn serve(self) {
        while let Ok((command_id, command)) = self.cmd_rx.recv() {
            let _ = self.event_tx.send(Event::Accepted { command_id });
            match command {
                Command::ConfigureProvider { config } => self.configure_provider(config),
                Command::OpenProject { path } => self.open_project(path),
                Command::SendMessage { thread_id, text } => self.start_turn(thread_id, text),
                Command::CancelTurn { thread_id } => {
                    self.shared.cancelled.lock().unwrap().insert(thread_id);
                }
                Command::ResolveApproval { call_id, approved } => {
                    self.shared.gate.resolve(&call_id, approved);
                }
            }
        }
        log::info!("command channel closed; runtime stopping");
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

        // The turn runs on a worker so CancelTurn/ResolveApproval stay live.
        let shared = Arc::clone(&self.shared);
        let event_tx = self.event_tx.clone();
        let threads_lock = Arc::new(Mutex::new(())); // serialise history mutation
        let data_dir = self.data_dir.clone();
        std::thread::Builder::new()
            .name("z-turn".into())
            .spawn(move || {
                run_turn(shared, event_tx, threads_lock, data_dir, thread, turn_id);
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

/// One full agent turn: stream → (tool calls → approve → execute)×N → done.
fn run_turn(
    shared: Arc<Shared>,
    event_tx: Sender<Event>,
    _history_lock: Arc<Mutex<()>>,
    data_dir: PathBuf,
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

    let Some(provider) = shared.provider.lock().unwrap().clone() else {
        finish(false, Some("no provider configured — set one in Settings".into()));
        return;
    };
    let Some(root) = shared.project_root.lock().unwrap().clone() else {
        finish(false, Some("no project open — open a folder first".into()));
        return;
    };

    for round in 0..MAX_TOOL_ROUNDS {
        if is_cancelled(&shared, &thread_id) {
            save_thread(&data_dir, &thread);
            finish(false, Some("cancelled by user".into()));
            return;
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
                    // Transient provider failures get one retry before failing
                    // the turn; the user's message is never lost either way.
                    if round == 0 && e.contains("stream read failed") {
                        continue;
                    }
                    save_thread(&data_dir, &thread);
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
            save_thread(&data_dir, &thread);
            finish(true, None);
            return;
        }

        // Assistant message carrying its tool calls, then results.
        let stored_calls: Vec<StoredToolCall> = outcome
            .tool_calls
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

        for call in &outcome.tool_calls {
            if is_cancelled(&shared, &thread_id) {
                save_thread(&data_dir, &thread);
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
                match shared.gate.wait(&call.id, APPROVAL_TIMEOUT) {
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

            let output = tools::execute(tools::ToolInvocation {
                name: &call.name,
                args,
                project_root: &root,
            });
            record_result(
                &mut thread,
                &event_tx,
                &turn_id,
                &call.id,
                output.ok,
                output.text.lines().next().unwrap_or("").chars().take(120).collect(),
            );
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
        }
    }
    save_thread(&data_dir, &thread);
    finish(false, Some(format!("stopped after {MAX_TOOL_ROUNDS} tool rounds")));
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

fn save_thread(data_dir: &std::path::Path, thread: &Thread) {
    let path = data_dir.join("threads").join(format!("{}.json", thread.id));
    if let Ok(json) = serde_json::to_string_pretty(thread) {
        let _ = std::fs::write(path, json);
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
}
