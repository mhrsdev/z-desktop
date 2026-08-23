//! Agent settings (set-001..003, ADR-0011): versioned
//! `<data_dir>/settings.json` with shape `{ "version": 1, "values": { … } }`.
//!
//! Defaults live here — the single home of the §75 values the old runtime
//! consts duplicated. A missing or corrupt file reproduces today's behavior
//! bit-for-bit (24 rounds, 300 s); a corrupt or out-of-range value falls back
//! to the default FOR THAT FIELD ONLY, warned and never fatal. Secrets never
//! enter this file; credentials stay in config.json.
//!
//! Access is snapshot-cached (set-003): `Snapshot` wraps an `Arc<Settings>`
//! held behind one mutex in `Shared`. Readers clone the `Arc` once per turn
//! and read typed values with no lock held during the turn (the ADR-0009
//! snapshot-read discipline).

use serde_json::json;
use std::path::Path;
use std::sync::Arc;

/// Documented defaults (§75 / former runtime.rs consts).
const DEFAULT_MAX_TOOL_ROUNDS: usize = 24;
const DEFAULT_APPROVAL_TIMEOUT_SECS: u64 = 300;
/// Doom-loop breaker threshold (ADR-0017 D6): steer at N identical calls.
const DEFAULT_DOOM_THRESHOLD: usize = 3;

/// Sanity bounds for hand-edited files: no zero-round wedge of the turn loop,
/// no multi-hour approval hang. Out-of-range ⇒ default + warn, never fail.
const MIN_TOOL_ROUNDS: u64 = 1;
const MAX_TOOL_ROUNDS_CAP: u64 = 200;
const MIN_APPROVAL_TIMEOUT_SECS: u64 = 5;
const MAX_APPROVAL_TIMEOUT_SECS: u64 = 3600;
const MIN_DOOM_THRESHOLD: u64 = 1;
const MAX_DOOM_THRESHOLD_CAP: u64 = 10;

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Upper bound on tool rounds per turn (was `MAX_TOOL_ROUNDS`).
    pub max_tool_rounds: usize,
    /// Approval-gate deadline in seconds (was `APPROVAL_TIMEOUT`).
    pub approval_timeout_secs: u64,
    /// Doom-loop breaker threshold (ADR-0017 D6): steer at N identical
    /// tool calls in one turn, hard-fail at 2N.
    pub doom_threshold: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
            approval_timeout_secs: DEFAULT_APPROVAL_TIMEOUT_SECS,
            doom_threshold: DEFAULT_DOOM_THRESHOLD,
        }
    }
}

/// Cheap-to-share view of settings (set-003). Held as `Mutex<Arc<Snapshot>>`;
/// writers swap the inner `Arc` under the lock, readers clone it once per turn.
#[derive(Clone, Default)]
pub struct Snapshot(Arc<Settings>);

impl Snapshot {
    pub fn new(settings: Settings) -> Self {
        Self(Arc::new(settings))
    }

    pub fn get(&self) -> &Arc<Settings> {
        &self.0
    }
}

/// Load settings from `<data_dir>/settings.json`. Never fails: a missing file
/// yields the defaults silently; invalid JSON warns and yields defaults; a
/// present-but-invalid field keeps its default while siblings load normally.
pub fn load(data_dir: &Path) -> Settings {
    let mut s = Settings::default();
    let Ok(raw) = std::fs::read_to_string(data_dir.join("settings.json")) else {
        return s; // no file yet: documented defaults, byte-identical behavior
    };
    let Ok(file) = serde_json::from_str::<serde_json::Value>(&raw) else {
        log::warn!("settings.json is not valid JSON; using defaults");
        return s;
    };
    let values = file.get("values");

    if let Some(v) = values.and_then(|vals| vals.get("max_tool_rounds")) {
        match v.as_u64() {
            Some(n) if (MIN_TOOL_ROUNDS..=MAX_TOOL_ROUNDS_CAP).contains(&n) => {
                s.max_tool_rounds = n as usize;
            }
            _ => log::warn!(
                "settings.json: ignoring invalid max_tool_rounds ({v}); using default {DEFAULT_MAX_TOOL_ROUNDS}"
            ),
        }
    }
    if let Some(v) = values.and_then(|vals| vals.get("approval_timeout_secs")) {
        match v.as_u64() {
            Some(n) if (MIN_APPROVAL_TIMEOUT_SECS..=MAX_APPROVAL_TIMEOUT_SECS).contains(&n) => {
                s.approval_timeout_secs = n;
            }
            _ => log::warn!(
                "settings.json: ignoring invalid approval_timeout_secs ({v}); using default {DEFAULT_APPROVAL_TIMEOUT_SECS}"
            ),
        }
    }
    if let Some(v) = values.and_then(|vals| vals.get("doom_threshold")) {
        match v.as_u64() {
            Some(n) if (MIN_DOOM_THRESHOLD..=MAX_DOOM_THRESHOLD_CAP).contains(&n) => {
                s.doom_threshold = n as usize;
            }
            _ => log::warn!(
                "settings.json: ignoring invalid doom_threshold ({v}); using default {DEFAULT_DOOM_THRESHOLD}"
            ),
        }
    }
    s
}

/// Persist settings atomically (edit-004 helper) as pretty JSON. Unknown keys
/// written by future versions are not preserved yet — set-004 owns merge
/// semantics when the command path lands.
pub fn store(data_dir: &Path, s: &Settings) -> Result<(), String> {
    let pretty = serde_json::to_string_pretty(&json!({
        "version": VERSION,
        "values": {
            "max_tool_rounds": s.max_tool_rounds,
            "approval_timeout_secs": s.approval_timeout_secs,
            "doom_threshold": s.doom_threshold,
        },
    }))
    .map_err(|e| e.to_string())?;
    crate::atomic_write::atomic_write(&data_dir.join("settings.json"), pretty.as_bytes())
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use std::path::PathBuf;

    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn unique_data_dir(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("zdt-set-{tag}-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp data dir");
        dir
    }

    #[test]
    fn missing_file_yields_documented_defaults() {
        let dir = unique_data_dir("missing");
        assert_eq!(load(&dir), Settings::default());
        assert_eq!(load(&dir).max_tool_rounds, 24);
        assert_eq!(load(&dir).approval_timeout_secs, 300);
        assert_eq!(load(&dir).doom_threshold, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doom_threshold_defaults_and_round_trips() {
        let dir = unique_data_dir("doom");
        assert_eq!(load(&dir).doom_threshold, 3, "documented default is 3");
        std::fs::write(
            dir.join("settings.json"),
            r#"{"version":1,"values":{"doom_threshold":7}}"#,
        )
        .unwrap();
        assert_eq!(load(&dir).doom_threshold, 7);
        store(&dir, &Settings { doom_threshold: 9, ..Settings::default() }).unwrap();
        assert_eq!(load(&dir).doom_threshold, 9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stored_file_round_trips_through_load() {
        let dir = unique_data_dir("roundtrip");
        let s = Settings { max_tool_rounds: 7, approval_timeout_secs: 60, doom_threshold: 5 };
        store(&dir, &s).expect("store succeeds");
        // Pretty, versioned shape on disk.
        let raw = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(raw.contains("\"version\": 1"));
        assert_eq!(load(&dir), s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn out_of_range_field_falls_back_independently() {
        let dir = unique_data_dir("range");
        std::fs::write(
            dir.join("settings.json"),
            r#"{"version":1,"values":{"max_tool_rounds":5000,"approval_timeout_secs":60}}"#,
        )
        .unwrap();
        let s = load(&dir);
        assert_eq!(s.max_tool_rounds, 24, "out-of-range rounds fall back to default");
        assert_eq!(s.approval_timeout_secs, 60, "valid sibling field is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_and_mistyped_files_never_fail() {
        let dir = unique_data_dir("corrupt");
        std::fs::write(dir.join("settings.json"), "{not json").unwrap();
        assert_eq!(load(&dir), Settings::default());

        std::fs::write(
            dir.join("settings.json"),
            r#"{"version":1,"values":{"max_tool_rounds":"many","approval_timeout_secs":-5}}"#,
        )
        .unwrap();
        assert_eq!(load(&dir), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
