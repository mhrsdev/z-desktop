//! JSONL task journal — the durable, append-only source of truth
//! (jour-001 record schema + writer skeleton, jour-002 O_APPEND + fsync
//! policy, jour-005 replay engine core).
//!
//! Design rules (Master Spec §18, JSONL-first persistence):
//! - One JSON object per line, newline-delimited UTF-8. Lines are compact
//!   (`serde_json::to_string`), so a record can never contain a raw newline.
//! - Writes are append-only: corrections are new events, never edits.
//! - The writer owns its file handle, opened with `OpenOptions::append(true)`
//!   (O_APPEND): every `write` lands atomically at the current end of file.
//! - Replay reads segments front-to-back and fails loud on malformed lines;
//!   the single sanctioned exception is jour-011's torn-tail auto-repair (see
//!   [`Journal::replay`]). Silent corruption tolerance never.
//! - Every record carries a monotonically increasing `seq`, so rotation
//!   (jour-003), checksums (jour-004), and gap detection (jour-010) can be
//!   layered on later without changing the wire format.

use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Sync policy default: force `fsync` once every N appended records.
///
/// Why batch at all? `flush()` per record pushes bytes to the OS page cache,
/// which already survives process/app crashes. Only `sync_all()` survives a
/// power loss / kernel panic, and forcing it per record costs a disk round
/// trip per event. Batching bounds the worst-case loss to the last N-1
/// records while keeping steady-state cost near zero; callers can shrink the
/// window (`open_with_policy`) or force durability at checkpoints via
/// [`Journal::flush_and_sync`] (e.g. turn boundaries).
pub const DEFAULT_FSYNC_EVERY: u32 = 32;

/// Lifecycle kinds the core journals today. Unknown future kinds must never
/// break replay of an older binary, so deserialization falls back to
/// [`JournalKind::Other`] instead of failing — additive evolution by design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalKind {
    /// A command arrived from the client (start of the turn pipeline).
    CommandReceived,
    /// Agent turn began executing.
    TurnStarted,
    /// Agent turn finished (success or failure recorded in the payload).
    TurnFinished,
    /// An assistant/user message was persisted to its thread snapshot.
    MessagePersisted,
    /// A task record changed status (orch-001; folded by the TasksView reducer).
    TaskStateChanged,
    /// A supervision evidence record was captured (sup-001/sup-002, ADR-0016;
    /// folded by the EvidenceView reducer).
    EvidenceRecorded,
    /// A memory record was written (mem-001, ADR-0014; folded by the
    /// MemoryView reducer into the per-layer JSONL views).
    MemoryRecorded,
    /// A thread was deleted (core-023 tombstone; shape-only payload, the
    /// snapshot file itself is gone — replay/reducers use this to exclude it).
    ThreadDeleted,
    /// A provider call failed (core-019, ADR-0017 D5). One breadcrumb per
    /// failed attempt; payload carries `attempt` (1 = initial call,
    /// 2 = the single retry) and the retry `class`.
    ProviderError,
    /// A user appeal overrode a supervision verdict (sup-017/024); folded
    /// back into the gate's override set at startup by the runtime loader.
    VerdictOverridden,
    /// Escape hatch for kinds this build does not know yet.
    Other(String),
}

impl JournalKind {
    fn as_str(&self) -> &str {
        match self {
            JournalKind::CommandReceived => "command_received",
            JournalKind::TurnStarted => "turn_started",
            JournalKind::TurnFinished => "turn_finished",
            JournalKind::MessagePersisted => "message_persisted",
            JournalKind::TaskStateChanged => "task_state_changed",
            JournalKind::EvidenceRecorded => "evidence_recorded",
            JournalKind::MemoryRecorded => "memory_recorded",
            JournalKind::ThreadDeleted => "thread_deleted",
            JournalKind::ProviderError => "provider_error",
            JournalKind::VerdictOverridden => "verdict_overridden",
            JournalKind::Other(s) => s.as_str(),
        }
    }

    fn from_str_lossy(s: String) -> Self {
        match s.as_str() {
            "command_received" => JournalKind::CommandReceived,
            "turn_started" => JournalKind::TurnStarted,
            "turn_finished" => JournalKind::TurnFinished,
            "message_persisted" => JournalKind::MessagePersisted,
            "task_state_changed" => JournalKind::TaskStateChanged,
            "evidence_recorded" => JournalKind::EvidenceRecorded,
            "memory_recorded" => JournalKind::MemoryRecorded,
            "thread_deleted" => JournalKind::ThreadDeleted,
            "provider_error" => JournalKind::ProviderError,
            "verdict_overridden" => JournalKind::VerdictOverridden,
            _ => JournalKind::Other(s),
        }
    }
}

impl Serialize for JournalKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for JournalKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(JournalKind::from_str_lossy(String::deserialize(
            deserializer,
        )?))
    }
}

/// One journal event: exactly one JSON object on one line on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Monotonically increasing per journal instance, starting at 1. Gaps in
    /// this sequence are how rotation/truncation damage is detected later
    /// (see [`first_seq_gap`], consumed by jour-010).
    pub seq: u64,
    /// Wall-clock milliseconds since the Unix epoch, stamped by the writer.
    pub ts_ms: u128,
    pub kind: JournalKind,
    pub thread_id: Option<String>,
    /// Free-form, kind-specific data. Kept as `Value` so new fields can be
    /// added without breaking old readers.
    pub payload: serde_json::Value,
}

/// What a caller hands to [`Journal::append`] before the journal stamps
/// `seq` and `ts_ms`. Any pre-set values on those fields would be ignored —
/// which is why they are not part of the draft type at all.
#[derive(Debug, Clone)]
pub struct RecordDraft {
    pub kind: JournalKind,
    pub thread_id: Option<String>,
    pub payload: serde_json::Value,
}

impl RecordDraft {
    pub fn new(kind: JournalKind, thread_id: Option<String>, payload: serde_json::Value) -> Self {
        RecordDraft {
            kind,
            thread_id,
            payload,
        }
    }
}

/// Append-only JSONL writer for a single `<dir>/<name>.jsonl` segment.
///
/// Concurrency contract: like everything in z-core this is blocking I/O meant
/// to live on one owner thread; the type is neither `Sync` nor cloned around.
pub struct Journal {
    file: std::io::BufWriter<std::fs::File>,
    path: PathBuf,
    next_seq: u64,
    fsync_every: u32,
    records_since_sync: u32,
}

impl Journal {
    /// Opens (creating `dir` and the file if missing) `<dir>/<name>.jsonl`
    /// with the default sync policy ([`DEFAULT_FSYNC_EVERY`]).
    ///
    /// A fresh instance starts assigning `seq` at 1; use [`Journal::open_resuming`]
    /// when reopening a segment whose last sequence number is known.
    pub fn open(dir: &Path, name: &str) -> Result<Journal, String> {
        Journal::open_full(dir, name, DEFAULT_FSYNC_EVERY, 1)
    }

    /// Like [`Journal::open`] but with an injectable fsync interval
    /// (`N`; `1` = sync every record, trading throughput for durability).
    pub fn open_with_policy(dir: &Path, name: &str, fsync_every: u32) -> Result<Journal, String> {
        Journal::open_full(dir, name, fsync_every, 1)
    }

    /// Reopens an existing journal, continuing the sequence after `last_seq`.
    /// Used when a previous writer instance is gone but its tail was observed
    /// (e.g. after restart); the file itself is opened in append mode either way.
    pub fn open_resuming(dir: &Path, name: &str, last_seq: u64) -> Result<Journal, String> {
        Journal::open_full(dir, name, DEFAULT_FSYNC_EVERY, last_seq.saturating_add(1))
    }

    fn open_full(
        dir: &Path,
        name: &str,
        fsync_every: u32,
        next_seq: u64,
    ) -> Result<Journal, String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("journal: cannot create directory {}: {}", dir.display(), e))?;
        let path = dir.join(format!("{name}.jsonl"));
        // O_APPEND (std maps `append(true)` to it on both Unix and Windows):
        // every write is positioned atomically at EOF, so a torn tail from a
        // crashed writer cannot overwrite earlier records.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("journal: cannot open {}: {}", path.display(), e))?;
        Ok(Journal {
            file: std::io::BufWriter::new(file),
            path,
            next_seq,
            fsync_every: fsync_every.max(1),
            records_since_sync: 0,
        })
    }

    /// Appends one record and returns the assigned sequence number.
    ///
    /// Durability policy: the line is `flush()`-ed to the OS on every call;
    /// `sync_all()` runs only every `fsync_every` records (and on
    /// [`Journal::flush_and_sync`]), bounding the power-loss window to the
    /// last N-1 records. See [`DEFAULT_FSYNC_EVERY`] for the reasoning.
    pub fn append(&mut self, draft: RecordDraft) -> Result<u64, String> {
        let seq = self.next_seq;
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let record = Record {
            seq,
            ts_ms,
            kind: draft.kind,
            thread_id: draft.thread_id,
            payload: draft.payload,
        };
        let mut line = serde_json::to_string(&record)
            .map_err(|e| format!("journal: record {seq} does not serialize: {e}"))?;
        line.push('\n');
        self.file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.flush())
            .map_err(|e| format!("journal: append to {} failed: {}", self.path.display(), e))?;

        self.records_since_sync += 1;
        if self.records_since_sync >= self.fsync_every {
            self.force_sync()?;
        }
        self.next_seq = seq
            .checked_add(1)
            .ok_or_else(|| "journal: sequence space exhausted".to_string())?;
        Ok(seq)
    }

    /// Flushes buffered bytes and forces an fsync; call at durability
    /// checkpoints (turn boundaries, before answering the user "it's saved").
    pub fn flush_and_sync(&mut self) -> Result<(), String> {
        self.file
            .flush()
            .map_err(|e| format!("journal: flush of {} failed: {}", self.path.display(), e))?;
        self.force_sync()
    }

    /// How many flushed records have not yet been fsync-ed (the current
    /// crash window). Observable so tests and callers can assert the policy.
    pub fn records_since_sync(&self) -> u32 {
        self.records_since_sync
    }

    /// Path of the backing `.jsonl` segment.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn force_sync(&mut self) -> Result<(), String> {
        self.file
            .get_ref()
            .sync_all()
            .map_err(|e| format!("journal: fsync of {} failed: {}", self.path.display(), e))?;
        self.records_since_sync = 0;
        Ok(())
    }

    /// Replays a journal segment in order: reads the file front-to-back,
    /// skips empty (typically trailing) lines, and parses every remaining
    /// line as a [`Record`].
    ///
    /// Malformed input fails loud with the offending line number — data is
    /// never silently skipped. The one sanctioned exception (jour-011): if
    /// the malformed line is the LAST one in the file it is treated as a
    /// torn final write from a crashed append; [`truncate_corrupt_tail`]
    /// runs once and replay retries. See that function for why this is safe.
    pub fn replay(path: &Path) -> Result<Vec<Record>, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("journal: cannot read {}: {}", path.display(), e))?;
        match replay_lines(&content, path) {
            Ok(records) => Ok(records),
            Err((line_no, message)) => {
                let last_line_no = content.lines().count();
                if line_no != last_line_no {
                    return Err(message);
                }
                if truncate_corrupt_tail(path)? > 0 {
                    let repaired = std::fs::read_to_string(path)
                        .map_err(|e| format!("journal: cannot read {}: {}", path.display(), e))?;
                    return replay_lines(&repaired, path).map_err(|(_, m)| m);
                }
                Err(message)
            }
        }
    }
}

/// One strict front-to-back parse pass over journal content. `Err` carries
/// the 1-based offending line number alongside the user-facing message so
/// [`Journal::replay`] can tell a torn tail from mid-file corruption.
fn replay_lines(content: &str, path: &Path) -> Result<Vec<Record>, (usize, String)> {
    let mut records = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let line_no = index + 1;
        let record: Record = serde_json::from_str(line).map_err(|e| {
            (
                line_no,
                format!(
                    "journal {}: line {} is malformed: {e}",
                    path.display(),
                    line_no
                ),
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

/// jour-011 corrupt-tail repair: rewrites `path` truncated to its last
/// complete newline when everything after that newline fails to parse as a
/// [`Record`]. Returns the number of bytes removed — `0` when there is no
/// tail at all or the tail already parses cleanly (nothing to repair).
///
/// Why safe (ADR-0004 posture): appends are O_APPEND, so a crash during
/// append can only leave garbage AFTER the last good record, never inside
/// earlier ones — dropping the tail loses nothing but the torn bytes.
/// Corruption in the MIDDLE of the segment is deliberately untouched here;
/// that stays replay's fail-loud domain rather than something a byte-level
/// heuristic guesses at.
pub fn truncate_corrupt_tail(path: &Path) -> Result<usize, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("journal: cannot read {}: {}", path.display(), e))?;
    match bytes.iter().rposition(|&b| b == b'\n') {
        // No newline: either an empty file or one lone torn line.
        None => {
            if bytes.is_empty() || serde_json::from_slice::<Record>(&bytes).is_ok() {
                return Ok(0);
            }
            std::fs::write(path, b"")
                .map_err(|e| format!("journal: cannot truncate {}: {}", path.display(), e))?;
            Ok(bytes.len())
        }
        Some(last_newline) => {
            let tail = &bytes[last_newline + 1..];
            if tail.is_empty() || serde_json::from_slice::<Record>(tail).is_ok() {
                return Ok(0);
            }
            let kept = &bytes[..=last_newline];
            std::fs::write(path, kept)
                .map_err(|e| format!("journal: cannot truncate {}: {}", path.display(), e))?;
            Ok(bytes.len() - kept.len())
        }
    }
}

/// Returns `(record_index, expected_seq)` for the first gap or regression in
/// `records` (jour-010 hook).
///
/// The first record's `seq` is the baseline (a rotated segment may legitimately
/// start above 1); afterwards each record must be exactly `previous + 1`.
/// Duplicates and backwards jumps violate that too and are reported the same
/// way, with `expected_seq` being what *should* have appeared at that index.
pub fn first_seq_gap(records: &[Record]) -> Option<(usize, u64)> {
    let mut expected = records.first()?.seq;
    for (index, record) in records.iter().enumerate().skip(1) {
        expected = expected.wrapping_add(1);
        if record.seq != expected {
            return Some((index, expected));
        }
    }
    None
}

#[cfg(test)]
mod journal_tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Process-unique scratch directory under the system temp dir.
    fn temp_journal_dir(tag: &str) -> PathBuf {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "z-journal-test-{}-{tag}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp journal dir");
        dir
    }

    fn mixed_drafts() -> Vec<RecordDraft> {
        vec![
            RecordDraft::new(
                JournalKind::CommandReceived,
                Some("thread-1".into()),
                json!({"command": "run_turn", "turn": 1}),
            ),
            RecordDraft::new(
                JournalKind::TurnStarted,
                Some("thread-1".into()),
                json!({"model": "local"}),
            ),
            RecordDraft::new(
                JournalKind::MessagePersisted,
                Some("thread-1".into()),
                json!({"role": "assistant", "chars": 42}),
            ),
            RecordDraft::new(
                JournalKind::TurnFinished,
                Some("thread-1".into()),
                json!({"ok": true, "rounds": 3}),
            ),
            RecordDraft::new(
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "t-9", "status": "done"}),
            ),
            RecordDraft::new(
                JournalKind::EvidenceRecorded,
                Some("thread-1".into()),
                json!({"id": "ev-1", "kind": "build", "ok": true}),
            ),
            RecordDraft::new(
                JournalKind::MemoryRecorded,
                Some("thread-1".into()),
                json!({"id": "mem-1", "content": "project uses pnpm"}),
            ),
            RecordDraft::new(
                JournalKind::ThreadDeleted,
                Some("thread-2".into()),
                json!({"thread_id": "thread-2"}),
            ),
        ]
    }

    #[test]
    fn round_trip_mixed_kinds_replays_identical_records_in_order() {
        let dir = temp_journal_dir("roundtrip");
        let drafts = mixed_drafts();
        let mut seqs = Vec::new();
        {
            let mut journal = Journal::open(&dir, "main").expect("open journal");
            for draft in &drafts {
                seqs.push(journal.append(draft.clone()).expect("append"));
            }
        }
        let records = Journal::replay(&dir.join("main.jsonl")).expect("replay");
        assert_eq!(records.len(), drafts.len());
        for (i, (record, draft)) in records.iter().zip(&drafts).enumerate() {
            assert_eq!(record.seq, seqs[i]);
            assert_eq!(record.kind, draft.kind);
            assert_eq!(record.thread_id, draft.thread_id);
            assert_eq!(record.payload, draft.payload);
            assert!(record.ts_ms > 0, "writer must stamp a timestamp");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn seq_starts_at_one_and_continues_after_reopen_with_known_last_seq() {
        let dir = temp_journal_dir("seq");
        let mut seen = Vec::new();
        {
            let mut journal = Journal::open(&dir, "main").expect("open journal");
            for _ in 0..3 {
                seen.push(
                    journal
                        .append(RecordDraft::new(JournalKind::TurnStarted, None, json!({})))
                        .expect("append"),
                );
            }
        } // writer dropped; file handle released
        {
            let mut journal =
                Journal::open_resuming(&dir, "main", *seen.last().expect("non-empty"))
                    .expect("reopen");
            assert_eq!(
                journal
                    .append(RecordDraft::new(JournalKind::TurnFinished, None, json!({})))
                    .expect("append"),
                4
            );
            assert_eq!(
                journal
                    .append(RecordDraft::new(JournalKind::TurnFinished, None, json!({})))
                    .expect("append"),
                5
            );
        }
        let seqs: Vec<u64> = Journal::replay(&dir.join("main.jsonl"))
            .expect("replay")
            .iter()
            .map(|r| r.seq)
            .collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_journal_replays_to_empty_vec() {
        let dir = temp_journal_dir("empty");
        {
            let _journal = Journal::open(&dir, "main").expect("open journal");
        }
        let records = Journal::replay(&dir.join("main.jsonl")).expect("replay");
        assert!(records.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_middle_line_fails_loud_with_line_number() {
        let dir = temp_journal_dir("malformed");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("broken.jsonl");
        let good = Record {
            seq: 1,
            ts_ms: 1_770_000_000_000,
            kind: JournalKind::CommandReceived,
            thread_id: None,
            payload: json!({"x": 1}),
        };
        std::fs::write(
            &path,
            format!(
                "{}\n{{\"seq\": not json at all\n{}\n",
                serde_json::to_string(&good).expect("serialize"),
                serde_json::to_string(&Record {
                    seq: 2,
                    ..good.clone()
                })
                .expect("serialize")
            ),
        )
        .expect("seed broken journal");
        let err = Journal::replay(&path).expect_err("must fail loud");
        assert!(
            err.contains("line 2"),
            "error should name line 2, got: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn burst_of_500_records_replays_correctly_and_quickly() {
        let dir = temp_journal_dir("burst");
        let started = std::time::Instant::now();
        let mut journal = Journal::open(&dir, "main").expect("open journal");
        for i in 0..500u64 {
            let seq = journal
                .append(RecordDraft::new(
                    if i % 2 == 0 {
                        JournalKind::TurnStarted
                    } else {
                        JournalKind::MessagePersisted
                    },
                    Some(format!("thread-{}", i % 7)),
                    json!({"i": i, "note": "burst payload"}),
                ))
                .expect("append");
            assert_eq!(seq, i + 1);
        }
        journal.flush_and_sync().expect("final sync");
        let elapsed = started.elapsed();
        let records = Journal::replay(&dir.join("main.jsonl")).expect("replay");
        assert_eq!(records.len(), 500);
        for (i, record) in records.iter().enumerate() {
            assert_eq!(record.seq, (i + 1) as u64);
            assert_eq!(record.payload["i"], i as u64);
        }
        assert!(first_seq_gap(&records).is_none());
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "500 appends took {elapsed:?}, budget is <2s"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fsync_policy_is_observable_and_injectable() {
        let dir = temp_journal_dir("fsync");

        // Default policy: counter climbs until the Nth record forces a sync.
        let mut journal = Journal::open(&dir, "default").expect("open journal");
        assert_eq!(journal.records_since_sync(), 0);
        for _ in 0..5 {
            journal
                .append(RecordDraft::new(JournalKind::TurnStarted, None, json!({})))
                .expect("append");
        }
        assert_eq!(journal.records_since_sync(), 5);
        journal.flush_and_sync().expect("explicit sync");
        assert_eq!(journal.records_since_sync(), 0);

        // Injectable N=1: every record is synced immediately; data must land.
        let mut strict = Journal::open_with_policy(&dir, "strict", 1).expect("open strict journal");
        for i in 0..3u64 {
            strict
                .append(RecordDraft::new(
                    JournalKind::MessagePersisted,
                    None,
                    json!({"i": i}),
                ))
                .expect("append");
            assert_eq!(strict.records_since_sync(), 0, "N=1 must sync every record");
        }
        drop(strict);
        let records = Journal::replay(&dir.join("strict.jsonl")).expect("replay");
        assert_eq!(records.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_kinds_survive_replay_via_other_escape_hatch() {
        let dir = temp_journal_dir("future-kind");
        let mut journal = Journal::open(&dir, "main").expect("open journal");
        journal
            .append(RecordDraft::new(
                JournalKind::Other("workflow_step_started".into()),
                None,
                json!({"step": 1}),
            ))
            .expect("append");
        drop(journal);

        // On-disk form uses the raw snake_case string, so newer binaries and
        // older binaries agree on the wire format.
        let raw = std::fs::read_to_string(dir.join("main.jsonl")).expect("read raw");
        assert!(
            raw.contains("\"kind\":\"workflow_step_started\""),
            "raw: {raw}"
        );

        let records = Journal::replay(&dir.join("main.jsonl")).expect("replay");
        assert_eq!(
            records[0].kind,
            JournalKind::Other("workflow_step_started".into())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn known_kinds_serialize_as_snake_case_strings() {
        let record = Record {
            seq: 7,
            ts_ms: 1_770_000_000_042,
            kind: JournalKind::CommandReceived,
            thread_id: Some("thread-1".into()),
            payload: json!({}),
        };
        let line = serde_json::to_string(&record).expect("serialize");
        assert!(
            line.contains("\"kind\":\"command_received\""),
            "line: {line}"
        );
        assert!(line.contains("\"ts_ms\":1770000000042"), "line: {line}");
        let parsed: Record = serde_json::from_str(&line).expect("deserialize");
        assert_eq!(parsed, record);
    }

    #[test]
    fn seq_gap_helper_reports_first_gap_and_none_when_contiguous() {
        let make = |seqs: &[u64]| -> Vec<Record> {
            seqs.iter()
                .map(|&seq| Record {
                    seq,
                    ts_ms: 0,
                    kind: JournalKind::TurnStarted,
                    thread_id: None,
                    payload: json!({}),
                })
                .collect()
        };
        assert_eq!(first_seq_gap(&make(&[])), None);
        assert_eq!(first_seq_gap(&make(&[1, 2, 3])), None);
        // Rotated segments may start above 1: baseline, not a gap.
        assert_eq!(first_seq_gap(&make(&[41, 42, 43])), None);
        assert_eq!(first_seq_gap(&make(&[1, 2, 4])), Some((2, 3)));
        assert_eq!(
            first_seq_gap(&make(&[1, 1])),
            Some((1, 2)),
            "regression counts as gap"
        );
        assert_eq!(first_seq_gap(&make(&[1, 5, 6])), Some((1, 2)));
    }

    #[test]
    fn trailing_empty_lines_are_tolerated_during_replay() {
        let dir = temp_journal_dir("trailing");
        let mut journal = Journal::open(&dir, "main").expect("open journal");
        journal
            .append(RecordDraft::new(JournalKind::TurnStarted, None, json!({})))
            .expect("append");
        drop(journal);
        let path = dir.join("main.jsonl");
        let mut raw = std::fs::read_to_string(&path).expect("read");
        raw.push('\n'); // simulate a torn/crashed final newline
        std::fs::write(&path, raw).expect("rewrite");
        let records = Journal::replay(&path).expect("replay tolerates blank tail");
        assert_eq!(records.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_journal_tail_truncates_nothing() {
        let dir = temp_journal_dir("tail-clean");
        {
            let mut journal = Journal::open(&dir, "main").expect("open journal");
            journal
                .append(RecordDraft::new(JournalKind::TurnStarted, None, json!({})))
                .expect("append");
        }
        let path = dir.join("main.jsonl");
        let before = std::fs::metadata(&path).expect("stat").len();
        assert_eq!(
            truncate_corrupt_tail(&path).expect("clean file needs no repair"),
            0,
            "a complete final record (even without trailing newline) is not corrupt"
        );
        assert_eq!(std::fs::metadata(&path).expect("stat").len(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn torn_final_line_is_auto_repaired_on_replay_and_file_shrinks() {
        let dir = temp_journal_dir("tail-torn");
        let path = dir.join("torn.jsonl");
        let record = |seq: u64| {
            serde_json::to_string(&Record {
                seq,
                ts_ms: 42,
                kind: JournalKind::TurnStarted,
                thread_id: None,
                payload: json!({}),
            })
            .expect("serialize")
        };
        let mut body = format!("{}\n{}\n{}\n", record(1), record(2), record(3));
        body.push_str("{\"seq\":9"); // torn write: no newline, invalid JSON
        std::fs::write(&path, &body).expect("seed torn journal");
        let before = std::fs::metadata(&path).expect("stat").len();

        // Replay must self-repair the torn tail and succeed.
        let records = Journal::replay(&path).expect("replay repairs torn tail");
        assert_eq!(records.len(), 3);
        assert_eq!(records.last().expect("non-empty").seq, 3);

        // File shrank by exactly the torn bytes and replays stay green.
        let after = std::fs::metadata(&path).expect("stat").len();
        assert!(after < before, "file must shrink: {before} -> {after}");
        assert_eq!(after as usize + "{\"seq\":9".len(), before as usize);
        assert_eq!(
            Journal::replay(&path).expect("second replay"),
            records,
            "repair is idempotent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn middle_corrupt_line_still_fails_loud_and_file_untouched() {
        let dir = temp_journal_dir("tail-middle");
        let dir2 = temp_journal_dir("tail-middle-empty");
        let good = Record {
            seq: 1,
            ts_ms: 1_770_000_000_000,
            kind: JournalKind::CommandReceived,
            thread_id: None,
            payload: json!({"x": 1}),
        };
        let path = dir.join("middle.jsonl");
        let body = format!(
            "{}\n{{\"seq\": not json at all\n{}\n",
            serde_json::to_string(&good).expect("serialize"),
            serde_json::to_string(&Record {
                seq: 2,
                ..good.clone()
            })
            .expect("serialize")
        );
        std::fs::write(&path, &body).expect("seed journal with mid-file corruption");
        let err = Journal::replay(&path).expect_err("mid-file corruption fails loud");
        assert!(err.contains("line 2"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            body,
            "auto-repair must not touch mid-file corruption"
        );

        // Empty segment: nothing to repair, replays to an empty vec.
        let empty_path = dir2.join("empty.jsonl");
        std::fs::write(&empty_path, b"").expect("seed empty journal");
        assert_eq!(truncate_corrupt_tail(&empty_path).expect("no-op"), 0);
        assert!(Journal::replay(&empty_path)
            .expect("empty file replays ok")
            .is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn truncate_corrupt_tail_drops_lone_newlineless_torn_line() {
        let dir = temp_journal_dir("tail-lone");
        let path = dir.join("lone.jsonl");
        std::fs::write(&path, b"{\"seq\":9,\"ts_m").expect("seed lone torn line");
        assert_eq!(
            truncate_corrupt_tail(&path).expect("repair lone torn line"),
            14, // every byte of the newline-less torn line
        );
        assert_eq!(std::fs::read(&path).expect("read back"), b"");
        assert!(Journal::replay(&path).expect("now empty").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
