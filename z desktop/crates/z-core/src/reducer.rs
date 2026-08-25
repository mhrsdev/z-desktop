//! Reducer views over the journal (jour-006 fold API, jour-007 threads view,
//! jour-008/orch-001 task view + store, jour-009 usage counters).
//!
//! ADR-0012: task records exist only as journal events folded by a reducer —
//! there is no tasks.json. Views are pure folds over [`Journal::replay`];
//! writes are append-only events. Unknown future kinds deserialize into
//! [`JournalKind::Other`] and are simply ignored by every view here.

use crate::journal::{first_seq_gap, lag_stats, Journal, JournalKind, Record, RecordDraft};
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

/// ctx-006 freshness: age in ms of the oldest persisted message across all
/// threads, measured against `now_ms`. `None` when no messages exist.
pub fn oldest_message_age_ms(path: &Path, now_ms: u128) -> Result<Option<u128>, String> {
    let mut oldest: Option<u128> = None;
    crate::reducer::fold(path, (), |(), record| {
        if record.kind == JournalKind::MessagePersisted && record.thread_id.is_some() {
            oldest = Some(oldest.unwrap_or(u128::MAX).min(record.ts_ms));
        }
    })?;
    Ok(oldest.map(|ts| now_ms.saturating_sub(ts)))
}

/// jour-021: shape-only CheckpointCreated-style draft summarizing a
/// [`ThreadsView`] snapshot — counts and a wall-clock stamp, no thread
/// contents. Kinded via the [`JournalKind::Other`] escape hatch because
/// journal.rs owns the kind enum; folds as an unknown kind today.
pub fn checkpoint_draft(view: &ThreadsView) -> RecordDraft {
    let threads = view.threads.len() as u64;
    let messages = view.threads.values().map(|t| t.message_count).sum::<u64>();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    RecordDraft::new(
        JournalKind::Other("checkpoint_created".to_string()),
        None,
        serde_json::json!({ "threads": threads, "messages": messages, "ts": ts }),
    )
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

/// jour-024: one-line journal size report — record count (via
/// [`lag_stats`]) plus on-disk bytes as a percentage of the 10 MB segment cap.
pub fn journal_size_report(path: &Path) -> Result<String, String> {
    const CAP_BYTES: u64 = 10 * 1024 * 1024;
    let records = lag_stats(&Journal::replay(path)?).records;
    let bytes = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let pct = bytes.saturating_mul(100) / CAP_BYTES;
    Ok(format!("{records} records, {bytes} bytes ({pct}% of 10MB cap)"))
}

/// jour-023: one-line journal health report — record count, summed sequence
/// gaps, and last seq, all via [`lag_stats`] over the replayed segment.
pub fn seq_health(path: &Path) -> Result<String, String> {
    let stats = lag_stats(&Journal::replay(path)?);
    Ok(format!(
        "{} records, {} gaps, last_seq {}",
        stats.records,
        stats.gaps,
        stats.last_seq.map_or_else(|| "?".to_string(), |s| s.to_string())
    ))
}

/// jour-043: last sequence number accessor — [`lag_stats`]'s `last_seq` over
/// the replayed segment. `None` on an empty segment.
pub fn journal_last_seq(path: &Path) -> Result<Option<u64>, String> {
    Ok(lag_stats(&Journal::replay(path)?).last_seq)
}

/// jour-044: record count accessor — [`lag_stats`]'s `records` over the
/// replayed segment. `0` on an empty segment.
pub fn journal_record_count(path: &Path) -> Result<usize, String> {
    Ok(lag_stats(&Journal::replay(path)?).records)
}

/// jour-045: gap-presence accessor — whether [`lag_stats`]'s summed sequence
/// gaps over the replayed segment is non-zero.
pub fn journal_has_gaps(path: &Path) -> Result<bool, String> {
    Ok(lag_stats(&Journal::replay(path)?).gaps > 0)
}

/// jour-046: empty-segment accessor — whether [`journal_record_count`] over
/// the replayed segment is zero.
pub fn journal_is_empty(path: &Path) -> Result<bool, String> {
    Ok(lag_stats(&Journal::replay(path)?).records == 0)
}

/// jour-025: combined journal health line — [`seq_health`] and
/// [`journal_size_report`] joined with " | ".
pub fn journal_health_line(path: &Path) -> Result<String, String> {
    Ok(format!(
        "{} | {}",
        seq_health(path)?,
        journal_size_report(path)?
    ))
}

/// jour-031: journal health as pretty JSON — `{records, bytes, pct_of_cap,
/// gaps, last_seq}` combining [`lag_stats`] and on-disk size (same 10 MB cap
/// basis as [`journal_size_report`]). `last_seq` is `null` on an empty segment.
pub fn journal_health_json(path: &Path) -> Result<String, String> {
    const CAP_BYTES: u64 = 10 * 1024 * 1024;
    let stats = lag_stats(&Journal::replay(path)?);
    let bytes = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let pct_of_cap = bytes.saturating_mul(100) / CAP_BYTES;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "records": stats.records,
        "bytes": bytes,
        "pct_of_cap": pct_of_cap,
        "gaps": stats.gaps,
        "last_seq": stats.last_seq,
    }))
    .map_err(|e| e.to_string())?)
}

/// jour-040: journal size as pretty JSON — `{records, bytes, pct_of_cap}`,
/// the size subset of [`journal_health_json`] (same 10 MB cap basis).
pub fn journal_size_json(path: &Path) -> Result<String, String> {
    const CAP_BYTES: u64 = 10 * 1024 * 1024;
    let records = lag_stats(&Journal::replay(path)?).records;
    let bytes = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let pct_of_cap = bytes.saturating_mul(100) / CAP_BYTES;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "records": records,
        "bytes": bytes,
        "pct_of_cap": pct_of_cap,
    }))
    .map_err(|e| e.to_string())?)
}

/// jour-041: journal gaps as pretty JSON — `{records, gaps}` from
/// [`lag_stats`], the lag subset of [`journal_health_json`].
pub fn journal_gaps_json(path: &Path) -> Result<String, String> {
    let stats = lag_stats(&Journal::replay(path)?);
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "records": stats.records,
        "gaps": stats.gaps,
    }))
    .map_err(|e| e.to_string())?)
}

/// jour-042: journal gaps as a single compact JSONL-style line —
/// `{records, gaps}` from [`lag_stats`], the single-line form of
/// [`journal_gaps_json`].
pub fn journal_gaps_jsonl(path: &Path) -> Result<String, String> {
    let stats = lag_stats(&Journal::replay(path)?);
    serde_json::to_string(&serde_json::json!({
        "records": stats.records,
        "gaps": stats.gaps,
    }))
    .map_err(|e| e.to_string())
}

/// jour-047: journal health as a single compact JSONL-style line —
/// `{records, bytes, pct_of_cap, gaps}` combining [`lag_stats`] and on-disk
/// size (same 10 MB cap basis as [`journal_health_json`]).
pub fn journal_health_jsonl(path: &Path) -> Result<String, String> {
    const CAP_BYTES: u64 = 10 * 1024 * 1024;
    let stats = lag_stats(&Journal::replay(path)?);
    let bytes = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let pct_of_cap = bytes.saturating_mul(100) / CAP_BYTES;
    serde_json::to_string(&serde_json::json!({
        "records": stats.records,
        "bytes": bytes,
        "pct_of_cap": pct_of_cap,
        "gaps": stats.gaps,
    }))
    .map_err(|e| e.to_string())
}

/// jour-026: metadata-only JSON export — pretty-printed JSON array of
/// `{seq, kind, thread_id, ts_ms}` for every record in the segment.
/// Payloads are deliberately excluded: this export is metadata-only.
pub fn journal_export_json(path: &Path) -> Result<String, String> {
    #[derive(Serialize)]
    struct Entry<'a> {
        seq: u64,
        kind: &'a JournalKind,
        thread_id: &'a Option<String>,
        ts_ms: u128,
    }
    let records = Journal::replay(path)?;
    let entries: Vec<Entry> = records
        .iter()
        .map(|r| Entry {
            seq: r.seq,
            kind: &r.kind,
            thread_id: &r.thread_id,
            ts_ms: r.ts_ms,
        })
        .collect();
    serde_json::to_string_pretty(&entries).map_err(|e| format!("reducer: export json: {e}"))
}

/// jour-030: metadata-only JSONL export — one compact JSON object per line
/// (`{seq, kind, thread_id, ts_ms}`), no pretty-printing, for streaming
/// consumers. Payloads are deliberately excluded, matching
/// [`journal_export_json`]. Empty segment yields an empty string.
pub fn journal_export_jsonl(path: &Path) -> Result<String, String> {
    #[derive(Serialize)]
    struct Entry<'a> {
        seq: u64,
        kind: &'a JournalKind,
        thread_id: &'a Option<String>,
        ts_ms: u128,
    }
    let records = Journal::replay(path)?;
    let lines: Result<Vec<String>, String> = records
        .iter()
        .map(|r| {
            serde_json::to_string(&Entry {
                seq: r.seq,
                kind: &r.kind,
                thread_id: &r.thread_id,
                ts_ms: r.ts_ms,
            })
            .map_err(|e| format!("reducer: export jsonl: {e}"))
        })
        .collect();
    Ok(lines?.join("\n"))
}

/// jour-034: compact JSONL of one thread's persisted messages — one
/// `{text, ts_ms}` object per line, journal order. Records without a `text`
/// payload are skipped. Unknown/empty thread yields an empty string.
pub fn journal_thread_jsonl(path: &Path, thread_id: &str) -> Result<String, String> {
    #[derive(Serialize)]
    struct Entry<'a> {
        text: &'a str,
        ts_ms: u128,
    }
    let mut lines: Vec<String> = Vec::new();
    for r in Journal::replay(path)? {
        if r.kind != JournalKind::MessagePersisted || r.thread_id.as_deref() != Some(thread_id) {
            continue;
        }
        if let Some(text) = r.payload.get("text").and_then(Value::as_str) {
            let line = serde_json::to_string(&Entry {
                text,
                ts_ms: r.ts_ms,
            })
            .map_err(|e| format!("reducer: thread jsonl: {e}"))?;
            lines.push(line);
        }
    }
    Ok(lines.join("\n"))
}

/// jour-039: one thread's persisted messages as a pretty-printed JSON array
/// of `{text, ts_ms}` in journal order. Records without a `text` payload are
/// skipped, matching [`journal_thread_jsonl`]. Unknown/empty thread yields
/// `[]`.
pub fn journal_thread_json(path: &Path, thread_id: &str) -> Result<String, String> {
    let mut entries: Vec<Value> = Vec::new();
    for r in Journal::replay(path)? {
        if r.kind != JournalKind::MessagePersisted || r.thread_id.as_deref() != Some(thread_id) {
            continue;
        }
        if let Some(text) = r.payload.get("text").and_then(Value::as_str) {
            entries.push(serde_json::json!({ "text": text, "ts_ms": r.ts_ms }));
        }
    }
    serde_json::to_string_pretty(&entries).map_err(|e| format!("reducer: thread json: {e}"))
}

/// jour-027: per-kind record counts for one segment, sorted by count
/// descending (ties broken by kind name ascending). Replays the same
/// [`Journal::replay`] segment the [`lag_stats`] reports consume, so totals
/// stay consistent with them.
pub fn journal_kind_counts(path: &Path) -> Result<Vec<(String, usize)>, String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for record in Journal::replay(path)? {
        // JournalKind::as_str is private to journal.rs; its Serialize impl
        // emits the same string, so round-trip through JSON for the name.
        let name = serde_json::to_string(&record.kind)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        *counts.entry(name).or_insert(0) += 1;
    }
    let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(pairs)
}

/// jour-032: per-kind counts as pretty JSON — array of `{kind, count}` in the
/// same count-desc/name-asc order as [`journal_kind_counts`]. Empty segment
/// yields `[]`.
pub fn journal_kind_json(path: &Path) -> Result<String, String> {
    #[derive(Serialize)]
    struct Entry {
        kind: String,
        count: usize,
    }
    let entries: Vec<Entry> = journal_kind_counts(path)?
        .into_iter()
        .map(|(kind, count)| Entry { kind, count })
        .collect();
    serde_json::to_string_pretty(&entries).map_err(|e| format!("reducer: kind json: {e}"))
}

/// jour-037: per-kind counts as compact JSONL — one `{kind, count}` object
/// per line in the same count-desc/name-asc order as [`journal_kind_counts`].
/// Empty segment yields an empty string.
pub fn journal_kind_counts_jsonl(path: &Path) -> Result<String, String> {
    #[derive(Serialize)]
    struct Entry {
        kind: String,
        count: usize,
    }
    let lines = journal_kind_counts(path)?
        .into_iter()
        .map(|(kind, count)| serde_json::to_string(&Entry { kind, count }))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("reducer: kind counts jsonl: {e}"))?;
    Ok(lines.join("\n"))
}

/// jour-048: thread summary as pretty JSON — `{threads, messages}` from
/// [`ThreadsView::fold`]: distinct thread ids seen and the total of their
/// persisted `message_count`s. Empty segment yields zeros.
pub fn journal_thread_summary_json(path: &Path) -> Result<String, String> {
    let view = ThreadsView::fold(path)?;
    let messages: u64 = view.threads.values().map(|t| t.message_count).sum();
    serde_json::to_string_pretty(&serde_json::json!({
        "threads": view.threads.len(),
        "messages": messages,
    }))
    .map_err(|e| format!("reducer: thread summary json: {e}"))
}

/// jour-049: thread summary as a single compact JSONL-style line —
/// `{threads, messages}` from [`ThreadsView::fold`]: distinct thread ids seen
/// and the total of their persisted `message_count`s. Empty segment yields
/// zeros.
pub fn journal_thread_summary_jsonl(path: &Path) -> Result<String, String> {
    let view = ThreadsView::fold(path)?;
    let messages: u64 = view.threads.values().map(|t| t.message_count).sum();
    serde_json::to_string(&serde_json::json!({
        "threads": view.threads.len(),
        "messages": messages,
    }))
    .map_err(|e| format!("reducer: thread summary jsonl: {e}"))
}

/// jour-050: total persisted message count — sum of [`ThreadSummary::message_count`]
/// over [`ThreadsView::fold`]. Empty segment yields 0.
pub fn journal_message_count(path: &Path) -> Result<usize, String> {
    let view = ThreadsView::fold(path)?;
    Ok(view.threads.values().map(|t| t.message_count).sum::<u64>() as usize)
}

/// jour-051: total persisted message count as a single compact JSONL-style
/// line — `{messages}` from [`journal_message_count`], the single-line form
/// of that fold. Empty segment yields `{"messages":0}`.
pub fn journal_message_count_jsonl(path: &Path) -> Result<String, String> {
    let messages = journal_message_count(path)?;
    serde_json::to_string(&serde_json::json!({ "messages": messages })).map_err(|e| e.to_string())
}

/// jour-052: thread summary report as a single compact JSONL-style line —
/// `{threads, messages, bytes}` combining [`ThreadsView::fold`] (distinct
/// thread ids, total persisted `message_count`s) with on-disk segment size.
/// Empty segment yields all zeros.
pub fn journal_thread_summary_report(path: &Path) -> Result<String, String> {
    let view = ThreadsView::fold(path)?;
    let messages: u64 = view.threads.values().map(|t| t.message_count).sum();
    let bytes = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    serde_json::to_string(&serde_json::json!({
        "threads": view.threads.len(),
        "messages": messages,
        "bytes": bytes,
    }))
    .map_err(|e| format!("reducer: thread summary report: {e}"))
}

/// jour-053: per-thread report as pretty JSON — `{thread, messages}` where
/// `messages` is the thread's persisted `message_count` from
/// [`ThreadsView::fold`]. Unknown thread yields zero messages.
pub fn journal_thread_report_json(path: &Path, thread_id: &str) -> Result<String, String> {
    let count = ThreadsView::fold(path)?
        .threads
        .get(thread_id)
        .map(|t| t.message_count)
        .unwrap_or(0);
    serde_json::to_string_pretty(&serde_json::json!({
        "thread": thread_id,
        "messages": count,
    }))
    .map_err(|e| e.to_string())
}

/// jour-054: per-thread report as a single-line compact JSONL string —
/// `{thread, messages}` where `messages` is the thread's persisted
/// `message_count` from [`ThreadsView::fold`]. Unknown thread yields zero
/// messages.
pub fn journal_thread_jsonl_report(path: &Path, thread_id: &str) -> Result<String, String> {
    let count = ThreadsView::fold(path)?
        .threads
        .get(thread_id)
        .map(|t| t.message_count)
        .unwrap_or(0);
    serde_json::to_string(&serde_json::json!({
        "thread": thread_id,
        "messages": count,
    }))
    .map_err(|e| e.to_string())
}

/// jour-038: compact JSONL of all records of one kind — one
/// `{seq, kind, thread_id, ts_ms}` object per line, journal order, same
/// metadata shape as [`journal_export_jsonl`]. Unknown/empty kind yields an
/// empty string.
pub fn journal_kind_jsonl(path: &Path, kind: &str) -> Result<String, String> {
    #[derive(Serialize)]
    struct Entry<'a> {
        seq: u64,
        kind: &'a JournalKind,
        thread_id: &'a Option<String>,
        ts_ms: u128,
    }
    let mut lines: Vec<String> = Vec::new();
    for r in Journal::replay(path)? {
        // Same name round-trip as journal_kind_counts (as_str is private).
        let name = serde_json::to_string(&r.kind)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        if name != kind {
            continue;
        }
        let line = serde_json::to_string(&Entry {
            seq: r.seq,
            kind: &r.kind,
            thread_id: &r.thread_id,
            ts_ms: r.ts_ms,
        })
        .map_err(|e| format!("reducer: kind jsonl: {e}"))?;
        lines.push(line);
    }
    Ok(lines.join("\n"))
}

/// jour-028: per-thread persisted message counts for one segment, sorted by
/// count descending (ties broken by thread id ascending). Reuses the shared
/// [`fold`] so seq-gap handling matches every other view here.
pub fn thread_message_counts(path: &Path) -> Result<Vec<(String, usize)>, String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    crate::reducer::fold(path, (), |(), record| {
        if record.kind == JournalKind::MessagePersisted {
            if let Some(thread) = &record.thread_id {
                *counts.entry(thread.clone()).or_insert(0) += 1;
            }
        }
    })?;
    let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(pairs)
}

/// jour-029: top-N threads by persisted message count (ties broken by thread
/// id ascending). Reuses [`thread_message_counts`]; `n == 0` yields empty.
pub fn journal_top_threads(path: &Path, n: usize) -> Result<Vec<(String, usize)>, String> {
    thread_message_counts(path).map(|mut pairs| {
        pairs.truncate(n);
        pairs
    })
}

/// jour-033: pretty JSON array of `{ "thread": ..., "messages": ... }` for
/// [`journal_top_threads`]. Empty input yields `[]`.
pub fn journal_top_threads_json(path: &Path, n: usize) -> Result<String, String> {
    #[derive(Serialize)]
    struct Entry {
        thread: String,
        messages: usize,
    }
    let entries: Vec<Entry> = journal_top_threads(path, n)?
        .into_iter()
        .map(|(thread, messages)| Entry { thread, messages })
        .collect();
    serde_json::to_string_pretty(&entries).map_err(|e| format!("reducer: top threads json: {e}"))
}

/// jour-035: pretty JSON array of `{ "thread": ..., "messages": ... }` for
/// every thread in [`thread_message_counts`] (count desc, id asc order).
/// Empty segment yields `[]`.
pub fn journal_thread_counts_json(path: &Path) -> Result<String, String> {
    #[derive(Serialize)]
    struct Entry {
        thread: String,
        messages: usize,
    }
    let entries: Vec<Entry> = thread_message_counts(path)?
        .into_iter()
        .map(|(thread, messages)| Entry { thread, messages })
        .collect();
    serde_json::to_string_pretty(&entries).map_err(|e| format!("reducer: thread counts json: {e}"))
}

/// jour-036: compact JSONL — one `{ "thread": ..., "messages": ... }` object
/// per line for every thread in [`thread_message_counts`] (count desc, id asc
/// order). Empty segment yields an empty string.
pub fn journal_thread_counts_jsonl(path: &Path) -> Result<String, String> {
    #[derive(Serialize)]
    struct Entry {
        thread: String,
        messages: usize,
    }
    let lines: Result<Vec<String>, String> = thread_message_counts(path)?
        .into_iter()
        .map(|(thread, messages)| {
            serde_json::to_string(&Entry { thread, messages })
                .map_err(|e| format!("reducer: thread counts jsonl: {e}"))
        })
        .collect();
    Ok(lines?.join("\n"))
}

/// jour-017: generates a synthetic, deterministic journal segment at `path`.
///
/// Each of `turns` turns contributes one `turn_started` record followed by
/// `msgs_per_turn` `message_persisted` records, with sequential seqs from 1
/// and one thread per turn. Unlike [`Journal::append`] the timestamps are
/// fixed constants, not wall-clock, so two calls with the same parameters
/// produce byte-identical files. Returns the total record count
/// (`turns * (1 + msgs_per_turn)`).
pub fn write_fixture(path: &Path, turns: usize, msgs_per_turn: usize) -> Result<usize, String> {
    // Fixed timestamp base: determinism beats realism for fixtures.
    const FIXTURE_BASE_TS_MS: u128 = 1_770_000_000_000;

    // Serializes exactly like Journal::append does (same `Record` shape).
    fn line(
        seq: u64,
        kind: JournalKind,
        thread: String,
        payload: Value,
        out: &mut String,
    ) -> Result<(), String> {
        let mut s = serde_json::to_string(&Record {
            seq,
            ts_ms: FIXTURE_BASE_TS_MS + u128::from(seq),
            kind,
            thread_id: Some(thread),
            payload,
        })
        .map_err(|e| format!("reducer: fixture record {seq}: {e}"))?;
        s.push('\n');
        out.push_str(&s);
        Ok(())
    }

    let mut body = String::new();
    let mut seq: u64 = 0;
    for turn in 1..=turns {
        let thread = format!("fixture-turn-{turn}");
        seq += 1;
        line(
            seq,
            JournalKind::TurnStarted,
            thread.clone(),
            serde_json::json!({}),
            &mut body,
        )?;
        for msg in 1..=msgs_per_turn {
            seq += 1;
            line(
                seq,
                JournalKind::MessagePersisted,
                thread.clone(),
                serde_json::json!({ "text": format!("fixture message {turn}-{msg}") }),
                &mut body,
            )?;
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("reducer: cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, body)
        .map_err(|e| format!("reducer: cannot write fixture {}: {e}", path.display()))?;
    Ok(seq as usize)
}

/// jour-019: replays the journal segment at `path` exactly once, wall-clock
/// timed. Returns `(record_count, elapsed_ms)` so callers (and the smoke
/// test below) can assert replay speed against their own budget.
pub fn replay_perf_smoke(path: &Path) -> Result<(usize, u128), String> {
    let start = std::time::Instant::now();
    let count = Journal::replay(path)?.len();
    Ok((count, start.elapsed().as_millis()))
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
    /// jour-022: status carried by each task's LAST `task_state_changed`
    /// event, tracked during fold so re-sync helpers can detect drift between
    /// the folded state and what the journal last said (they only diverge
    /// when a caller mutates `tasks` out-of-band, e.g. from a snapshot).
    pub(crate) last_event_status: HashMap<String, TaskStatus>,
    /// orch-019: per-task status history `(status, ts_ms)` in journal order,
    /// appended during fold so [`task_timeline`] can report it.
    timeline: HashMap<String, Vec<(String, Option<u128>)>>,
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
                        view.last_event_status.insert(id.to_string(), status);
                        let status_str = serde_json::to_value(&status)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_string))
                            .unwrap_or_default();
                        view.timeline
                            .entry(id.to_string())
                            .or_default()
                            .push((status_str, Some(record.ts_ms)));
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

/// orch-022: ids of tasks whose current status matches `status`, using the
/// same snake_case spelling as the journal ("pending"|"running"|"done"|
/// "failed"). Unknown statuses yield an empty list. Sorted for deterministic
/// order, like [`TasksView::ready_set`].
pub fn tasks_by_status(view: &TasksView, status: &str) -> Vec<String> {
    let want = match status {
        "pending" => TaskStatus::Pending,
        "running" => TaskStatus::Running,
        "done" => TaskStatus::Done,
        "failed" => TaskStatus::Failed,
        _ => return Vec::new(),
    };
    let mut ids: Vec<String> = view
        .tasks
        .values()
        .filter(|t| t.status == want)
        .map(|t| t.id.clone())
        .collect();
    ids.sort();
    ids
}

/// orch-027: pretty-printed JSON array of ids from [`tasks_by_status`], for
/// UI/state export. Unknown statuses yield `"[]"`.
pub fn tasks_by_status_json(view: &TasksView, status: &str) -> String {
    let ids: Vec<Value> = tasks_by_status(view, status)
        .into_iter()
        .map(Value::String)
        .collect();
    serde_json::to_string_pretty(&ids).unwrap_or_else(|_| "[]".into())
}

/// orch-021: one-line human summary of task health.
pub fn task_health_line(view: &TasksView) -> String {
    let (done, running, failed, pending) = task_counts(view);
    format!(
        "{} tasks: {} done, {} running, {} failed, {} pending",
        done + running + failed + pending,
        done,
        running,
        failed,
        pending
    )
}

/// orch-023: pretty JSON status summary (`{total, done, running, failed,
/// pending}`) from [`task_counts`], for UI/state export.
pub fn task_status_summary_json(view: &TasksView) -> String {
    let (done, running, failed, pending) = task_counts(view);
    let summary = serde_json::json!({
        "total": done + running + failed + pending,
        "done": done,
        "running": running,
        "failed": failed,
        "pending": pending,
    });
    serde_json::to_string_pretty(&summary).unwrap_or_else(|_| "{}".into())
}

/// orch-028: pretty JSON status counts (`{done, running, failed, pending}`) from
/// [`task_counts`] — like [`task_status_summary_json`] without `total`.
pub fn task_counts_json(view: &TasksView) -> String {
    let (done, running, failed, pending) = task_counts(view);
    let counts = serde_json::json!({
        "done": done,
        "running": running,
        "failed": failed,
        "pending": pending,
    });
    serde_json::to_string_pretty(&counts).unwrap_or_else(|_| "{}".into())
}

/// orch-029: single-line compact JSONL status counts
/// (`{"done":n,"running":n,"failed":n,"pending":n}`) from [`task_counts`] —
/// compact twin of [`task_counts_json`] for line-oriented exports.
pub fn task_status_counts_jsonl(view: &TasksView) -> String {
    let (done, running, failed, pending) = task_counts(view);
    serde_json::json!({
        "done": done,
        "running": running,
        "failed": failed,
        "pending": pending,
    })
    .to_string()
}

/// orch-024: pretty JSON health snapshot combining [`task_counts`] with the
/// total number of folded timeline events: `{counts: {...}, events: n}`.
pub fn tasks_health_json(view: &TasksView) -> String {
    let (done, running, failed, pending) = task_counts(view);
    let events: usize = view.timeline.values().map(Vec::len).sum();
    let health = serde_json::json!({
        "counts": {
            "total": done + running + failed + pending,
            "done": done,
            "running": running,
            "failed": failed,
            "pending": pending,
        },
        "events": events,
    });
    serde_json::to_string_pretty(&health).unwrap_or_else(|_| "{}".into())
}

/// orch-018: every task's current status as `(id, status)` sorted by id, for
/// UI/state export. Status strings use the journal's snake_case spelling.
pub fn task_state_events(view: &TasksView) -> Vec<(String, String)> {
    let mut events: Vec<(String, String)> = view
        .tasks
        .values()
        .map(|t| {
            let status = serde_json::to_value(&t.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            (t.id.clone(), status)
        })
        .collect();
    events.sort_by(|a, b| a.0.cmp(&b.0));
    events
}

/// orch-018: pretty-printed JSON array of `{id, status}` for the UI.
pub fn task_state_json(view: &TasksView) -> String {
    let items: Vec<Value> = task_state_events(view)
        .into_iter()
        .map(|(id, status)| serde_json::json!({ "id": id, "status": status }))
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
}

/// orch-019: one task's ordered status history as `(status, ts_ms)` pairs in
/// journal order. Unknown ids return an empty vec. Status strings use the
/// journal's snake_case spelling.
pub fn task_timeline(view: &TasksView, id: &str) -> Vec<(String, Option<u128>)> {
    view.timeline.get(id).cloned().unwrap_or_default()
}

/// orch-026: pretty-printed JSON array of `{status, ts}` pairs for one task's
/// timeline in journal order, built on [`task_timeline`]. Unknown ids yield
/// `"[]"`.
pub fn task_timeline_json(view: &TasksView, id: &str) -> String {
    let items: Vec<Value> = task_timeline(view, id)
        .into_iter()
        .map(|(status, ts)| serde_json::json!({ "status": status, "ts": ts }))
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
}

/// orch-020: pretty-printed JSON array of `{id, status, timeline_len}` for all
/// tasks sorted by id — full-state export built on [`task_state_events`] +
/// [`task_timeline`].
pub fn tasks_export_json(view: &TasksView) -> String {
    let items: Vec<Value> = task_state_events(view)
        .into_iter()
        .map(|(id, status)| {
            serde_json::json!({
                "id": id,
                "status": status,
                "timeline_len": task_timeline(view, &id).len(),
            })
        })
        .collect();
    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
}

/// orch-025: compact JSONL export — one `{id, status}` object per line,
/// sorted by id (via [`task_state_events`]). Empty view → empty string.
pub fn tasks_export_jsonl(view: &TasksView) -> String {
    task_state_events(view)
        .into_iter()
        .map(|(id, status)| serde_json::json!({ "id": id, "status": status }).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// jour-022: idempotent re-sync batch — one shape-only draft per task whose
/// folded status differs from its LAST `task_state_changed` event (or that
/// has no recorded event yet). Each draft is identical in shape to what
/// [`TaskStore::transition`] appends, so appending the returned batch and
/// re-folding yields no further drift. Sorted by id for deterministic batches.
pub fn task_state_journal_events(view: &TasksView) -> Vec<RecordDraft> {
    let mut stale: Vec<&TaskRecord> = view
        .tasks
        .values()
        .filter(|t| view.last_event_status.get(&t.id) != Some(&t.status))
        .collect();
    stale.sort_by(|a, b| a.id.cmp(&b.id));
    stale
        .into_iter()
        .map(|t| {
            RecordDraft::new(
                JournalKind::TaskStateChanged,
                None,
                serde_json::json!({ "id": t.id, "status": t.status }),
            )
        })
        .collect()
}

/// Newest `user`-role `MessagePersisted` record's `text`, via one full replay.
/// Both payload shapes found in real journals are accepted (`role` from early
/// seeds, `last_role` from the runtime's jour-024 recorder).
fn last_user_message_text(path: &Path) -> Result<Option<String>, String> {
    Ok(Journal::replay(path)?
        .iter()
        .rev()
        .find(|r| {
            r.kind == JournalKind::MessagePersisted
                && ["role", "last_role"]
                    .iter()
                    .any(|k| r.payload.get(k).and_then(Value::as_str) == Some("user"))
        })
        .and_then(|r| r.payload.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// ctx-013: runtime check of the never-compact-latest-user-message invariant.
///
/// Folds the segment into [`ThreadsView`] TWICE (two fully separate replays of
/// the same bytes, same posture as [`fold_twice_equal`]) and additionally
/// verifies the LAST user message's text survives both folds byte-identical:
/// each round independently re-extracts the newest user-role
/// `MessagePersisted` text while folding, and both extractions must match
/// exactly. `Ok(true)` on an empty journal (vacuously holds).
pub fn never_compact_latest_user_invariant(path: &Path) -> Result<bool, String> {
    let first_view = ThreadsView::fold(path)?;
    let first_text = last_user_message_text(path)?;
    let second_view = ThreadsView::fold(path)?;
    let second_text = last_user_message_text(path)?;
    Ok(first_view == second_view && first_text == second_text)
}

/// ctx-014: per-thread `(message_text, ts_ms)` items in journal order —
/// the session-layer data a context restore replays into a fresh context.
/// Threads come back sorted by id; non-message records are ignored.
pub fn load_session_items(path: &Path) -> Result<Vec<(String, Vec<(String, u128)>)>, String> {
    let mut items: BTreeMap<String, Vec<(String, u128)>> = BTreeMap::new();
    crate::reducer::fold(path, (), |(), record| {
        if record.kind == JournalKind::MessagePersisted {
            if let Some(thread_id) = &record.thread_id {
                let text = record
                    .payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                items
                    .entry(thread_id.clone())
                    .or_default()
                    .push((text, record.ts_ms));
            }
        }
    })?;
    Ok(items.into_iter().collect())
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
    fn journal_kind_counts_seeded_mixed_kinds_sorted_desc() {
        let dir = temp_dir("kind-counts");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::CommandReceived, None, json!({}));
        }
        assert_eq!(
            journal_kind_counts(&dir.join("main.jsonl")).expect("counts"),
            vec![
                ("message_persisted".to_string(), 3),
                ("command_received".to_string(), 1), // tie with turn_started: name asc
                ("turn_started".to_string(), 1),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_kind_counts_empty_segment_is_empty() {
        let dir = temp_dir("kind-counts-empty");
        // Create the segment file with no records (drop flushes/closes it).
        let _ = Journal::open(&dir, "main").expect("open");
        assert!(journal_kind_counts(&dir.join("main.jsonl"))
            .expect("counts")
            .is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_kind_json_seeded_matches_counts_order_exactly() {
        let dir = temp_dir("jour-032");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        assert_eq!(
            journal_kind_json(&dir.join("main.jsonl")).expect("json"),
            "[\n  {\n    \"kind\": \"message_persisted\",\n    \"count\": 3\n  },\n  {\n    \"kind\": \"turn_started\",\n    \"count\": 1\n  }\n]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_kind_json_empty_segment_is_empty_array() {
        let dir = temp_dir("jour-032-empty");
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_kind_json(&dir.join("main.jsonl")).expect("json"),
            "[]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_summary_json_seeded_counts_threads_and_messages_exactly() {
        let dir = temp_dir("jour-048");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        assert_eq!(
            journal_thread_summary_json(&dir.join("main.jsonl")).expect("json"),
            "{\n  \"messages\": 3,\n  \"threads\": 2\n}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_summary_json_empty_segment_is_zeros() {
        let dir = temp_dir("jour-048-empty");
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_thread_summary_json(&dir.join("main.jsonl")).expect("json"),
            "{\n  \"messages\": 0,\n  \"threads\": 0\n}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_summary_jsonl_seeded_counts_threads_and_messages_exactly() {
        let dir = temp_dir("jour-049");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        assert_eq!(
            journal_thread_summary_jsonl(&dir.join("main.jsonl")).expect("jsonl"),
            "{\"messages\":3,\"threads\":2}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_summary_jsonl_empty_segment_is_zeros() {
        let dir = temp_dir("jour-049-empty");
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_thread_summary_jsonl(&dir.join("main.jsonl")).expect("jsonl"),
            "{\"messages\":0,\"threads\":0}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_message_count_seeded_sums_persisted_messages_exactly() {
        let dir = temp_dir("jour-050");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        assert_eq!(
            journal_message_count(&dir.join("main.jsonl")).expect("count"),
            3
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_message_count_empty_segment_is_zero() {
        let dir = temp_dir("jour-050-empty");
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_message_count(&dir.join("main.jsonl")).expect("count"),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_message_count_jsonl_seeded_yields_exact_compact_line() {
        let dir = temp_dir("jour-051");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        assert_eq!(
            journal_message_count_jsonl(&dir.join("main.jsonl")).expect("jsonl"),
            "{\"messages\":3}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_message_count_jsonl_empty_segment_is_zeros_line() {
        let dir = temp_dir("jour-051-empty");
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_message_count_jsonl(&dir.join("main.jsonl")).expect("jsonl"),
            "{\"messages\":0}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_summary_report_seeded_matches_summary_plus_disk_bytes() {
        let dir = temp_dir("jour-052");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        let path = dir.join("main.jsonl");
        let out = journal_thread_summary_report(&path).expect("report");
        // bytes tracks wall-clock ts_ms, so assert the exact field values
        // (incl. bytes == real on-disk size) instead of a brittle full string.
        assert_eq!(out.lines().count(), 1);
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v["threads"], 2);
        assert_eq!(v["messages"], 3);
        assert_eq!(v["bytes"], std::fs::metadata(&path).expect("meta").len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_summary_report_empty_segment_is_zeros_line() {
        let dir = temp_dir("jour-052-empty");
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_thread_summary_report(&dir.join("main.jsonl")).expect("report"),
            "{\"bytes\":0,\"messages\":0,\"threads\":0}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-053 ---------------------------------------------------------------

    #[test]
    fn journal_thread_report_json_seeded_matches_exact_pretty_json() {
        let dir = temp_dir("jour-053");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        assert_eq!(
            journal_thread_report_json(&dir.join("main.jsonl"), "t1").expect("report"),
            "{\n  \"messages\": 2,\n  \"thread\": \"t1\"\n}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_report_json_unknown_thread_is_zero_messages() {
        let dir = temp_dir("jour-053-empty");
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_thread_report_json(&dir.join("main.jsonl"), "nope").expect("report"),
            "{\n  \"messages\": 0,\n  \"thread\": \"nope\"\n}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-054 ---------------------------------------------------------------

    #[test]
    fn journal_thread_jsonl_report_seeded_matches_exact_line() {
        let dir = temp_dir("jour-054");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        assert_eq!(
            journal_thread_jsonl_report(&dir.join("main.jsonl"), "t1").expect("report"),
            "{\"messages\":2,\"thread\":\"t1\"}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_jsonl_report_unknown_thread_is_zero_messages() {
        let dir = temp_dir("jour-054-empty");
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_thread_jsonl_report(&dir.join("main.jsonl"), "nope").expect("report"),
            "{\"messages\":0,\"thread\":\"nope\"}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_kind_counts_jsonl_seeded_yields_one_valid_line_per_kind() {
        let dir = temp_dir("jour-037");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        let out = journal_kind_counts_jsonl(&dir.join("main.jsonl")).expect("jsonl");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        // count-desc order; each line is valid compact JSON {kind, count}.
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("line0 json");
        assert_eq!(first["kind"], "message_persisted");
        assert_eq!(first["count"], 3);
        let second: serde_json::Value = serde_json::from_str(lines[1]).expect("line1 json");
        assert_eq!(second["kind"], "turn_started");
        assert_eq!(second["count"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_kind_counts_jsonl_empty_segment_is_empty_string() {
        let dir = temp_dir("jour-037-empty");
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_kind_counts_jsonl(&dir.join("main.jsonl")).expect("jsonl"),
            ""
        );
        let _ = std::fs::remove_dir_all(&dir);
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
    fn checkpoint_draft_seeded_view_has_correct_counts() {
        let dir = temp_dir("checkpoint");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "hello"}),
            );
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::TurnStarted, Some("t2"), json!({}));
        }
        let view = ThreadsView::fold(&dir.join("main.jsonl")).expect("fold");
        let draft = checkpoint_draft(&view);
        assert_eq!(
            draft.kind,
            JournalKind::Other("checkpoint_created".to_string())
        );
        assert_eq!(draft.thread_id, None);
        assert_eq!(draft.payload["threads"], 2);
        assert_eq!(draft.payload["messages"], 2); // t1=2 messages, t2=0
        assert!(draft.payload["ts"].as_u64().expect("ts") > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_draft_empty_view_is_zeros() {
        let draft = checkpoint_draft(&ThreadsView::default());
        assert_eq!(draft.payload["threads"], 0);
        assert_eq!(draft.payload["messages"], 0);
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
    fn task_timeline_returns_ordered_status_history() {
        let dir = temp_dir("timeline");
        TaskStore::create(&dir, "t").expect("create");
        TaskStore::transition(&dir, "t", TaskStatus::Running).expect("running");
        TaskStore::transition(&dir, "t", TaskStatus::Done).expect("done");

        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        let view = TasksView::fold(&path).expect("fold");
        // Expected timestamps come from the same records the fold saw.
        let stamps: Vec<Option<u128>> = Journal::replay(&path)
            .expect("replay")
            .iter()
            .map(|r| Some(r.ts_ms))
            .collect();
        assert_eq!(
            task_timeline(&view, "t"),
            vec![
                ("pending".to_string(), stamps[0]),
                ("running".to_string(), stamps[1]),
                ("done".to_string(), stamps[2]),
            ]
        );

        // Interleaved tasks stay independent; unknown ids are empty.
        TaskStore::transition(&dir, "other", TaskStatus::Failed).expect("failed");
        let view = TasksView::fold(&path).expect("refold");
        assert_eq!(task_timeline(&view, "other").len(), 1);
        assert!(task_timeline(&view, "unknown").is_empty());
        assert!(task_timeline(&TasksView::default(), "any").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_timeline_json_lists_seeded_entries_and_unknown_is_empty_array() {
        let dir = temp_dir("timeline-json");
        TaskStore::create(&dir, "t").expect("create");
        TaskStore::transition(&dir, "t", TaskStatus::Done).expect("done");

        let path = dir.join(format!("{TASKS_SEGMENT}.jsonl"));
        let view = TasksView::fold(&path).expect("fold");
        let stamps: Vec<u128> = Journal::replay(&path)
            .expect("replay")
            .iter()
            .map(|r| r.ts_ms)
            .collect();

        let text = task_timeline_json(&view, "t");
        assert!(text.contains('\n'), "pretty-printed: {text}");
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&text).expect("parses");
        assert_eq!(
            parsed,
            vec![
                serde_json::json!({ "status": "pending", "ts": stamps[0] }),
                serde_json::json!({ "status": "done", "ts": stamps[1] }),
            ]
        );

        assert_eq!(task_timeline_json(&view, "unknown"), "[]");
        assert_eq!(task_timeline_json(&TasksView::default(), "any"), "[]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_state_events_lists_every_task_sorted_by_id() {
        let mut view = TasksView::default();
        for (id, status) in [
            ("b", TaskStatus::Done),
            ("a", TaskStatus::Running),
            ("c", TaskStatus::Failed),
        ] {
            view.tasks.insert(
                id.into(),
                TaskRecord {
                    id: id.into(),
                    status,
                    deps: vec![],
                },
            );
        }
        assert_eq!(
            task_state_events(&view),
            vec![
                ("a".to_string(), "running".to_string()),
                ("b".to_string(), "done".to_string()),
                ("c".to_string(), "failed".to_string()),
            ]
        );
    }

    #[test]
    fn task_state_export_of_empty_view_is_empty_and_bare_array() {
        let view = TasksView::default();
        assert!(task_state_events(&view).is_empty());
        assert_eq!(task_state_json(&view), "[]");
    }

    #[test]
    fn task_state_json_is_pretty_and_parses_back() {
        let mut view = TasksView::default();
        view.tasks.insert(
            "t-1".into(),
            TaskRecord {
                id: "t-1".into(),
                status: TaskStatus::Pending,
                deps: vec![],
            },
        );
        let text = task_state_json(&view);
        assert!(text.contains('\n'), "pretty-printed");
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&text).expect("parses");
        assert_eq!(parsed, vec![json!({ "id": "t-1", "status": "pending" })]);
    }

    #[test]
    fn tasks_export_json_lists_sorted_rows_with_timeline_len() {
        let mut view = TasksView::default();
        for (id, status, timeline) in [
            (
                "b",
                TaskStatus::Done,
                vec![
                    ("running".to_string(), Some(2)),
                    ("done".to_string(), Some(3)),
                ],
            ),
            (
                "a",
                TaskStatus::Pending,
                vec![("pending".to_string(), Some(1))],
            ),
        ] {
            view.tasks.insert(
                id.into(),
                TaskRecord {
                    id: id.into(),
                    status,
                    deps: vec![],
                },
            );
            view.timeline.insert(id.into(), timeline);
        }
        let text = tasks_export_json(&view);
        assert!(text.contains('\n'), "pretty-printed");
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&text).expect("parses");
        assert_eq!(
            parsed,
            vec![
                json!({ "id": "a", "status": "pending", "timeline_len": 1 }),
                json!({ "id": "b", "status": "done", "timeline_len": 2 }),
            ]
        );
    }

    #[test]
    fn tasks_export_of_empty_view_is_bare_array() {
        assert_eq!(tasks_export_json(&TasksView::default()), "[]");
    }

    #[test]
    fn tasks_export_jsonl_lists_sorted_compact_lines() {
        let mut view = TasksView::default();
        for (id, status) in [
            ("b", TaskStatus::Done),
            ("a", TaskStatus::Running),
            ("c", TaskStatus::Failed),
        ] {
            view.tasks.insert(
                id.into(),
                TaskRecord {
                    id: id.into(),
                    status,
                    deps: vec![],
                },
            );
        }
        let text = tasks_export_jsonl(&view);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| !l.contains('\n')));
        let parsed: Vec<serde_json::Value> = lines
            .iter()
            .map(|l| serde_json::from_str(l).expect("line parses"))
            .collect();
        assert_eq!(
            parsed,
            vec![
                json!({ "id": "a", "status": "running" }),
                json!({ "id": "b", "status": "done" }),
                json!({ "id": "c", "status": "failed" }),
            ]
        );
    }

    #[test]
    fn tasks_export_jsonl_of_empty_view_is_empty_string() {
        assert_eq!(tasks_export_jsonl(&TasksView::default()), "");
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

    // orch-022 ---------------------------------------------------------------

    #[test]
    fn tasks_by_status_filters_seeded_ids_and_unknown_status_is_empty() {
        let mut view = TasksView::default();
        for (id, status) in [
            ("t-done", TaskStatus::Done),
            ("b-pending", TaskStatus::Pending),
            ("a-pending", TaskStatus::Pending),
            ("run", TaskStatus::Running),
            ("boom", TaskStatus::Failed),
        ] {
            view.tasks.insert(
                id.into(),
                TaskRecord {
                    id: id.into(),
                    status,
                    deps: vec![],
                },
            );
        }
        assert_eq!(
            tasks_by_status(&view, "pending"),
            vec!["a-pending".to_string(), "b-pending".to_string()]
        );
        assert_eq!(tasks_by_status(&view, "done"), vec!["t-done".to_string()]);
        assert_eq!(tasks_by_status(&view, "running"), vec!["run".to_string()]);
        assert_eq!(tasks_by_status(&view, "failed"), vec!["boom".to_string()]);
        assert!(tasks_by_status(&view, "archived").is_empty());
        assert!(tasks_by_status(&TasksView::default(), "done").is_empty());
    }

    // orch-027 ---------------------------------------------------------------

    #[test]
    fn tasks_by_status_json_lists_sorted_ids_and_unknown_status_is_bare_array() {
        let mut view = TasksView::default();
        for (id, status) in [
            ("b-pending", TaskStatus::Pending),
            ("a-pending", TaskStatus::Pending),
            ("run", TaskStatus::Running),
        ] {
            view.tasks.insert(
                id.into(),
                TaskRecord {
                    id: id.into(),
                    status,
                    deps: vec![],
                },
            );
        }
        let text = tasks_by_status_json(&view, "pending");
        assert!(text.contains('\n'), "pretty-printed");
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&text).expect("parses");
        assert_eq!(parsed, vec![json!("a-pending"), json!("b-pending")]);
        assert_eq!(
            tasks_by_status_json(&TasksView::default(), "archived"),
            "[]"
        );
        assert_eq!(tasks_by_status_json(&TasksView::default(), "done"), "[]");
    }

    // orch-021 ---------------------------------------------------------------

    #[test]
    fn task_health_line_seeded_matches_counts() {
        let dir = temp_dir("orch021-seeded");
        TaskStore::create(&dir, "d").expect("create d");
        TaskStore::create(&dir, "r").expect("create r");
        TaskStore::create(&dir, "f").expect("create f");
        TaskStore::create(&dir, "p").expect("create p");
        TaskStore::transition(&dir, "d", TaskStatus::Done).expect("d done");
        TaskStore::transition(&dir, "r", TaskStatus::Running).expect("r running");
        TaskStore::transition(&dir, "f", TaskStatus::Failed).expect("f failed");

        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        assert_eq!(
            task_health_line(&view),
            "4 tasks: 1 done, 1 running, 1 failed, 1 pending"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_health_line_empty_is_all_zeros() {
        let dir = temp_dir("orch021-empty");
        {
            Journal::open(&dir, TASKS_SEGMENT).expect("open");
        }
        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        assert_eq!(
            task_health_line(&view),
            "0 tasks: 0 done, 0 running, 0 failed, 0 pending"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // orch-023 ---------------------------------------------------------------

    #[test]
    fn task_status_summary_json_seeded_matches_counts() {
        let dir = temp_dir("orch023-seeded");
        TaskStore::create(&dir, "d").expect("create d");
        TaskStore::create(&dir, "r").expect("create r");
        TaskStore::create(&dir, "f").expect("create f");
        TaskStore::create(&dir, "p").expect("create p");
        TaskStore::transition(&dir, "d", TaskStatus::Done).expect("d done");
        TaskStore::transition(&dir, "r", TaskStatus::Running).expect("r running");
        TaskStore::transition(&dir, "f", TaskStatus::Failed).expect("f failed");

        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        let parsed: Value = serde_json::from_str(&task_status_summary_json(&view)).expect("json");
        assert_eq!(parsed["total"], 4);
        assert_eq!(parsed["done"], 1);
        assert_eq!(parsed["running"], 1);
        assert_eq!(parsed["failed"], 1);
        assert_eq!(parsed["pending"], 1);
        assert!(task_status_summary_json(&view).contains('\n'), "pretty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_status_summary_json_empty_is_all_zeros() {
        let dir = temp_dir("orch023-empty");
        {
            Journal::open(&dir, TASKS_SEGMENT).expect("open");
        }
        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        let parsed: Value = serde_json::from_str(&task_status_summary_json(&view)).expect("json");
        for key in ["total", "done", "running", "failed", "pending"] {
            assert_eq!(parsed[key], 0, "{key}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // orch-028 ---------------------------------------------------------------

    #[test]
    fn task_counts_json_seeded_matches_counts_exactly() {
        let dir = temp_dir("orch028-seeded");
        TaskStore::create(&dir, "d").expect("create d");
        TaskStore::create(&dir, "r").expect("create r");
        TaskStore::create(&dir, "f").expect("create f");
        TaskStore::create(&dir, "p").expect("create p");
        TaskStore::transition(&dir, "d", TaskStatus::Done).expect("d done");
        TaskStore::transition(&dir, "r", TaskStatus::Running).expect("r running");
        TaskStore::transition(&dir, "f", TaskStatus::Failed).expect("f failed");

        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        // serde_json's json! orders object keys alphabetically; no `total` key.
        assert_eq!(
            task_counts_json(&view),
            "{\n  \"done\": 1,\n  \"failed\": 1,\n  \"pending\": 1,\n  \"running\": 1\n}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_counts_json_empty_is_all_zeros() {
        assert_eq!(
            task_counts_json(&TasksView::default()),
            "{\n  \"done\": 0,\n  \"failed\": 0,\n  \"pending\": 0,\n  \"running\": 0\n}"
        );
    }

    // orch-029 ---------------------------------------------------------------

    #[test]
    fn task_status_counts_jsonl_seeded_is_single_compact_line() {
        let dir = temp_dir("orch029-seeded");
        TaskStore::create(&dir, "d").expect("create d");
        TaskStore::create(&dir, "r").expect("create r");
        TaskStore::create(&dir, "f").expect("create f");
        TaskStore::create(&dir, "p").expect("create p");
        TaskStore::transition(&dir, "d", TaskStatus::Done).expect("d done");
        TaskStore::transition(&dir, "r", TaskStatus::Running).expect("r running");
        TaskStore::transition(&dir, "f", TaskStatus::Failed).expect("f failed");

        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        // serde_json's json! orders object keys alphabetically; single line.
        assert_eq!(
            task_status_counts_jsonl(&view),
            "{\"done\":1,\"failed\":1,\"pending\":1,\"running\":1}"
        );
        assert!(!task_status_counts_jsonl(&view).contains('\n'), "one line");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn task_status_counts_jsonl_empty_is_all_zeros() {
        assert_eq!(
            task_status_counts_jsonl(&TasksView::default()),
            "{\"done\":0,\"failed\":0,\"pending\":0,\"running\":0}"
        );
    }

    // orch-024 ---------------------------------------------------------------

    #[test]
    fn tasks_health_json_seeded_matches_counts_and_timeline_exactly() {
        let dir = temp_dir("orch024-seeded");
        TaskStore::create(&dir, "d").expect("create d");
        TaskStore::create(&dir, "r").expect("create r");
        TaskStore::create(&dir, "f").expect("create f");
        TaskStore::create(&dir, "p").expect("create p");
        TaskStore::transition(&dir, "d", TaskStatus::Done).expect("d done");
        TaskStore::transition(&dir, "r", TaskStatus::Running).expect("r running");
        TaskStore::transition(&dir, "f", TaskStatus::Failed).expect("f failed");

        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        // 4 creates + 3 transitions = 7 folded timeline events.
        // serde_json's json! orders object keys alphabetically.
        assert_eq!(
            tasks_health_json(&view),
            "{\n  \"counts\": {\n    \"done\": 1,\n    \"failed\": 1,\n    \"pending\": 1,\n    \"running\": 1,\n    \"total\": 4\n  },\n  \"events\": 7\n}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tasks_health_json_empty_is_all_zeros() {
        assert_eq!(
            tasks_health_json(&TasksView::default()),
            "{\n  \"counts\": {\n    \"done\": 0,\n    \"failed\": 0,\n    \"pending\": 0,\n    \"running\": 0,\n    \"total\": 0\n  },\n  \"events\": 0\n}"
        );
    }

    // ctx-013 ---------------------------------------------------------------

    #[test]
    fn never_compact_invariant_holds_on_seeded_journal() {
        let dir = temp_dir("ctx013-seeded");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"role": "user", "text": "first request"}),
            );
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"role": "agent", "text": "answer one"}),
            );
            // Runtime-shaped payload (`last_role`) as the FINAL user message;
            // a trailing agent reply must not displace it.
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"last_role": "user", "text": "final user ask — must survive"}),
            );
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"role": "agent", "text": "trailing agent reply"}),
            );
        }
        let path = dir.join("main.jsonl");
        assert!(
            never_compact_latest_user_invariant(&path).expect("invariant check"),
            "latest user text must survive both folds byte-identical"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn never_compact_invariant_true_on_empty_journal() {
        let dir = temp_dir("ctx013-empty");
        {
            // Create the file with no records (drop flushes/closes it).
            Journal::open(&dir, "main").expect("open");
        }
        assert!(never_compact_latest_user_invariant(&dir.join("main.jsonl")).expect("check"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ctx-014 ---------------------------------------------------------------

    /// Like [`seed_record_at`] but with controlled thread id and message
    /// text, so per-thread item lists can be asserted exactly.
    fn seed_thread_message(dir: &Path, seq: u64, thread: &str, text: &str, ts_ms: u128) {
        use std::io::Write as _;
        let record = Record {
            seq,
            ts_ms,
            kind: JournalKind::MessagePersisted,
            thread_id: Some(thread.into()),
            payload: json!({ "text": text }),
        };
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("main.jsonl"))
            .expect("open segment");
        writeln!(f, "{}", serde_json::to_string(&record).expect("serialize")).expect("write");
    }

    #[test]
    fn load_session_items_returns_exact_per_thread_lists_in_journal_order() {
        let dir = temp_dir("ctx014-seeded");
        seed_thread_message(&dir, 1, "tA", "a one", MSG_OLD_MS);
        seed_thread_message(&dir, 2, "tB", "b one", MSG_OLD_MS + 1);
        // Non-message kinds never become session items.
        seed_record_at(&dir, 3, MSG_OLD_MS + 2, JournalKind::TurnStarted);
        seed_thread_message(&dir, 4, "tA", "a two", MSG_NEW_MS);
        assert_eq!(
            load_session_items(&dir.join("main.jsonl")).expect("fold"),
            vec![
                (
                    "tA".to_string(),
                    vec![
                        ("a one".to_string(), MSG_OLD_MS),
                        ("a two".to_string(), MSG_NEW_MS),
                    ]
                ),
                ("tB".to_string(), vec![("b one".to_string(), MSG_OLD_MS + 1)]),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_session_items_on_empty_journal_is_empty() {
        let dir = temp_dir("ctx014-empty");
        {
            // Create the file with no records (drop flushes/closes it).
            Journal::open(&dir, "main").expect("open");
        }
        assert!(
            load_session_items(&dir.join("main.jsonl"))
                .expect("fold")
                .is_empty()
        );
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

    // ctx-006: oldest_message_age_ms ------------------------------------------

    const MSG_OLD_MS: u128 = 1_700_000_000_000;
    const MSG_NEW_MS: u128 = 1_710_000_000_000;

    /// Like [`seed_record_at`] but with a controlled thread_id, so messages
    /// can be spread across threads.
    fn seed_message_at(dir: &Path, seq: u64, thread: &str, ts_ms: u128) {
        use std::io::Write as _;
        let record = Record {
            seq,
            ts_ms,
            kind: JournalKind::MessagePersisted,
            thread_id: Some(thread.into()),
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
    fn oldest_message_age_ms_takes_min_across_threads_and_ignores_other_kinds() {
        let dir = temp_dir("oldest-age-multi");
        seed_message_at(&dir, 1, "tA", MSG_NEW_MS);
        seed_message_at(&dir, 2, "tB", MSG_OLD_MS);
        // Non-message kinds never count toward freshness; tB holds the oldest.
        seed_record_at(&dir, 3, MSG_OLD_MS - 60_000, JournalKind::TurnStarted);
        assert_eq!(
            oldest_message_age_ms(&dir.join("main.jsonl"), MSG_NEW_MS + 1_234).expect("fold"),
            Some(MSG_NEW_MS + 1_234 - MSG_OLD_MS)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oldest_message_age_ms_on_empty_journal_is_none() {
        let dir = temp_dir("oldest-age-empty");
        {
            // Create the file with no records (drop flushes/closes it).
            Journal::open(&dir, "main").expect("open");
        }
        assert_eq!(
            oldest_message_age_ms(&dir.join("main.jsonl"), MSG_NEW_MS).expect("fold"),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oldest_message_age_ms_single_message_is_its_age() {
        let dir = temp_dir("oldest-age-single");
        seed_message_at(&dir, 1, "t1", MSG_OLD_MS);
        assert_eq!(
            oldest_message_age_ms(&dir.join("main.jsonl"), MSG_OLD_MS + 5_000).expect("fold"),
            Some(5_000)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-017 ---------------------------------------------------------------

    #[test]
    fn write_fixture_record_count_shape_and_replay() {
        let dir = temp_dir("fixture-count");
        let path = dir.join("fixture.jsonl");
        let count = write_fixture(&path, 3, 4).expect("write");
        assert_eq!(count, 3 * (1 + 4));

        // Replay succeeds; seqs are sequential from 1.
        let records = Journal::replay(&path).expect("replay");
        assert_eq!(records.len(), count);
        assert_eq!(
            records.iter().map(|r| r.seq).collect::<Vec<_>>(),
            (1..=count as u64).collect::<Vec<_>>()
        );
        // Shape: each turn is TurnStarted followed by exactly msgs_per_turn
        // MessagePersisted records sharing one thread id.
        assert_eq!(records[0].kind, JournalKind::TurnStarted);
        assert_eq!(records[1].kind, JournalKind::MessagePersisted);
        for turn in 0..3 {
            let start = turn * (1 + 4);
            assert_eq!(records[start].kind, JournalKind::TurnStarted);
            assert_eq!(
                records[start].thread_id,
                Some(format!("fixture-turn-{}", turn + 1))
            );
            for msg in 1..=4 {
                let record = &records[start + msg];
                assert_eq!(record.kind, JournalKind::MessagePersisted);
                assert_eq!(record.thread_id, records[start].thread_id);
                assert_eq!(
                    record.payload["text"],
                    format!("fixture message {}-{msg}", turn + 1)
                );
            }
        }
        // Views fold it like any real segment.
        let usage = usage_fold(&path).expect("usage fold");
        assert_eq!(usage.turns_started, 3);
        assert_eq!(usage.messages_persisted, 12);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_fixture_same_params_are_byte_identical() {
        let dir = temp_dir("fixture-determinism");
        let a = dir.join("a.jsonl");
        let b = dir.join("b.jsonl");
        write_fixture(&a, 2, 3).expect("write a");
        write_fixture(&b, 2, 3).expect("write b");
        assert_eq!(
            std::fs::read(&a).expect("read a"),
            std::fs::read(&b).expect("read b")
        );
        // Different params produce different bytes.
        write_fixture(&a, 2, 4).expect("write a2");
        assert_ne!(
            std::fs::read(&a).expect("read a2"),
            std::fs::read(&b).expect("read b")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_fixture_zero_turns_is_empty_and_replays() {
        let dir = temp_dir("fixture-empty");
        let path = dir.join("empty-fixture.jsonl");
        assert_eq!(write_fixture(&path, 0, 5).expect("write"), 0);
        assert_eq!(Journal::replay(&path).expect("replay").len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// jour-019 perf smoke: a 10k-record fixture must replay well under 2s.
    #[test]
    fn replay_perf_smoke_10k_records_under_two_seconds() {
        let dir = temp_dir("perf-smoke");
        let path = dir.join("perf.jsonl");
        // 1000 turns * (1 + 9) = 10_000 records.
        let expected = write_fixture(&path, 1_000, 9).expect("write fixture");

        let (count, ms) = replay_perf_smoke(&path).expect("perf smoke");
        assert_eq!(count, expected);
        assert!(
            ms < 2_000,
            "replay of {count} records took {ms}ms (budget: <2000ms)"
        );
        let recs_per_sec = count as f64 / (ms.max(1) as f64 / 1000.0);
        println!("jour-019 replay perf: {count} records in {ms}ms ({recs_per_sec:.0} records/sec)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// jour-022: a view folded straight off the journal never drifts.
    #[test]
    fn task_state_journal_events_no_drift_is_empty() {
        let dir = temp_dir("jour-022-clean");
        {
            let mut j = Journal::open(&dir, TASKS_SEGMENT).expect("open");
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "t1", "status": "pending"}),
            );
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "t1", "status": "running"}),
            );
        }
        let view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        assert!(task_state_journal_events(&view).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// jour-022: out-of-band knowledge that a task moved on yields exactly one
    /// shape-only draft for it; untouched tasks stay silent.
    #[test]
    fn task_state_journal_events_emits_one_draft_per_stale_task() {
        let dir = temp_dir("jour-022-stale");
        {
            let mut j = Journal::open(&dir, TASKS_SEGMENT).expect("open");
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "t1", "status": "pending"}),
            );
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "t1", "status": "running"}),
            );
            append(
                &mut j,
                JournalKind::TaskStateChanged,
                None,
                json!({"id": "t2", "status": "done"}),
            );
        }
        let mut view = TasksView::fold(&dir.join(format!("{TASKS_SEGMENT}.jsonl"))).expect("fold");
        // Orchestrator/snapshot knows t1 finished since the journal last spoke.
        view.tasks.get_mut("t1").expect("t1").status = TaskStatus::Done;

        let drafts = task_state_journal_events(&view);
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].kind, JournalKind::TaskStateChanged);
        assert_eq!(drafts[0].thread_id, None);
        assert_eq!(
            drafts[0].payload,
            json!({"id": "t1", "status": TaskStatus::Done})
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_size_report_seeded_matches_real_bytes_and_count() {
        let dir = temp_dir("jour-024");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let path = dir.join("main.jsonl");
        let bytes = std::fs::metadata(&path).expect("metadata").len();
        let pct = bytes * 100 / (10 * 1024 * 1024);
        assert_eq!(
            journal_size_report(&path).expect("report"),
            format!("3 records, {bytes} bytes ({pct}% of 10MB cap)")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_size_report_missing_file_is_err() {
        let dir = temp_dir("jour-024-missing");
        assert!(journal_size_report(&dir.join("nope.jsonl")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_health_json_seeded_has_exact_fields() {
        let dir = temp_dir("jour-031");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let path = dir.join("main.jsonl");
        let bytes = std::fs::metadata(&path).expect("metadata").len();
        // json! uses a BTreeMap → keys serialize in sorted order.
        assert_eq!(
            journal_health_json(&path).expect("json"),
            format!(
                "{{\n  \"bytes\": {bytes},\n  \"gaps\": 0,\n  \"last_seq\": 3,\n  \"pct_of_cap\": {},\n  \"records\": 3\n}}",
                bytes * 100 / (10 * 1024 * 1024)
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_health_json_missing_file_is_err() {
        let dir = temp_dir("jour-031-missing");
        assert!(journal_health_json(&dir.join("nope.jsonl")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-040 ---------------------------------------------------------------

    #[test]
    fn journal_size_json_seeded_has_exact_fields() {
        let dir = temp_dir("jour-040");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let path = dir.join("main.jsonl");
        let bytes = std::fs::metadata(&path).expect("metadata").len();
        // json! uses a BTreeMap → keys serialize in sorted order.
        assert_eq!(
            journal_size_json(&path).expect("json"),
            format!(
                "{{\n  \"bytes\": {bytes},\n  \"pct_of_cap\": {},\n  \"records\": 3\n}}",
                bytes * 100 / (10 * 1024 * 1024)
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_size_json_missing_file_is_err() {
        let dir = temp_dir("jour-040-missing");
        assert!(journal_size_json(&dir.join("nope.jsonl")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-041 ---------------------------------------------------------------

    #[test]
    fn journal_gaps_json_clean_journal_has_zero_gaps() {
        let dir = temp_dir("jour-041");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let path = dir.join("main.jsonl");
        // json! uses a BTreeMap → keys serialize in sorted order.
        assert_eq!(
            journal_gaps_json(&path).expect("json"),
            "{\n  \"gaps\": 0,\n  \"records\": 3\n}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_gaps_json_gapped_fixture_counts_missing_seqs_exactly() {
        let dir = temp_dir("jour-041-gapped");
        let path = dir.join("main.jsonl");
        // Hand-written records with a seq jump 2 -> 5 (seqs 3, 4 missing).
        let lines = [1u64, 2, 5]
            .map(|seq| {
                serde_json::to_string(&Record {
                    seq,
                    ts_ms: 1_770_000_000_000,
                    kind: JournalKind::TurnStarted,
                    thread_id: None,
                    payload: json!({}),
                })
                .expect("serialize")
            })
            .join("\n");
        std::fs::write(&path, lines).expect("write fixture");
        assert_eq!(
            journal_gaps_json(&path).expect("json"),
            "{\n  \"gaps\": 2,\n  \"records\": 3\n}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-042 ---------------------------------------------------------------

    #[test]
    fn journal_gaps_jsonl_clean_journal_is_single_zero_line() {
        let dir = temp_dir("jour-042");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let path = dir.join("main.jsonl");
        // json! uses a BTreeMap → keys serialize in sorted order.
        assert_eq!(
            journal_gaps_jsonl(&path).expect("jsonl"),
            "{\"gaps\":0,\"records\":3}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_gaps_jsonl_gapped_fixture_counts_missing_seqs_exactly() {
        let dir = temp_dir("jour-042-gapped");
        let path = dir.join("main.jsonl");
        // Hand-written records with a seq jump 2 -> 5 (seqs 3, 4 missing).
        let lines = [1u64, 2, 5]
            .map(|seq| {
                serde_json::to_string(&Record {
                    seq,
                    ts_ms: 1_770_000_000_000,
                    kind: JournalKind::TurnStarted,
                    thread_id: None,
                    payload: json!({}),
                })
                .expect("serialize")
            })
            .join("\n");
        std::fs::write(&path, lines).expect("write fixture");
        assert_eq!(
            journal_gaps_jsonl(&path).expect("jsonl"),
            "{\"gaps\":2,\"records\":3}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-047 ---------------------------------------------------------------

    #[test]
    fn journal_health_jsonl_seeded_is_exact_line() {
        let dir = temp_dir("jour-047");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let path = dir.join("main.jsonl");
        let bytes = std::fs::metadata(&path).expect("metadata").len();
        // json! uses a BTreeMap → keys serialize in sorted order.
        assert_eq!(
            journal_health_jsonl(&path).expect("jsonl"),
            format!(
                "{{\"bytes\":{bytes},\"gaps\":0,\"pct_of_cap\":{},\"records\":3}}",
                bytes * 100 / (10 * 1024 * 1024)
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_health_jsonl_missing_file_is_err() {
        let dir = temp_dir("jour-047-missing");
        assert!(journal_health_jsonl(&dir.join("nope.jsonl")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn seq_health_clean_journal_reports_zero_gaps() {
        let dir = temp_dir("jour-023");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        assert_eq!(
            seq_health(&dir.join("main.jsonl")).expect("report"),
            "3 records, 0 gaps, last_seq 3"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_last_seq_seeded_returns_some() {
        let dir = temp_dir("jour-043");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        assert_eq!(
            journal_last_seq(&dir.join("main.jsonl")).expect("seq"),
            Some(2)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_last_seq_empty_segment_is_none() {
        let dir = temp_dir("jour-043-empty");
        // Create the segment file with no records (drop flushes/closes it).
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_last_seq(&dir.join("main.jsonl")).expect("seq"),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-044 ---------------------------------------------------------------

    #[test]
    fn journal_record_count_seeded_returns_exact_count() {
        let dir = temp_dir("jour-044");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        assert_eq!(
            journal_record_count(&dir.join("main.jsonl")).expect("count"),
            3
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_record_count_empty_segment_is_zero() {
        let dir = temp_dir("jour-044-empty");
        // Create the segment file with no records (drop flushes/closes it).
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_record_count(&dir.join("main.jsonl")).expect("count"),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-045 ---------------------------------------------------------------

    #[test]
    fn journal_has_gaps_clean_journal_is_false() {
        let dir = temp_dir("jour-045");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        assert_eq!(
            journal_has_gaps(&dir.join("main.jsonl")).expect("gaps"),
            false
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_has_gaps_gapped_fixture_is_true() {
        let dir = temp_dir("jour-045-gapped");
        let path = dir.join("main.jsonl");
        // Hand-written records with a seq jump 2 -> 5 (seqs 3, 4 missing).
        let lines = [1u64, 2, 5]
            .map(|seq| {
                serde_json::to_string(&Record {
                    seq,
                    ts_ms: 1_770_000_000_000,
                    kind: JournalKind::TurnStarted,
                    thread_id: None,
                    payload: json!({}),
                })
                .expect("serialize")
            })
            .join("\n");
        std::fs::write(&path, lines).expect("write fixture");
        assert_eq!(journal_has_gaps(&path).expect("gaps"), true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-046 ---------------------------------------------------------------

    #[test]
    fn journal_is_empty_seeded_is_false() {
        let dir = temp_dir("jour-046");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        assert_eq!(
            journal_is_empty(&dir.join("main.jsonl")).expect("empty"),
            false
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_is_empty_empty_segment_is_true() {
        let dir = temp_dir("jour-046-empty");
        // Create the segment file with no records (drop flushes/closes it).
        let _ = Journal::open(&dir, "main").expect("open");
        assert_eq!(
            journal_is_empty(&dir.join("main.jsonl")).expect("empty"),
            true
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn seq_health_gapped_fixture_counts_missing_seqs() {
        let dir = temp_dir("jour-023-gapped");
        let path = dir.join("main.jsonl");
        // Hand-written records with a seq jump 2 -> 5 (seqs 3, 4 missing).
        let lines = [1u64, 2, 5]
            .map(|seq| {
                serde_json::to_string(&Record {
                    seq,
                    ts_ms: 1_770_000_000_000,
                    kind: JournalKind::TurnStarted,
                    thread_id: None,
                    payload: json!({}),
                })
                .expect("serialize")
            })
            .join("\n");
        std::fs::write(&path, lines).expect("write fixture");
        assert_eq!(
            seq_health(&path).expect("report"),
            "3 records, 2 gaps, last_seq 5"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_health_line_seeded_contains_both_halves() {
        let dir = temp_dir("jour-025");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let line = journal_health_line(&dir.join("main.jsonl")).expect("line");
        let halves: Vec<&str> = line.split(" | ").collect();
        assert_eq!(halves.len(), 2);
        assert_eq!(halves[0], "2 records, 0 gaps, last_seq 2");
        assert!(halves[1].starts_with("2 records, ") && halves[1].contains(" bytes ("));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_health_line_missing_file_is_err() {
        let dir = temp_dir("jour-025-missing");
        assert!(journal_health_line(&dir.join("nope.jsonl")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_export_json_seeded_has_metadata_no_payload() {
        let dir = temp_dir("jour-026");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(
                &mut j,
                JournalKind::TurnStarted,
                Some("t1"),
                json!({"x": 1}),
            );
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let out = journal_export_json(&dir.join("main.jsonl")).expect("export");
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid json array");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["seq"], 1);
        assert_eq!(parsed[0]["kind"], "turn_started");
        assert_eq!(parsed[0]["thread_id"], "t1");
        assert!(parsed[0]["ts_ms"].is_u64());
        // Metadata-only: payloads must not leak into the export.
        assert!(parsed[0].get("payload").is_none());
        assert_eq!(parsed[1]["kind"], "message_persisted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_export_json_empty_segment_is_empty_array() {
        let dir = temp_dir("jour-026-empty");
        Journal::open(&dir, "main").expect("open"); // creates an empty segment
        assert_eq!(
            journal_export_json(&dir.join("main.jsonl")).expect("export"),
            "[]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_export_jsonl_seeded_one_valid_json_line_per_record() {
        let dir = temp_dir("jour-030");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(
                &mut j,
                JournalKind::TurnStarted,
                Some("t1"),
                json!({"x": 1}),
            );
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let out = journal_export_jsonl(&dir.join("main.jsonl")).expect("export");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        // Compact per-line JSON: each line parses standalone; no pretty array.
        for (i, line) in lines.iter().enumerate() {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("each line is valid json");
            assert_eq!(parsed["seq"], i as u64 + 1);
            assert!(parsed.get("payload").is_none());
        }
        // Compact: no whitespace after ':' or ','.
        assert!(lines[0]
            .starts_with("{\"seq\":1,\"kind\":\"turn_started\",\"thread_id\":\"t1\",\"ts_ms\":"));
        assert!(lines[0].ends_with('}'));
        assert!(!out.ends_with('\n')); // no trailing newline
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_export_jsonl_empty_segment_is_empty_string() {
        let dir = temp_dir("jour-030-empty");
        Journal::open(&dir, "main").expect("open"); // creates an empty segment
        assert_eq!(
            journal_export_jsonl(&dir.join("main.jsonl")).expect("export"),
            ""
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-028 ---------------------------------------------------------------

    #[test]
    fn thread_message_counts_seeded_multi_thread_sorted_desc() {
        let dir = temp_dir("jour-028");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            // t2: 3 messages, t3: 2, t1: 1. TurnStarted records register a
            // thread but never count as messages.
            append(&mut j, JournalKind::TurnStarted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::TurnStarted, Some("t3"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t3"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t3"), json!({}));
        }
        assert_eq!(
            thread_message_counts(&dir.join("main.jsonl")).expect("fold"),
            vec![
                ("t2".to_string(), 3),
                ("t3".to_string(), 2),
                ("t1".to_string(), 1),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thread_message_counts_empty_journal_is_empty_vec() {
        let dir = temp_dir("jour-028-empty");
        Journal::open(&dir, "main").expect("open"); // creates an empty segment
        assert!(
            thread_message_counts(&dir.join("main.jsonl"))
                .expect("fold")
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-029 ---------------------------------------------------------------

    #[test]
    fn journal_top_threads_seeded_returns_top_n_in_order() {
        let dir = temp_dir("jour-029-top");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            for _ in 0..3 {
                append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            }
            for _ in 0..2 {
                append(&mut j, JournalKind::MessagePersisted, Some("t3"), json!({}));
            }
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        assert_eq!(
            journal_top_threads(&dir.join("main.jsonl"), 2).expect("fold"),
            vec![("t2".to_string(), 3), ("t3".to_string(), 2)]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_top_threads_zero_is_empty() {
        let dir = temp_dir("jour-029-zero");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        assert!(journal_top_threads(&dir.join("main.jsonl"), 0)
            .expect("fold")
            .is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_top_threads_n_over_total_returns_all() {
        let dir = temp_dir("jour-029-all");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
        }
        assert_eq!(
            journal_top_threads(&dir.join("main.jsonl"), 99).expect("fold"),
            vec![("t1".to_string(), 1), ("t2".to_string(), 1)]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-033 ---------------------------------------------------------------

    #[test]
    fn journal_top_threads_json_seeded_is_exact_pretty_json() {
        let dir = temp_dir("jour033-seeded");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        assert_eq!(
            journal_top_threads_json(&dir.join("main.jsonl"), 10).expect("json"),
            "[\n  {\n    \"thread\": \"t2\",\n    \"messages\": 2\n  },\n  {\n    \"thread\": \"t1\",\n    \"messages\": 1\n  }\n]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_top_threads_json_empty_yields_empty_array() {
        let dir = temp_dir("jour033-empty");
        Journal::open(&dir, "main").expect("open"); // creates an empty segment
        assert_eq!(
            journal_top_threads_json(&dir.join("main.jsonl"), 5).expect("json"),
            "[]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-035 ---------------------------------------------------------------

    #[test]
    fn journal_thread_counts_json_seeded_is_exact_pretty_json() {
        let dir = temp_dir("jour035-seeded");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        assert_eq!(
            journal_thread_counts_json(&dir.join("main.jsonl")).expect("json"),
            "[\n  {\n    \"thread\": \"t2\",\n    \"messages\": 2\n  },\n  {\n    \"thread\": \"t1\",\n    \"messages\": 1\n  }\n]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_counts_json_empty_yields_empty_array() {
        let dir = temp_dir("jour035-empty");
        Journal::open(&dir, "main").expect("open"); // creates an empty segment
        assert_eq!(
            journal_thread_counts_json(&dir.join("main.jsonl")).expect("json"),
            "[]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-036 ---------------------------------------------------------------

    #[test]
    fn journal_thread_counts_jsonl_seeded_yields_one_compact_line_per_thread() {
        let dir = temp_dir("jour036-seeded");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::MessagePersisted, Some("t2"), json!({}));
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t2"),
                json!({"ignored": true}),
            );
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
            append(&mut j, JournalKind::TurnStarted, Some("t3"), json!({}));
        }
        let out = journal_thread_counts_jsonl(&dir.join("main.jsonl")).expect("jsonl");
        assert_eq!(out.lines().count(), 2); // t2: 2, t1: 1 — TurnStarted skipped
        let lines: Vec<serde_json::Value> = out
            .lines()
            .map(|l| serde_json::from_str(l).expect("line is compact json"))
            .collect();
        assert_eq!(lines[0]["thread"], "t2");
        assert_eq!(lines[0]["messages"], 2);
        assert_eq!(lines[1]["thread"], "t1");
        assert_eq!(lines[1]["messages"], 1);
        // Compact: no spaces after separators.
        assert!(out.lines().all(|l| l.starts_with("{\"thread\":")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_counts_jsonl_empty_yields_empty_string() {
        let dir = temp_dir("jour036-empty");
        Journal::open(&dir, "main").expect("open"); // creates an empty segment
        assert_eq!(
            journal_thread_counts_jsonl(&dir.join("main.jsonl")).expect("jsonl"),
            ""
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_jsonl_seeded_yields_only_that_threads_lines() {
        let dir = temp_dir("jour-034");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "first"}),
            );
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t2"),
                json!({"text": "other thread"}),
            );
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "second \"quoted\""}),
            );
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let out = journal_thread_jsonl(&dir.join("main.jsonl"), "t1").expect("jsonl");
        let lines: Vec<serde_json::Value> = out
            .lines()
            .map(|l| serde_json::from_str(l).expect("line is compact json"))
            .collect();
        assert_eq!(out.lines().count(), 2);
        assert_eq!(lines[0]["text"], "first");
        assert_eq!(lines[1]["text"], "second \"quoted\"");
        // ts_ms present and non-decreasing in journal order.
        assert!(lines[0]["ts_ms"].as_u64().unwrap() <= lines[1]["ts_ms"].as_u64().unwrap());
        assert!(!out.contains("other thread"));
        // Compact: no spaces after separators.
        assert!(out.lines().all(|l| l.starts_with("{\"text\":")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_jsonl_unknown_thread_is_empty_string() {
        let dir = temp_dir("jour-034-empty");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "hello"}),
            );
        }
        assert_eq!(
            journal_thread_jsonl(&dir.join("main.jsonl"), "missing").expect("jsonl"),
            ""
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // jour-039 ---------------------------------------------------------------

    #[test]
    fn journal_thread_json_seeded_yields_exact_entries() {
        let dir = temp_dir("jour-039");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "first"}),
            );
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t2"),
                json!({"text": "other thread"}),
            );
            append(&mut j, JournalKind::MessagePersisted, Some("t1"), json!({}));
        }
        let path = dir.join("main.jsonl");
        let out = journal_thread_json(&path, "t1").expect("json");
        let got: serde_json::Value = serde_json::from_str(&out).expect("pretty json array");
        let expected: Vec<serde_json::Value> = Journal::replay(&path)
            .expect("replay")
            .into_iter()
            .filter(|r| {
                r.kind == JournalKind::MessagePersisted && r.thread_id.as_deref() == Some("t1")
            })
            .filter_map(|r| {
                r.payload
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|t| json!({"text": t, "ts_ms": r.ts_ms}))
            })
            .collect();
        assert_eq!(got, serde_json::Value::Array(expected));
        assert!(!out.contains("other thread"));
        // Pretty: entries indented on their own lines.
        assert!(out.starts_with("[\n  {\n    \"text\":"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_thread_json_unknown_thread_is_empty_array() {
        let dir = temp_dir("jour-039-empty");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "hello"}),
            );
        }
        assert_eq!(
            journal_thread_json(&dir.join("main.jsonl"), "missing").expect("json"),
            "[]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_kind_jsonl_seeded_yields_only_that_kinds_lines() {
        let dir = temp_dir("jour-038");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t1"),
                json!({"text": "first"}),
            );
            append(
                &mut j,
                JournalKind::MessagePersisted,
                Some("t2"),
                json!({"text": "second"}),
            );
            append(&mut j, JournalKind::MessagePersisted, None, json!({}));
            append(&mut j, JournalKind::ProviderError, Some("t1"), json!({}));
        }
        let out = journal_kind_jsonl(&dir.join("main.jsonl"), "message_persisted").expect("jsonl");
        let lines: Vec<serde_json::Value> = out
            .lines()
            .map(|l| serde_json::from_str(l).expect("line is compact json"))
            .collect();
        assert_eq!(out.lines().count(), 3);
        // Only message_persisted records, journal order.
        assert!(lines
            .iter()
            .all(|l| l["kind"] == "message_persisted" && l["seq"].as_u64().is_some()));
        assert_eq!(lines[0]["thread_id"], "t1");
        assert_eq!(lines[1]["thread_id"], "t2");
        assert!(lines[2]["thread_id"].is_null());
        // Compact: no spaces after separators.
        assert!(out.lines().all(|l| l.starts_with("{\"seq\":")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_kind_jsonl_unknown_kind_is_empty_string() {
        let dir = temp_dir("jour-038-empty");
        {
            let mut j = Journal::open(&dir, "main").expect("open");
            append(&mut j, JournalKind::TurnStarted, Some("t1"), json!({}));
        }
        assert_eq!(
            journal_kind_jsonl(&dir.join("main.jsonl"), "message_persisted").expect("jsonl"),
            ""
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
