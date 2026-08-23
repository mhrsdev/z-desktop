//! Reducer views over the journal (jour-006 fold API, jour-007 threads view,
//! jour-008/orch-001 task view + store).
//!
//! ADR-0012: task records exist only as journal events folded by a reducer —
//! there is no tasks.json. Views are pure folds over [`Journal::replay`];
//! writes are append-only events. Unknown future kinds deserialize into
//! [`JournalKind::Other`] and are simply ignored by every view here.

use crate::journal::{Journal, JournalKind, Record, RecordDraft};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Replays the journal segment at `path` and folds its records in order.
pub fn fold<State, F: FnMut(&mut State, &Record)>(
    path: &Path,
    mut init: State,
    mut f: F,
) -> Result<State, String> {
    for record in Journal::replay(path)? {
        f(&mut init, &record);
    }
    Ok(init)
}

/// jour-007: per-thread rollup of message counts and latest activity.
#[derive(Debug, Default, PartialEq)]
pub struct ThreadsView {
    pub threads: HashMap<String, ThreadSummary>,
}

#[derive(Debug, PartialEq)]
pub struct ThreadSummary {
    /// `payload.text` of the thread's first persisted message ("" if none yet).
    pub title_first_msg: String,
    pub message_count: u64,
    pub last_kind: JournalKind,
}

impl ThreadsView {
    pub fn fold(path: &Path) -> Result<ThreadsView, String> {
        let mut view = ThreadsView::default();
        crate::reducer::fold(path, (), |(), record| view.apply(record))?;
        Ok(view)
    }

    fn apply(&mut self, record: &Record) {
        if !matches!(
            record.kind,
            JournalKind::MessagePersisted | JournalKind::TurnStarted
        ) {
            return;
        }
        let Some(thread_id) = record.thread_id.clone() else {
            return;
        };
        let entry = self
            .threads
            .entry(thread_id)
            .or_insert_with(|| ThreadSummary {
                title_first_msg: String::new(),
                message_count: 0,
                last_kind: record.kind.clone(),
            });
        if record.kind == JournalKind::MessagePersisted {
            if entry.message_count == 0 && entry.title_first_msg.is_empty() {
                entry.title_first_msg = record
                    .payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
            }
            entry.message_count += 1;
        }
        entry.last_kind = record.kind.clone();
    }
}

/// orch-001: a task record and its lifecycle status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
}

/// Folded state of all tasks seen in a journal segment.
#[derive(Debug, Default, PartialEq)]
pub struct TasksView {
    pub tasks: HashMap<String, TaskRecord>,
}

/// Segment name used by [`TaskStore`].
const TASKS_SEGMENT: &str = "tasks";

impl TasksView {
    /// Folds `task_state_changed` events into current task states. A corrupt
    /// status payload fails loud (same policy as replay on malformed lines).
    pub fn fold(path: &Path) -> Result<TasksView, String> {
        let mut bad_payload: Option<String> = None;
        let view = crate::reducer::fold(path, TasksView::default(), |view, record| {
            if record.kind != JournalKind::TaskStateChanged || bad_payload.is_some() {
                return;
            }
            let parsed =
                serde_json::from_value::<TaskStatus>(record.payload["status"].clone()).map_err(
                    |e| {
                        format!(
                            "reducer {}: bad task status payload in seq {}: {e}",
                            path.display(),
                            record.seq
                        )
                    },
                );
            match parsed {
                Ok(status) => {
                    if let Some(id) = record.payload["id"].as_str() {
                        view.tasks.insert(
                            id.to_string(),
                            TaskRecord {
                                id: id.to_string(),
                                status,
                            },
                        );
                    }
                }
                Err(e) => bad_payload = Some(e),
            }
        })?;
        match bad_payload {
            Some(e) => Err(e),
            None => Ok(view),
        }
    }
}

/// orch-001: appends `task_state_changed` journal events; state is read back
/// with [`TasksView::fold`]. Stateless by design — each call re-observes the
/// tail sequence so separate calls keep one continuous seq stream (single-
/// owner-thread concurrency contract, same as `Journal`).
pub struct TaskStore;

impl TaskStore {
    /// Records task creation (`status = Pending`).
    pub fn create(dir: &Path, id: &str) -> Result<(), String> {
        Self::transition(dir, id, TaskStatus::Pending)
    }

    /// Appends one transition event for `id`.
    pub fn transition(dir: &Path, id: &str, status: TaskStatus) -> Result<(), String> {
        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        let last_seq = if path.exists() {
            Journal::replay(&path)?.last().map(|r| r.seq).unwrap_or(0)
        } else {
            0
        };
        let mut journal = if last_seq == 0 {
            Journal::open(dir, TASKS_SEGMENT)?
        } else {
            Journal::open_resuming(dir, TASKS_SEGMENT, last_seq)?
        };
        journal.append(RecordDraft::new(
            JournalKind::TaskStateChanged,
            None,
            serde_json::json!({ "id": id, "status": status }),
        ))?;
        Ok(())
    }
}

#[cfg(test)]
mod reducer_tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "z-reducer-test-{}-{tag}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn append(journal: &mut Journal, kind: JournalKind, thread: Option<&str>, payload: Value) {
        journal
            .append(RecordDraft::new(kind, thread.map(str::to_string), payload))
            .expect("append");
    }

    #[test]
    fn threads_view_counts_messages_and_tracks_titles_and_last_kind() {
        let dir = temp_dir("threads");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::CommandReceived, Some("t1"), json!({}));
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "hello world"}),
            );
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::TurnFinished, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::TurnStarted,
                Some("t2"),
                json!({"model": "local"}),
            );
        }
        let view = ThreadsView::fold(&dir.join("main.jsonl")).expect("fold");
        assert_eq!(view.threads.len(), 2);

        let t1 = &view.threads["t1"];
        assert_eq!(t1.title_first_msg, "hello world");
        assert_eq!(t1.message_count, 2);
        assert_eq!(t1.last_kind, JournalKind::MessagePersisted); // TurnFinished ignored

        let t2 = &view.threads["t2"];
        assert_eq!(t2.title_first_msg, "");
        assert_eq!(t2.message_count, 0);
        assert_eq!(t2.last_kind, JournalKind::TurnStarted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_lifecycle_create_running_done_folds_to_final_status() {
        let dir = temp_dir("lifecycle");
        TaskStore::create(&dir, "task-a").expect("create");
        TaskStore::transition(&dir, "task-a", TaskStatus::Running).expect("running");
        TaskStore::transition(&dir, "task-a", TaskStatus::Done).expect("done");

        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        let view = TasksView::fold(&path).expect("fold");
        assert_eq!(view.tasks.len(), 1);
        assert_eq!(
            view.tasks["task-a"],
            TaskRecord {
                id: "task-a".into(),
                status: TaskStatus::Done
            }
        );

        // Seq continuity across the three separate store calls.
        let records = Journal::replay(&path).expect("replay");
        assert_eq!(
            records.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn interleaved_tasks_fold_independently() {
        let dir = temp_dir("interleaved");
        TaskStore::create(&dir, "a").expect("create a");
        TaskStore::create(&dir, "b").expect("create b");
        TaskStore::transition(&dir, "a", TaskStatus::Running).expect("a running");
        TaskStore::transition(&dir, "b", TaskStatus::Failed).expect("b failed");

        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        assert_eq!(view.tasks["a"].status, TaskStatus::Running);
        assert_eq!(view.tasks["b"].status, TaskStatus::Failed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_kinds_and_irrelevant_records_do_not_break_folds() {
        let dir = temp_dir("unknown");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(
                &mut j,
                JournalKind::Other("workflow_step_started".into()),
                None,
                json!({"step": 1}),
            );
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "x", "status": "pending"}),
            );
            append(
                &mut j,
                JournalKind::Other("brand_new_future_kind".into()),
                Some("t1"),
                json!({}),
            );
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "survives"}),
            );
        }
        let threads = ThreadsView::fold(&dir.join("main.jsonl")).expect("threads fold");
        assert_eq!(threads.threads["t1"].message_count, 1);
        let tasks = TasksView::fold(&dir.join("main.jsonl")).expect("tasks fold");
        assert_eq!(tasks.tasks["x"].status, TaskStatus::Pending);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_task_status_fails_loud() {
        let dir = temp_dir("corrupt-status");
        {
            let mut j = Journal::open(&dir, TASKS_SEGMENT).expect("open");
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "x", "status": "not_a_status"}),
            );
        }
        let err = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl")))
            .expect_err("bad status must fail loud");
        assert!(err.contains("bad task status payload"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
