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
    /// Request the thread list (answered with [`Event::ThreadList`]).
    ListThreads,
    /// Set a thread's title (clamped to 120 chars).
    RenameThread { thread_id: Id, title: String },
    /// Remove a thread from memory and from disk.
    DeleteThread { thread_id: Id },
    /// Deep-copy a thread's messages under `new_id`.
    DuplicateThread { thread_id: Id, new_id: Id },
    /// Request the folded supervision-evidence summary (answered with
    /// [`Event::EvidenceSummary`]). `Some(turn_id)` limits to one turn.
    GetEvidence { turn_id: Option<Id> },
    /// sup-017: appeal a supervision verdict on `turn_id`. Journaled by the
    /// runtime (sup-024 persistence) and honored by the gate from then on.
    AppealVerdict { turn_id: Id, reason: String },
    /// set-004: change one runtime setting live (ADR-0011 swap-on-write).
    /// Only whitelisted keys with in-range values apply; rejections ride
    /// [`Event::ProviderStatus`] with a `settings:` message prefix.
    SetSetting { key: String, value: serde_json::Value },
}

/// One row of a thread listing (core-021): cheap projection, never carries
/// the messages themselves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: Id,
    pub title: String,
    pub message_count: u64,
    /// Last activity time (ms since epoch), best-effort.
    pub updated_ms: u64,
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

/// sup-008 (ADR-0016): supervision verdict summary riding on
/// [`Event::TurnFinished`]. Mirrors z-core's `SupervisionVerdict` without a
/// cross-crate type dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisionVerdictInfo {
    pub blocked: bool,
    pub reason: Option<String>,
}

/// One folded evidence row for the UI badge strip (ui-040). A projection of
/// z-core's journal evidence — never the full records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceInfo {
    pub id: Id,
    /// snake_case evidence kind ("build", "tests", …).
    pub kind: String,
    pub ok: bool,
    pub summary: String,
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
    TurnFinished {
        thread_id: Id,
        turn_id: Id,
        ok: bool,
        error: Option<String>,
        /// sup-008 (ADR-0016): supervision outcome for this turn. Present only
        /// when a supervision evaluation actually ran during the turn; absent
        /// (never renamed) otherwise.
        #[serde(default)]
        verdict: Option<SupervisionVerdictInfo>,
    },
    /// Project indexing progress/completion.
    ProjectIndexed { path: String, files: u64, symbols: u64 },
    /// Provider configuration accepted/rejected with reason.
    ProviderStatus { ok: bool, message: String },
    /// Refreshed snapshot of all threads, most recent first. Emitted both on
    /// request and after every thread mutation so the UI stays consistent.
    ThreadList { threads: Vec<ThreadInfo> },
    /// Folded supervision-evidence summary (ui-040), journal order, capped to
    /// the most recent 50 rows. Answered on [`Command::GetEvidence`].
    EvidenceSummary { items: Vec<EvidenceInfo> },
    /// sup-017/024: an appealed verdict was journaled as overridden. Echoes
    /// the turn id so the UI can drop its blocked badge when
    /// `blocked_cleared` (i.e. the override is durably recorded).
    VerdictOverridden { turn_id: Id, blocked_cleared: bool },
    /// set-004: a setting was validated, persisted, and swapped into the
    /// live snapshot. `value` echoes the applied value.
    SettingChanged { key: String, value: serde_json::Value },
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

    // sup-008: TurnFinished round-trips its verdict; payloads from before the
    // field existed (no "verdict" key) still deserialize to None.
    #[test]
    fn turn_finished_round_trips_with_and_without_verdict() {
        let blocked = Event::TurnFinished {
            thread_id: "t1".into(),
            turn_id: "u1".into(),
            ok: false,
            error: Some("claimed Tests success without recorded evidence".into()),
            verdict: Some(SupervisionVerdictInfo {
                blocked: true,
                reason: Some("claimed Tests success without recorded evidence".into()),
            }),
        };
        let back: Event =
            serde_json::from_str(&serde_json::to_string(&blocked).unwrap()).unwrap();
        match back {
            Event::TurnFinished { ok: false, verdict: Some(v), .. } => {
                assert!(v.blocked);
                assert_eq!(
                    v.reason.as_deref(),
                    Some("claimed Tests success without recorded evidence")
                );
            }
            other => panic!("wrong event: {other:?}"),
        }

        let plain = Event::TurnFinished {
            thread_id: "t1".into(),
            turn_id: "u2".into(),
            ok: true,
            error: None,
            verdict: None,
        };
        let back: Event =
            serde_json::from_str(&serde_json::to_string(&plain).unwrap()).unwrap();
        assert!(matches!(back, Event::TurnFinished { ok: true, verdict: None, .. }));

        // Additive evolution (ADR-0018): old payload without the field.
        let legacy = r#"{"type":"turn_finished","thread_id":"t1","turn_id":"u3","ok":true,"error":null}"#;
        let back: Event = serde_json::from_str(legacy).unwrap();
        assert!(matches!(back, Event::TurnFinished { verdict: None, .. }));
    }

    // ui-040: GetEvidence/EvidenceSummary round-trip; kind stays a plain
    // snake_case string so the UI never needs the z-core type.
    #[test]
    fn get_evidence_round_trips_through_json() {
        let command = Command::GetEvidence { turn_id: Some("u1".into()) };
        let back: Command =
            serde_json::from_str(&serde_json::to_string(&command).unwrap()).unwrap();
        assert!(matches!(back, Command::GetEvidence { turn_id: Some(t) } if t == "u1"));

        let event = Event::EvidenceSummary {
            items: vec![EvidenceInfo {
                id: "ev-1".into(),
                kind: "tests".into(),
                ok: false,
                summary: "3 failed".into(),
            }],
        };
        let back: Event = serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        match back {
            Event::EvidenceSummary { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].kind, "tests");
                assert!(!items[0].ok);
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    // sup-017/024: appeal command and override event round-trip; both stay
    // snake_case-tagged like every other variant.
    #[test]
    fn appeal_and_override_round_trip_through_json() {
        let cmd = Command::AppealVerdict {
            turn_id: "u1".into(),
            reason: "the tests really ran".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""type":"appeal_verdict""#), "{json}");
        let back: Command = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(back, Command::AppealVerdict { turn_id, reason } if turn_id == "u1" && reason == "the tests really ran")
        );

        let event = Event::VerdictOverridden { turn_id: "u1".into(), blocked_cleared: true };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"verdict_overridden""#), "{json}");
        let back: Event = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Event::VerdictOverridden { blocked_cleared: true, .. }));
    }
}