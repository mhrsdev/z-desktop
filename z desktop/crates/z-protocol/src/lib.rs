//! Z Desktop protocol — the versioned contract between the UI and the Agent
//! Runtime.
//!
//! The UI is a projection of runtime state. It never mutates agent state
//! directly: it sends [`Command`]s and renders [`Event`]s. Both sides are
//! versioned through [`PROTOCOL_VERSION`]; an envelope mismatch is a hard error,
//! not a best-effort guess.

use serde::{Deserialize, Serialize};

/// Bumped on any breaking change to `Command` or `Event`.
pub const PROTOCOL_VERSION: u32 = 1;

/// Opaque identifiers. Strings rather than newtypes so they survive serde
/// round-trips without custom impls, while staying unguessable in logs.
pub type Id = String;

/// Who authored a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Agent,
}

/// Risk classification of a tool call. Drives whether approval is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Reads within the project scope. Auto-allowed.
    ReadOnly,
    /// Writes inside the project scope. Requires approval by default.
    Write,
    /// Executes commands or touches anything outside project scope.
    Execute,
}

/// A command from the UI (or any client) to the Agent Runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Send user text; starts a new turn on the thread.
    SendMessage { thread_id: Id, text: String },
    /// Steer a running turn: queue user text for injection between tool
    /// rounds. If no turn is running, the text waits for the next turn's
    /// first drain (never lost, never a second concurrent turn).
    EnqueueMessage { thread_id: Id, text: String },
    /// Cancel the running turn. Already-applied work stays applied.
    CancelTurn { thread_id: Id },
    /// Answer a pending approval for a tool call.
    ResolveApproval { call_id: Id, approved: bool },
    /// Point the workspace at a project root (indexes it).
    OpenProject { path: String },
    /// Replace provider configuration (BYOK). Keys live in config, not here.
    ConfigureProvider { config: ProviderConfig },
}

/// Provider configuration. One active provider at a time in Personal v0.1;
/// the registry already accepts several so the Router can grow without a
/// breaking change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    /// "openai" | "anthropic" — the two wire formats v0.1 speaks.
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    /// API key. Persisted only in the local config file, never logged.
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
}

/// An event from the Agent Runtime to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// The runtime accepted the command. Acceptance is not success.
    Accepted { command_id: u64 },
    TurnStarted { thread_id: Id, turn_id: Id },
    /// User text was queued for steering on a running turn. `depth` is the
    /// queue length after this enqueue (drives the queue-depth indicator).
    SteeringQueued { thread_id: Id, depth: u64 },
    /// Streaming assistant text delta.
    TextDelta { thread_id: Id, turn_id: Id, delta: String },
    TextDone { thread_id: Id, turn_id: Id },
    /// A tool call step began. Shown immediately as "in progress".
    StepStarted { thread_id: Id, turn_id: Id, call_id: Id, tool: String, detail: String },
    StepFinished {
        thread_id: Id,
        turn_id: Id,
        call_id: Id,
        ok: bool,
        summary: String,
    },
    /// A tool call needs explicit approval before it runs.
    ApprovalRequested { thread_id: Id, call_id: Id, tool: String, detail: String, risk: Risk },
    TurnFinished { thread_id: Id, turn_id: Id, ok: bool, error: Option<String> },
    /// Project indexing progress/completion.
    ProjectIndexed { path: String, files: u64, symbols: u64 },
    /// Provider configuration accepted/rejected with reason.
    ProviderStatus { ok: bool, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_through_json() {
        let event = Event::StepFinished {
            thread_id: "t1".into(),
            turn_id: "u1".into(),
            call_id: "c1".into(),
            ok: false,
            summary: "exit code 1".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Event::StepFinished { ok: false, .. }));
    }

    #[test]
    fn commands_are_tagged_snake_case() {
        let json = serde_json::to_string(&Command::CancelTurn { thread_id: "t".into() }).unwrap();
        assert!(json.contains(r#""type":"cancel_turn""#), "{json}");
    }

    #[test]
    fn enqueue_message_round_trips_through_json() {
        let command = Command::EnqueueMessage { thread_id: "t1".into(), text: "steer left".into() };
        let json = serde_json::to_string(&command).unwrap();
        assert!(json.contains(r#""type":"enqueue_message""#), "{json}");
        let back: Command = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Command::EnqueueMessage { text, .. } if text == "steer left"));
    }

    #[test]
    fn steering_queued_event_carries_depth() {
        let event = Event::SteeringQueued { thread_id: "t1".into(), depth: 3 };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"steering_queued""#), "{json}");
        let back: Event = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Event::SteeringQueued { depth: 3, .. }));
    }
}