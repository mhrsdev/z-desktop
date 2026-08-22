//! Conversation projection — the view's model of a thread.
//!
//! This is a projection of Agent Runtime state, rebuilt from [`z_protocol`]
//! events; it holds no authority. Reference data remains for visual QA and
//! long-thread tests, but never masquerades as a user's session on launch.

use z_core::runtime::Thread;

/// Who sent a message. Drives which side of the surface it lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Right-hand side, per the chat spec.
    User,
    /// Left-hand side.
    Agent,
}

/// State of one step in an agent plan.
///
/// `Failed` exists and is rendered distinctly because a step that did not work
/// must never display as completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Completed,
    InProgress,
    Pending,
    Failed,
}

impl StepState {
    pub fn label(self) -> &'static str {
        match self {
            StepState::Completed => "Completed",
            StepState::InProgress => "In progress",
            StepState::Pending => "Pending",
            StepState::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanStep {
    pub title: String,
    pub state: StepState,
}

/// Work the agent is doing right now.
#[derive(Debug, Clone)]
pub struct Progress {
    pub title: String,
    /// The file or operation currently being touched. Rendered isolated, so a
    /// path keeps its own direction inside prose running the other way.
    pub detail: String,
    pub done: u32,
    pub total: u32,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.done as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub author: String,
    pub timestamp: String,
    pub body: String,
    pub lead_in: Option<String>,
    pub plan: Vec<PlanStep>,
    pub progress: Option<Progress>,
    pub actions: bool,
}

impl Message {
    fn plain(role: Role, author: &str, timestamp: &str, body: &str) -> Self {
        Self {
            role,
            author: author.into(),
            timestamp: timestamp.into(),
            body: body.into(),
            lead_in: None,
            plan: Vec::new(),
            progress: None,
            actions: false,
        }
    }

    /// A live agent turn: body grows as text deltas arrive.
    fn streaming_agent() -> Self {
        Self { ..Self::plain(Role::Agent, "Z", "", "") }
    }
}

/// One entry in the Context Panel's list.
#[derive(Debug, Clone, Copy)]
pub struct ContextEntry {
    pub label: &'static str,
    pub count: u32,
}

/// Everything the workspace displays.
pub struct Conversation {
    pub messages: Vec<Message>,
    pub project: String,
    pub branch: String,
    pub context_usage: f32,
    pub entries: Vec<ContextEntry>,
}

impl Conversation {
    /// A truthful first-run workspace: no prior messages, no inferred project
    /// and no fabricated agent work.
    pub fn empty() -> Self {
        Self {
            messages: Vec::new(),
            project: "No project selected".into(),
            branch: "—".into(),
            context_usage: 0.0,
            entries: Vec::new(),
        }
    }

    /// Project a persisted runtime thread into view messages. Tool calls
    /// become plan steps whose state mirrors the recorded outcome exactly.
    pub fn from_thread(thread: &Thread) -> Self {
        let mut conversation = Self::empty();
        for m in &thread.messages {
            match m.role {
                z_protocol::Role::User => {
                    if m.tool_calls.is_empty() && !m.text.is_empty() {
                        conversation.messages.push(Message::plain(Role::User, "You", "", &m.text));
                    }
                    // Tool-result carriers are not user-visible chat.
                }
                z_protocol::Role::Agent => {
                    let mut message = Message::plain(Role::Agent, "Z", "", &m.text);
                    for call in &m.tool_calls {
                        let state = match call.ok {
                            Some(true) => StepState::Completed,
                            Some(false) => StepState::Failed,
                            None => StepState::InProgress,
                        };
                        message.plan.push(PlanStep {
                            title: format!("{} · {}", call.name, call.summary),
                            state,
                        });
                    }
                    if !message.plan.is_empty() {
                        let done = message
                            .plan
                            .iter()
                            .filter(|s| matches!(s.state, StepState::Completed | StepState::InProgress))
                            .count() as u32;
                        message.progress = Some(Progress {
                            title: "Z is working".into(),
                            detail: String::new(),
                            done,
                            total: message.plan.len() as u32,
                        });
                    }
                    conversation.messages.push(message);
                }
            }
        }
        conversation.project = thread.title.clone();
        conversation
    }

    /// The last agent message, creating one when a turn starts streaming.
    pub fn ensure_streaming_agent(&mut self) -> &mut Message {
        if !self.messages.last().is_some_and(|m| m.role == Role::Agent) {
            self.messages.push(Message::streaming_agent());
        }
        self.messages.last_mut().unwrap()
    }

    /// Complete the newest in-progress step whose title starts with `tool`.
    pub fn finish_step(&mut self, tool: &str, ok: bool, summary: &str) {
        for message in self.messages.iter_mut().rev() {
            if message.role != Role::Agent {
                continue;
            }
            if let Some(step) = message
                .plan
                .iter_mut()
                .rev()
                .find(|s| s.state == StepState::InProgress && s.title.starts_with(tool))
            {
                step.state = if ok { StepState::Completed } else { StepState::Failed };
                step.title = format!("{tool} · {summary}");
                return;
            }
        }
    }
}

impl Conversation {
    /// The reference conversation, for visual QA only.
    pub fn reference() -> Self {
        Self {
            messages: vec![
                Message::plain(
                    Role::User,
                    "You",
                    "10:42 AM",
                    "Refactor the authentication flow and improve the session handling.",
                ),
                Message {
                    lead_in: Some(
                        "I'll refactor the authentication flow to improve security and session handling."
                            .into(),
                    ),
                    plan: vec![
                        PlanStep { title: "Analyze current auth flow".into(), state: StepState::Completed },
                        PlanStep { title: "Refactor session management".into(), state: StepState::Completed },
                        PlanStep { title: "Improve token validation".into(), state: StepState::InProgress },
                        PlanStep { title: "Add unit tests".into(), state: StepState::Pending },
                        PlanStep { title: "Update documentation".into(), state: StepState::Pending },
                    ],
                    progress: Some(Progress {
                        title: "Z is making changes".into(),
                        detail: "src/auth/session.ts".into(),
                        done: 3,
                        total: 5,
                    }),
                    ..Message::plain(Role::Agent, "Z", "10:42 AM", "Here's my plan:")
                },
                Message::plain(
                    Role::User,
                    "You",
                    "10:45 AM",
                    "Sounds good. Make sure we preserve backward compatibility.",
                ),
                Message {
                    actions: true,
                    ..Message::plain(
                        Role::Agent,
                        "Z",
                        "10:46 AM",
                        "Will do. I'll add regression tests to ensure backward compatibility and \
                         validate all existing sessions continue to work.",
                    )
                },
            ],
            project: "Reference Project".into(),
            branch: "main".into(),
            context_usage: 0.32,
            entries: vec![
                ContextEntry { label: "Plan", count: 5 },
                ContextEntry { label: "Agents", count: 4 },
                ContextEntry { label: "Changes", count: 4 },
                ContextEntry { label: "Threads", count: 3 },
                ContextEntry { label: "Resources", count: 7 },
                ContextEntry { label: "Notes", count: 2 },
            ],
        }
    }

    /// A reference conversation whose newest step failed, so the failure
    /// rendering can be proved rather than assumed.
    pub fn with_failed_step() -> Self {
        let mut conversation = Self::reference();
        if let Some(message) = conversation.messages.iter_mut().find(|m| !m.plan.is_empty()) {
            for step in message.plan.iter_mut() {
                if step.state == StepState::InProgress {
                    step.state = StepState::Failed;
                }
            }
            let total = message.plan.len() as u32;
            let done = message
                .plan
                .iter()
                .filter(|s| matches!(s.state, StepState::Completed))
                .count() as u32;
            message.progress = Some(Progress {
                title: "Z is making changes".into(),
                detail: "src/auth/session.ts".into(),
                done,
                total,
            });
        }
        conversation
    }

    /// The reference conversation repeated until it holds `count` messages,
    /// so virtualization can be proved rather than assumed.
    pub fn long(count: usize) -> Self {
        let mut conversation = Self::reference();
        let pattern = conversation.messages.clone();
        conversation.messages.clear();
        for index in 0..count {
            conversation.messages.push(pattern[index % pattern.len()].clone());
        }
        conversation
    }
}

/// Utilities the Floating Tool offers.
pub const FLOATING_TOOLS: &[(&str, Option<&str>)] = &[
    ("Active Tasks", Some("3 in progress")),
    ("Terminal", None),
    ("Debug Console", None),
    ("Agent Timeline", None),
    ("Notes", None),
    ("3D Tools", None),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_conversation_never_claims_prior_agent_work() {
        let conversation = Conversation::empty();
        assert!(conversation.messages.is_empty());
        assert_eq!(conversation.context_usage, 0.0);
    }

    #[test]
    fn progress_fraction_is_bounded() {
        let p = Progress { title: "t".into(), detail: "d".into(), done: 9, total: 5 };
        assert_eq!(p.fraction(), 1.0);

        let empty = Progress { title: "t".into(), detail: "d".into(), done: 1, total: 0 };
        assert_eq!(empty.fraction(), 0.0);
    }

    #[test]
    fn every_step_state_has_a_distinct_label() {
        let labels = [
            StepState::Completed.label(),
            StepState::InProgress.label(),
            StepState::Pending.label(),
            StepState::Failed.label(),
        ];
        let unique: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len());
    }

    #[test]
    fn a_long_conversation_holds_exactly_what_was_asked_for() {
        assert_eq!(Conversation::long(10_000).messages.len(), 10_000);
    }

    #[test]
    fn streaming_appends_to_the_last_agent_message_only() {
        let mut c = Conversation::empty();
        c.ensure_streaming_agent().body.push_str("Hel");
        c.ensure_streaming_agent().body.push_str("lo");
        // A user message closes the stream; the next delta opens a new one.
        c.messages.push(Message::plain(Role::User, "You", "", "hi"));
        c.ensure_streaming_agent().body.push_str("!");
        assert_eq!(c.messages[0].body, "Hello");
        assert_eq!(c.messages.last().unwrap().body, "!");
    }

    #[test]
    fn finishing_a_step_updates_state_honestly() {
        let mut c = Conversation::empty();
        c.ensure_streaming_agent();
        c.finish_step("fs_read", true, "ok");
        // No step was started, so nothing appears out of nowhere.
        assert!(c.messages.iter().all(|m| m.plan.is_empty()));
    }
}