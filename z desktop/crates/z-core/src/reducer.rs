//! Reducer views over the journal (jour-006 fold API, jour-007 threads view,
//! jour-008/orch-001 task view + store, jour-009 usage counters).
//!
//! ADR-0012: task records exist only as journal events folded by a reducer —
//! there is no tasks.json. Views are pure folds over [`Journal::replay`];
//! writes are append-only events. Unknown future kinds deserialize into
//! [`JournalKind::Other`] and are simply ignored by every view here.

use crate::journal::{first_seq_gap, Journal, JournalKind, Record, RecordDraft};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Replays the journal segment at `path` and folds its records in order.
///
/// jour-010: a sequence gap (lost/rotated-away middle lines) is survivable
/// for views — the fold warns via `log` and keeps folding. Fail-loud stays
/// in [`Journal::replay`]'s malformed-line path only.
pub fn fold<State, F: FnMut(&mut State, &Record)>(
    path: &Path,
    mut init: State,
    mut f: F,
) -> Result<State, String> {
    let records = Journal::replay(path)?;
    if let Some((index, expected)) = first_seq_gap(&records) {
        log::warn!(
            "journal {}: gap at record {} (expected seq {expected}, got {})",
            path.display(),
            index + 1,
            records[index].seq
        );
    }
    for record in &records {
        f(&mut init, record);
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

/// jour-009: lifetime usage counters folded from journal events.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UsageView {
    pub turns_started: u64,
    pub commands_total: u64,
    pub messages_persisted: u64,
    pub provider_errors: u64,
}

/// Folds a journal segment into [`UsageView`], counting TurnStarted,
/// CommandReceived, MessagePersisted, and ProviderError records.
pub fn usage_fold(path: &Path) -> Result<UsageView, String> {
    crate::reducer::fold(path, UsageView::default(), |view, record| {
        match record.kind {
            JournalKind::TurnStarted => view.turns_started += 1,
            JournalKind::CommandReceived => view.commands_total += 1,
            JournalKind::MessagePersisted => view.messages_persisted += 1,
            JournalKind::ProviderError => view.provider_errors += 1,
            _ => {}
        }
    })
}

/// jour-009 extension: turns started per UTC day, ascending by day.
///
/// Buckets are derived from each TurnStarted record's top-level `ts_ms`
/// (wall-clock milliseconds, stamped by the journal writer).
pub fn usage_by_day(path: &Path) -> Result<Vec<(String, u64)>, String> {
    let mut days: BTreeMap<String, u64> = BTreeMap::new();
    crate::reducer::fold(path, (), |(), record| {
        if record.kind == JournalKind::TurnStarted {
            *days.entry(utc_day(record.ts_ms)).or_insert(0) += 1;
        }
    })?;
    Ok(days.into_iter().collect())
}

/// jour-016: counts journal records whose payload contains no secret-shaped
/// substrings. A record passes when [`crate::redact::redact`] applied to its
/// serialized payload leaves it byte-identical. This is only a summary
/// counter over one segment; the full redaction audit of every persisted
/// surface is red-005.
pub fn redacted_summary(path: &Path) -> Result<usize, String> {
    Ok(Journal::replay(path)?
        .iter()
        .filter(|record| {
            let text = record.payload.to_string();
            crate::redact::redact(&text) == text
        })
        .count())
}

/// UTC calendar day ("YYYY-MM-DD") of a wall-clock millisecond timestamp.
/// Days-since-epoch → civil date (Howard Hinnant's algorithm); no chrono dep.
fn utc_day(ts_ms: u128) -> String {
    let z = (ts_ms / 86_400_000) as i64 + 719_468;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + 400 * (z / 146_097) + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// orch-001: a task record and its lifecycle status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub status: TaskStatus,
    /// orch-002: declared dependency ids. A Pending task is ready only when
    /// every dependency is Done; unknown dep ids block forever (safe default).
    #[serde(default)]
    pub deps: Vec<String>,
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

/// Segment name used by [`TaskStore`] and orch-024 recovery (`runtime.rs`).
pub(crate) const TASKS_SEGMENT: &str = "tasks";

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
            // orch-002: `deps` is declared once at creation and absent from
            // later transition events — only overwrite it when carried.
            let deps = match record.payload.get("deps") {
                Some(v) => match serde_json::from_value::<Vec<String>>(v.clone()) {
                    Ok(deps) => Some(deps),
                    Err(e) => {
                        bad_payload = Some(format!(
                            "reducer {}: bad task deps payload in seq {}: {e}",
                            path.display(),
                            record.seq
                        ));
                        return;
                    }
                },
                None => None,
            };
            match parsed {
                Ok(status) => {
                    if let Some(id) = record.payload["id"].as_str() {
                        match view.tasks.get_mut(id) {
                            Some(existing) => {
                                existing.status = status;
                                if let Some(deps) = deps {
                                    existing.deps = deps;
                                }
                            }
                            None => {
                                view.tasks.insert(
                                    id.to_string(),
                                    TaskRecord {
                                        id: id.to_string(),
                                        status,
                                        deps: deps.unwrap_or_default(),
                                    },
                                );
                            }
                        }
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

    /// orch-002: tasks eligible to run — Pending with every declared
    /// dependency already Done. Unknown dep ids block forever (safe default).
    /// Sorted for deterministic order.
    pub fn ready_set(&self) -> Vec<String> {
        let mut ready: Vec<String> = self
            .tasks
            .values()
            .filter(|t| t.status == TaskStatus::Pending)
            .filter(|t| {
                t.deps
                    .iter()
                    .all(|d| self.tasks.get(d).map_or(false, |dep| dep.status == TaskStatus::Done))
            })
            .map(|t| t.id.clone())
            .collect();
        ready.sort();
        ready
    }
}

/// ctx-012: jour-012 double-replay determinism as a runtime-checkable helper —
/// folds the segment into [`TasksView`] twice (two fully separate replays of
/// the same bytes) and reports whether the views are deeply equal.
pub fn fold_twice_equal(path: &Path) -> Result<bool, String> {
    Ok(TasksView::fold(path)? == TasksView::fold(path)?)
}

/// ctx-012: counts tasks by status as (done, running, failed, pending).
pub fn task_counts(view: &TasksView) -> (usize, usize, usize, usize) {
    let mut counts = (0, 0, 0, 0);
    for task in view.tasks.values() {
        match task.status {
            TaskStatus::Done => counts.0 += 1,
            TaskStatus::Running => counts.1 += 1,
            TaskStatus::Failed => counts.2 += 1,
            TaskStatus::Pending => counts.3 += 1,
        }
    }
    counts
}

/// orch-001: appends `task_state_changed` journal events; state is read back
/// with [`TasksView::fold`]. Stateless by design — each call re-observes the
/// tail sequence so separate calls keep one continuous seq stream (single-
/// owner-thread concurrency contract, same as `Journal`).
pub struct TaskStore;

impl TaskStore {
    /// Records task creation (`status = Pending`).
    pub fn create(dir: &Path, id: &str) -> Result<(), String> {
        Self::create_with_deps(dir, id, &[])
    }

    /// orch-002: records task creation with declared dependency ids.
    pub fn create_with_deps(dir: &Path, id: &str, deps: &[String]) -> Result<(), String> {
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
            serde_json::json!({ "id": id, "status": TaskStatus::Pending, "deps": deps }),
        ))?;
        Ok(())
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
pub(crate) mod reducer_tests {
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
                status: TaskStatus::Done,
                deps: vec![]
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

    // jour-010 ---------------------------------------------------------------

    pub(crate) static WARN_LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    /// Serializes every test that asserts on WARN_LOG (shared with
    /// runtime::sup_e2e_tests) so parallel clears can't erase a peer's lines.
    pub(crate) static LOG_SECTION: std::sync::Mutex<()> = std::sync::Mutex::new(());
    pub(crate) static WARN_LOGGER_INIT: std::sync::Once = std::sync::Once::new();

    /// Captures `warn!` messages so tests can assert jour-010's warn-through.
    pub(crate) struct WarnCapture;
    impl log::Log for WarnCapture {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Warn
        }
        fn log(&self, record: &log::Record) {
            if record.level() == log::Level::Warn {
                WARN_LOG
                    .lock()
                    .expect("warn log lock")
                    .push(record.args().to_string());
            }
        }
        fn flush(&self) {}
    }

    #[test]
    fn fold_on_gapped_journal_warns_and_still_produces_view() {
        let _section = LOG_SECTION.lock().expect("log section lock");
        WARN_LOGGER_INIT.call_once(|| {
            let _ = log::set_boxed_logger(Box::new(WarnCapture));
            log::set_max_level(log::LevelFilter::Warn);
        });
        WARN_LOG.lock().expect("warn log lock").clear();

        let dir = temp_dir("gap-fold");
        {
            let mut j = Journal::open(&dir, TASKS_SEGMENT).expect("open");
            for id in ["a", "b", "c", "d"] {
                append(
                    &mut j,
                    JournalKind::TaskStateChanged,
                    None,
                    json!({"id": id, "status": "done"}),
                );
            }
        }
        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        let lines: Vec<String> = std::fs::read_to_string(&path)
            .expect("read journal")
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(lines.len(), 4);
        // Drop the middle line (seq 3): seqs on disk become [1, 2, 4].
        let gapped = format!("{}\n", [0, 1, 3].map(|i| lines[i].as_str()).join("\n"));
        std::fs::write(&path, gapped).expect("rewrite gapped journal");

        let view = TasksView::fold(&path).expect("gaps are survivable for views");
        assert_eq!(view.tasks.len(), 3, "seq-3 record (task c) is gone");
        assert!(view.tasks.contains_key("d"), "later records still fold");
        let warns = WARN_LOG.lock().expect("warn log lock");
        assert!(
            warns
                .iter()
                .any(|m| m.contains("gap at record 3") && m.contains("expected seq 3")),
            "must warn about the gap at record 3, got: {warns:?}"
        );
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

    // orch-002 ---------------------------------------------------------------

    #[test]
    fn ready_set_chain_only_dependency_free_then_unblocked_by_done() {
        let dir = temp_dir("ready-chain");
        // B depends on A; A has no deps.
        TaskStore::create_with_deps(&dir, "b", &["a".into()]).expect("create b");
        TaskStore::create(&dir, "a").expect("create a");
        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));

        let view = TasksView::fold(&path).expect("fold");
        assert_eq!(view.ready_set(), vec!["a"], "initially only A is ready");

        TaskStore::transition(&dir, "a", TaskStatus::Done).expect("a done");
        let view = TasksView::fold(&path).expect("fold");
        assert_eq!(view.ready_set(), vec!["b"], "A Done unblocks B");

        // A later dep-less transition of B must NOT erase its declared deps.
        TaskStore::transition(&dir, "b", TaskStatus::Running).expect("b running");
        let view = TasksView::fold(&path).expect("fold");
        assert_eq!(view.tasks["b"].deps, vec!["a"]);
        assert!(view.ready_set().is_empty(), "running B is not ready");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_or_unknown_dependencies_block_forever() {
        let dir = temp_dir("ready-blocked");
        TaskStore::create_with_deps(&dir, "d", &["f".into()]).expect("create d");
        TaskStore::create_with_deps(&dir, "u", &["ghost".into()]).expect("create u");
        TaskStore::create_with_deps(&dir, "m", &["f".into(), "ghost".into()])
            .expect("create m multi-dep");
        TaskStore::create(&dir, "f").expect("create f");
        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));

        let view = TasksView::fold(&path).expect("fold");
        assert_eq!(view.ready_set(), vec!["f"]);

        // F Failed: d and m never become ready; u's dep does not exist at all.
        TaskStore::transition(&dir, "f", TaskStatus::Failed).expect("f failed");
        let view = TasksView::fold(&path).expect("fold");
        assert!(
            view.ready_set().is_empty(),
            "failed/unknown deps must block forever: {:?}",
            view.ready_set()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-012 ---------------------------------------------------------------
    //
    // Deterministic double-replay equality (ADR-0004): every view fold routes
    // through fold() -> Journal::replay, so two View::fold calls are two fully
    // separate replays of the same bytes; their folded views must be deeply
    // equal, regardless of fold/construction order or prior process state.

    #[test]
    fn jour012_double_replay_threads_view_deeply_equal() {
        let dir = temp_dir("jour012-threads");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "title here"}),
            );
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::TurnFinished, Some("t1"), json!({}));
            append(&mut j, JournalKind::TurnStarted, Some("t2"), json!({}));
        }
        let path = dir.join("main.jsonl");
        let first = ThreadsView::fold(&path).expect("replay 1");
        let second = ThreadsView::fold(&path).expect("replay 2");
        assert_eq!(first, second, "two separate replays must fold identically");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jour012_double_replay_tasks_view_deeply_equal() {
        let dir = temp_dir("jour012-tasks");
        TaskStore::create(&dir, "task-a").expect("create");
        TaskStore::transition(&dir, "task-a", TaskStatus::Running).expect("running");
        TaskStore::create(&dir, "task-b").expect("create b");
        TaskStore::transition(&dir, "task-a", TaskStatus::Done).expect("done");
        TaskStore::transition(&dir, "task-b", TaskStatus::Failed).expect("failed");

        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        let first = TasksView::fold(&path).expect("replay 1");
        let second = TasksView::fold(&path).expect("replay 2");
        assert_eq!(first, second, "two separate replays must fold identically");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jour012_double_replay_tasks_view_with_deps_deeply_equal() {
        let dir = temp_dir("jour012-deps");
        TaskStore::create_with_deps(&dir, "b", &["a".into(), "ghost".into()]).expect("create b");
        TaskStore::create(&dir, "a").expect("create a");
        TaskStore::transition(&dir, "a", TaskStatus::Done).expect("a done");
        TaskStore::transition(&dir, "b", TaskStatus::Pending).expect("b pending keeps deps");

        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        let first = TasksView::fold(&path).expect("replay 1");
        let second = TasksView::fold(&path).expect("replay 2");
        assert_eq!(first, second, "two separate replays must fold identically");
        assert_eq!(second.tasks["b"].deps, vec!["a", "ghost"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jour012_reverse_construction_order_yields_identical_views() {
        let dir = temp_dir("jour012-reverse");
        {
            let mut j = Journal::open(&dir, TASKS_SEGMENT).expect("open");
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "x", "status": "pending", "deps": ["y"]}),
            );
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "hello"}),
            );
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "y", "status": "done"}),
            );
        }
        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        // Forward construction order ...
        let tasks_forward = TasksView::fold(&path).expect("tasks fold");
        let threads_forward = ThreadsView::fold(&path).expect("threads fold");
        // ... then the exact reverse order, fresh folds over the same events.
        let threads_reverse = ThreadsView::fold(&path).expect("threads fold");
        let tasks_reverse = TasksView::fold(&path).expect("tasks fold");

        assert_eq!(tasks_forward, tasks_reverse);
        assert_eq!(threads_forward, threads_reverse);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ctx-012 ---------------------------------------------------------------

    #[test]
    fn task_counts_seeded_journal_matches_lifecycle_statuses() {
        let dir = temp_dir("ctx012-counts");
        TaskStore::create(&dir, "d").expect("create d");
        TaskStore::create(&dir, "r").expect("create r");
        TaskStore::create(&dir, "f").expect("create f");
        TaskStore::create(&dir, "p").expect("create p");
        TaskStore::transition(&dir, "d", TaskStatus::Done).expect("d done");
        TaskStore::transition(&dir, "r", TaskStatus::Running).expect("r running");
        TaskStore::transition(&dir, "f", TaskStatus::Failed).expect("f failed");

        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        assert_eq!(
            task_counts(&view),
            (1, 1, 1, 1),
            "one done, one running, one failed, one pending"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fold_twice_equal_on_seeded_journal_returns_true() {
        let dir = temp_dir("ctx012-twice");
        TaskStore::create_with_deps(&dir, "b", &["a".into()]).expect("create b");
        TaskStore::create(&dir, "a").expect("create a");
        TaskStore::transition(&dir, "a", TaskStatus::Done).expect("a done");
        TaskStore::transition(&dir, "b", TaskStatus::Running).expect("b running");

        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        assert!(
            fold_twice_equal(&path).expect("fold twice"),
            "two separate replays of the same bytes must fold identically"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_journal_task_counts_are_zeroed_and_fold_twice_holds() {
        let dir = temp_dir("ctx012-empty");
        {
            // Create the file with no records (drop flushes/closes it).
            Journal::open(&dir, TASKS_SEGMENT).expect("open");
        }
        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        let view = TasksView::fold(&path).expect("fold");
        assert_eq!(task_counts(&view), (0, 0, 0, 0));
        assert!(fold_twice_equal(&path).expect("fold twice"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-009 ---------------------------------------------------------------

    #[test]
    fn usage_fold_counts_known_kinds_exactly() {
        let dir = temp_dir("usage-counts");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::CommandReceived, Some("t1"), json!({}));
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "hi"}),
            );
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::ProviderError,
                None,
                json!({"attempt": 1}),
            );
            // Second turn: command + turn again.
            append(&mut j, JournalKind::CommandReceived, Some("t1"), json!({}));
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
        }
        let view = usage_fold(&dir.join("main.jsonl")).expect("fold");
        assert_eq!(
            view,
            UsageView {
                turns_started: 2,
                commands_total: 2,
                messages_persisted: 2,
                provider_errors: 1,
            }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_fold_on_empty_journal_is_zeroed() {
        let dir = temp_dir("usage-empty");
        {
            // Create the file with no records (drop flushes/closes it).
            Journal::open(&dir, "main").expect("open");
        }
        let view = usage_fold(&dir.join("main.jsonl")).expect("fold");
        assert_eq!(view, UsageView::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_fold_ignores_unknown_and_other_kinds() {
        let dir = temp_dir("usage-unknown");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(
                &mut j,
                JournalKind::Other("brand_new_future_kind".into()),
                Some("t1"),
                json!({}),
            );
            append(&mut j, JournalKind::TurnFinished, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "x"}),
            );
        }
        let view = usage_fold(&dir.join("main.jsonl")).expect("fold");
        assert_eq!(
            view,
            UsageView::default(),
            "only the four counted kinds move counters"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-009 extension: usage_by_day ---------------------------------------

    const DAY_A_MS: u128 = 1_704_067_200_000; // 2024-01-01T00:00:00Z
    const DAY_B_MS: u128 = 1_704_153_600_000; // 2024-01-02T00:00:00Z

    /// Writes a record with a controlled ts_ms straight into the segment
    /// file — `Journal::append` stamps wall-clock now, which tests can't set.
    fn seed_record_at(dir: &Path, seq: u64, ts_ms: u128, kind: JournalKind) {
        use std::io::Write as _;
        let record = Record {
            seq,
            ts_ms,
            kind,
            thread_id: Some("t1".into()),
            payload: json!({}),
        };
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("main.jsonl"))
            .expect("open segment");
        writeln!(f, "{}", serde_json::to_string(&record).expect("serialize")).expect("write");
    }

    #[test]
    fn usage_by_day_buckets_two_days_in_order() {
        let dir = temp_dir("usage-by-day-two");
        seed_record_at(&dir, 1, DAY_A_MS, JournalKind::TurnStarted);
        seed_record_at(&dir, 2, DAY_A_MS, JournalKind::TurnStarted);
        seed_record_at(&dir, 3, DAY_B_MS, JournalKind::TurnStarted);
        assert_eq!(
            usage_by_day(&dir.join("main.jsonl")).expect("fold"),
            vec![
                ("2024-01-01".to_string(), 2),
                ("2024-01-02".to_string(), 1),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_by_day_on_empty_journal_is_empty_vec() {
        let dir = temp_dir("usage-by-day-empty");
        {
            // Create the file with no records (drop flushes/closes it).
            Journal::open(&dir, "main").expect("open");
        }
        let days = usage_by_day(&dir.join("main.jsonl")).expect("fold");
        assert!(days.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_by_day_groups_single_day_and_counts_only_turns() {
        let dir = temp_dir("usage-by-day-single");
        seed_record_at(&dir, 1, DAY_A_MS, JournalKind::TurnStarted);
        seed_record_at(
            &dir,
            2,
            DAY_A_MS + 86_399_999, // last millisecond of the same UTC day
            JournalKind::TurnStarted,
        );
        // Non-turn kinds never land in buckets.
        seed_record_at(&dir, 3, DAY_B_MS, JournalKind::CommandReceived);
        assert_eq!(
            usage_by_day(&dir.join("main.jsonl")).expect("fold"),
            vec![("2024-01-01".to_string(), 2)]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-016 ---------------------------------------------------------------

    #[test]
    fn redacted_summary_counts_only_secret_free_records() {
        let dir = temp_dir("redacted-summary");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "hello world"}),
            );
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "key sk-abcdefghijklmnopqrstuvwx"}),
            );
            append(&mut j, JournalKind::CommandReceived, None, json!({}));
        }
        let count = redacted_summary(&dir.join("main.jsonl")).expect("summary");
        assert_eq!(count, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn redacted_summary_on_empty_journal_is_zero() {
        let dir = temp_dir("redacted-empty");
        {
            // Create the file with no records (drop flushes/closes it).
            Journal::open(&dir, "main").expect("open");
        }
        assert_eq!(
            redacted_summary(&dir.join("main.jsonl")).expect("summary"),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
