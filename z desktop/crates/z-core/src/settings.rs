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

/// Current on-disk schema version (set-007). Bump only alongside a new
/// vN→vN+1 step in [`migrate`].
pub const SETTINGS_VERSION: u32 = 1;

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

/// set-004: validate + apply one SetSetting against a copy of the current
/// values. Unknown keys, mistyped values, and out-of-range values are
/// rejected with a human message; nothing mutates on rejection. The bounds
/// live here so the command path can never drift from `load`'s sanity checks.
pub fn apply(s: &Settings, key: &str, value: &serde_json::Value) -> Result<Settings, String> {
    let mut out = s.clone();
    match key {
        "max_tool_rounds" => match value.as_u64() {
            Some(n) if (MIN_TOOL_ROUNDS..=MAX_TOOL_ROUNDS_CAP).contains(&n) => {
                out.max_tool_rounds = n as usize;
            }
            _ => {
                return Err(format!(
                    "{key} must be an integer in {MIN_TOOL_ROUNDS}..={MAX_TOOL_ROUNDS_CAP}"
                ))
            }
        },
        "approval_timeout_secs" => match value.as_u64() {
            Some(n) if (MIN_APPROVAL_TIMEOUT_SECS..=MAX_APPROVAL_TIMEOUT_SECS).contains(&n) => {
                out.approval_timeout_secs = n;
            }
            _ => {
                return Err(format!(
                    "{key} must be an integer in {MIN_APPROVAL_TIMEOUT_SECS}..={MAX_APPROVAL_TIMEOUT_SECS}"
                ))
            }
        },
        "doom_threshold" => match value.as_u64() {
            Some(n) if (MIN_DOOM_THRESHOLD..=MAX_DOOM_THRESHOLD_CAP).contains(&n) => {
                out.doom_threshold = n as usize;
            }
            _ => {
                return Err(format!(
                    "{key} must be an integer in {MIN_DOOM_THRESHOLD}..={MAX_DOOM_THRESHOLD_CAP}"
                ))
            }
        },
        other => return Err(format!("unknown setting \"{other}\"")),
    }
    Ok(out)
}

/// Persist settings atomically (edit-004 helper) as pretty JSON. Unknown keys
/// written by future versions are not preserved yet — set-004 owns merge
/// semantics when the command path lands.
pub fn store(data_dir: &Path, s: &Settings) -> Result<(), String> {
    let pretty = serde_json::to_string_pretty(&json!({
        "version": SETTINGS_VERSION,
        "values": {
            "max_tool_rounds": s.max_tool_rounds,
            "approval_timeout_secs": s.approval_timeout_secs,
            "doom_threshold": s.doom_threshold,
        },
    }))
    .map_err(|e| e.to_string())?;
    crate::atomic_write::atomic_write(&data_dir.join("settings.json"), pretty.as_bytes())
}

/// set-010: pretty JSON of the current user settings for external UIs —
/// `{ "version": N, "values": { … } }`, one entry per [`schema_defs`] key
/// (a `Settings` field without a schema entry is skipped). Shape mirrors
/// [`store`], so the output passes [`migrate`] unchanged.
pub fn export_user_json(current: &Settings) -> String {
    let mut values = serde_json::Map::new();
    for def in schema_defs() {
        let v = match def.key {
            "max_tool_rounds" => Some(json!(current.max_tool_rounds)),
            "approval_timeout_secs" => Some(json!(current.approval_timeout_secs)),
            "doom_threshold" => Some(json!(current.doom_threshold)),
            _ => None,
        };
        if let Some(v) = v {
            values.insert(def.key.to_string(), v);
        }
    }
    serde_json::to_string_pretty(&json!({ "version": SETTINGS_VERSION, "values": values }))
        .expect("settings always serialize")
}

/// set-017: versioned export for external UIs. Same document as
/// [`export_user_json`] — alias kept so callers read by intent.
pub fn settings_export_json(current: &Settings) -> String {
    export_user_json(current)
}

/// set-007: migrate raw `settings.json` text up to [`SETTINGS_VERSION`],
/// returning the (possibly rewritten) payload and its new version.
///
/// - version == [`SETTINGS_VERSION`] ⇒ byte-for-byte passthrough.
/// - version < [`SETTINGS_VERSION`] ⇒ run the chained per-step migrations
///   (one `fn vN_to_vN_plus_1(doc) -> Value` each, applied oldest-first).
///   None exist yet — the chain below is where future steps slot in.
/// - version > [`SETTINGS_VERSION`] ⇒ Err: the file was written by a newer
///   build and silently downgrading could drop fields.
/// - Anything else (bad JSON, missing/non-integer `version`) ⇒ Err.
pub fn migrate(raw: &str) -> Result<(String, u32), String> {
    let mut doc: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| format!("settings.json is not valid JSON: {e}"))?;
    let version = doc
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "settings.json has no integer \"version\" field".to_string())?
        as u32;
    if version > SETTINGS_VERSION {
        return Err(format!(
            "settings.json version {version} is newer than this build understands \
             (supported: {SETTINGS_VERSION}); please upgrade the application"
        ));
    }
    if version == SETTINGS_VERSION {
        return Ok((raw.to_string(), version));
    }
    // Future migration chain, oldest first:
    // if version < 2 { doc = v1_to_v2(doc); }
    // if version < 3 { doc = v2_to_v3(doc); }
    Ok((doc.to_string(), SETTINGS_VERSION))
}

/// Type of one setting's value in the schema (set-002).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefKind {
    U64,
    F32,
    Bool,
    String,
}

/// Documented default for one setting, tagged by its [`DefKind`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingDefault {
    U64(u64),
    F32(f32),
    Bool(bool),
    String(&'static str),
}

/// Schema entry for one setting (set-002): key, type, sanity bounds, default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettingDef {
    pub key: &'static str,
    pub kind: DefKind,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub default: SettingDefault,
}

/// §75 draft schema: one [`SettingDef`] per `Settings` field, with the same
/// sanity bounds `load`/`apply` enforce. Single source for UIs and validators;
/// bounds here must not drift from the consts above.
pub fn schema_defs() -> &'static [SettingDef] {
    const fn u64_def(key: &'static str, min: u64, max: u64, default: u64) -> SettingDef {
        SettingDef {
            key,
            kind: DefKind::U64,
            min: Some(min as f64),
            max: Some(max as f64),
            default: SettingDefault::U64(default),
        }
    }
    // A `static` (not an inline literal) so the const-fn entries get 'static life.
    static DEFS: [SettingDef; 3] = [
        u64_def(
            "max_tool_rounds",
            MIN_TOOL_ROUNDS,
            MAX_TOOL_ROUNDS_CAP,
            DEFAULT_MAX_TOOL_ROUNDS as u64,
        ),
        u64_def(
            "approval_timeout_secs",
            MIN_APPROVAL_TIMEOUT_SECS,
            MAX_APPROVAL_TIMEOUT_SECS,
            DEFAULT_APPROVAL_TIMEOUT_SECS,
        ),
        u64_def(
            "doom_threshold",
            MIN_DOOM_THRESHOLD,
            MAX_DOOM_THRESHOLD_CAP,
            DEFAULT_DOOM_THRESHOLD as u64,
        ),
    ];
    &DEFS
}

/// set-006: every key this version understands, derived from [`schema_defs`].
/// Single source of truth for command-path spelling checks.
pub fn known_keys() -> Vec<&'static str> {
    schema_defs().iter().map(|d| d.key).collect()
}

/// set-011: tokenized search index over [`schema_defs`] — one `(key, tokens)`
/// per def, tokens being the key split on `_` plus the kind name (`"u64"`,
/// matching [`export_schema_json`] spelling). For future token-based
/// settings search UIs; order mirrors [`schema_defs`].
pub fn schema_search_index() -> Vec<(String, Vec<String>)> {
    schema_defs()
        .iter()
        .map(|d| {
            let mut tokens: Vec<String> = d.key.split('_').map(str::to_string).collect();
            tokens.push(
                match d.kind {
                    DefKind::U64 => "u64",
                    DefKind::F32 => "f32",
                    DefKind::Bool => "bool",
                    DefKind::String => "string",
                }
                .to_string(),
            );
            (d.key.to_string(), tokens)
        })
        .collect()
}

/// set-008: case-insensitive substring search over [`schema_defs`] keys, for
/// a future settings search UI. Empty query ⇒ every def (schema order).
pub fn search_defs(query: &str) -> Vec<&'static SettingDef> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return schema_defs().iter().collect();
    }
    schema_defs()
        .iter()
        .filter(|d| d.key.to_lowercase().contains(&q))
        .collect()
}

/// One [`SettingDef`] as its compact JSON row (`key`, `kind`, `min`, `max`,
/// `default`) — single source shared by [`export_schema_json`] and
/// [`schema_defs_jsonl`] so their field shapes can never drift apart.
fn schema_def_value(d: &SettingDef) -> serde_json::Value {
    json!({
        "key": d.key,
        "kind": match d.kind {
            DefKind::U64 => "u64",
            DefKind::F32 => "f32",
            DefKind::Bool => "bool",
            DefKind::String => "string",
        },
        "min": d.min,
        "max": d.max,
        "default": match d.default {
            SettingDefault::U64(v) => json!(v),
            SettingDefault::F32(v) => json!(v),
            SettingDefault::Bool(v) => json!(v),
            SettingDefault::String(v) => json!(v),
        },
    })
}

/// set-008 (ext): pretty JSON export of [`schema_defs`] — one object per
/// setting with `key`, `kind`, `min`, `max`, `default`; rows come from
/// [`schema_def_value`]. Single source for external UIs/validators.
pub fn export_schema_json() -> String {
    let defs: Vec<serde_json::Value> = schema_defs().iter().map(schema_def_value).collect();
    serde_json::to_string_pretty(&defs).expect("schema defs always serialize")
}

/// set-022: compact JSONL of [`schema_defs`] — one self-contained JSON object
/// per line (trailing newline included), fields identical to the
/// [`export_schema_json`] rows via the shared [`schema_def_value`]. Line-per-
/// record shape for streaming/append-friendly external UIs.
pub fn schema_defs_jsonl() -> String {
    let mut out = String::new();
    for d in schema_defs() {
        let line = serde_json::to_string(&schema_def_value(d)).expect("schema defs always serialize");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// set-012: markdown table of [`schema_defs`] (`| key | kind | min | max |
/// default |`) for docs embedding. Header row included, one row per def in
/// schema order; bounds render as plain numbers (all current bounds are
/// integral) and absent bounds render as an empty cell.
pub fn export_schema_markdown() -> String {
    let mut out = String::from("| key | kind | min | max | default |\n");
    out.push_str("|---|---|---|---|---|\n");
    for d in schema_defs() {
        let bound = |b: Option<f64>| b.map(|v| v.to_string()).unwrap_or_default();
        let default = match d.default {
            SettingDefault::U64(v) => v.to_string(),
            SettingDefault::F32(v) => v.to_string(),
            SettingDefault::Bool(v) => v.to_string(),
            SettingDefault::String(v) => v.to_string(),
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            d.key,
            match d.kind {
                DefKind::U64 => "u64",
                DefKind::F32 => "f32",
                DefKind::Bool => "bool",
                DefKind::String => "string",
            },
            bound(d.min),
            bound(d.max),
            default
        ));
    }
    out
}

/// set-006: each setting's documented default rendered as its string form
/// (`"24"`, `"300"`, …), same order as [`schema_defs`]. For UIs and help text.
pub fn defaults_map() -> Vec<(&'static str, String)> {
    schema_defs()
        .iter()
        .map(|d| {
            let rendered = match d.default {
                SettingDefault::U64(v) => v.to_string(),
                SettingDefault::F32(v) => v.to_string(),
                SettingDefault::Bool(v) => v.to_string(),
                SettingDefault::String(v) => v.to_string(),
            };
            (d.key, rendered)
        })
        .collect()
}

/// set-013: schema-keyed `(key, value)` pairs where `current` differs from its
/// documented default, both sides rendered as strings; order mirrors
/// [`schema_defs`]. Fresh defaults ⇒ empty. Defaults come from [`defaults_map`]
/// so this can never drift from the schema.
pub fn diff_from_default(current: &Settings) -> Vec<(String, String)> {
    defaults_map()
        .into_iter()
        .filter_map(|(key, default)| {
            let value = match key {
                "max_tool_rounds" => Some(current.max_tool_rounds.to_string()),
                "approval_timeout_secs" => Some(current.approval_timeout_secs.to_string()),
                "doom_threshold" => Some(current.doom_threshold.to_string()),
                _ => None,
            }?;
            (value != default).then_some((key.to_string(), value))
        })
        .collect()
}

/// set-025: pretty JSON array of `{key, value}` rows from [`diff_from_default`]
/// for external UIs; fresh defaults ⇒ `"[]"`. Single source is
/// [`diff_from_default`], so this can never drift from the schema.
pub fn settings_diff_json(current: &Settings) -> String {
    let rows: Vec<serde_json::Value> = diff_from_default(current)
        .into_iter()
        .map(|(key, value)| json!({ "key": key, "value": value }))
        .collect();
    serde_json::to_string_pretty(&rows).expect("diff rows always serialize")
}

/// set-015: one-line summary of `current` — `"{n} settings, {d} changed
/// from default"` where `n` counts every [`schema_defs`] key and `d` comes
/// from [`diff_from_default`], so "changed" can never drift from the schema.
pub fn settings_summary(current: &Settings) -> String {
    format!(
        "{} settings, {} changed from default",
        schema_defs().len(),
        diff_from_default(current).len()
    )
}

/// set-016: one-line health readout — [`settings_summary`] + validation
/// status from [`validate_all`]: `"{summary} | {n} validation errors"`
/// (`"0 validation errors"` when everything is in bounds). Both halves reuse
/// their single sources, so this cannot drift from the schema.
pub fn settings_health_line(current: &Settings) -> String {
    format!(
        "{} | {} validation errors",
        settings_summary(current),
        validate_all(current).len()
    )
}

/// set-023: pretty JSON health readout for external UIs —
/// `{ "summary": "…", "errors": [...] }`, combining [`settings_summary`] and
/// [`validate_all`]. Both halves reuse their single sources, so this cannot
/// drift from the schema.
pub fn settings_health_json(current: &Settings) -> String {
    serde_json::to_string_pretty(&json!({
        "summary": settings_summary(current),
        "errors": validate_all(current),
    }))
    .expect("health report always serializes")
}

/// set-018: schema keys starting with `prefix` (case-insensitive), in schema
/// order. Empty prefix ⇒ every key.
pub fn settings_by_prefix(prefix: &str) -> Vec<&'static str> {
    let p = prefix.to_lowercase();
    schema_defs()
        .iter()
        .filter(|d| d.key.to_lowercase().starts_with(&p))
        .map(|d| d.key)
        .collect()
}

/// set-019: count of [`schema_defs`] entries per [`DefKind`], as
/// `(kind name, count)` pairs sorted by count descending (ties keep the
/// [`DefKind`] declaration order). Kind names match [`export_schema_json`].
pub fn schema_kind_counts() -> Vec<(String, usize)> {
    let mut counts = [("u64", 0usize), ("f32", 0), ("bool", 0), ("string", 0)];
    for def in schema_defs() {
        let idx = match def.kind {
            DefKind::U64 => 0,
            DefKind::F32 => 1,
            DefKind::Bool => 2,
            DefKind::String => 3,
        };
        counts[idx].1 += 1;
    }
    // Stable sort: equal counts stay in DefKind declaration order.
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    counts.into_iter().map(|(k, n)| (k.to_string(), n)).collect()
}

/// set-024: pretty schema-stats JSON — `{ "total": n, "by_kind": { … } }`,
/// reusing [`schema_kind_counts`] as its single source so it cannot drift
/// from the schema.
pub fn settings_schema_stats_json() -> String {
    let counts = schema_kind_counts();
    let by_kind: serde_json::Map<String, serde_json::Value> =
        counts.iter().map(|(k, n)| (k.clone(), json!(n))).collect();
    serde_json::to_string_pretty(&json!({
        "total": counts.iter().map(|(_, n)| n).sum::<usize>(),
        "by_kind": by_kind,
    }))
    .expect("stats always serialize")
}

/// set-020: pretty JSON of [`defaults_map`] as `{ key: value }` — one entry
/// per schema key in schema order, values being the same string renderings
/// [`defaults_map`] produces. Single source is [`defaults_map`], so this can
/// never drift from the schema.
pub fn settings_defaults_json() -> String {
    let mut obj = serde_json::Map::new();
    for (key, value) in defaults_map() {
        obj.insert(key.to_string(), json!(value));
    }
    serde_json::to_string_pretty(&obj).expect("defaults always serialize")
}

/// Validate a numeric value against the schema bounds for `key`. The unknown-
/// key path never silently ignores: an unrecognized key is rejected with an
/// "unknown setting \\"key\\"" message so hand-edited typos surface instead of
/// being dropped (set-006: callers can check spelling against [`known_keys`]).
/// Out-of-range values are likewise rejected with a human message; nothing
/// mutates. This mirrors `apply`'s unknown-key handling: reject + report,
/// keeping the previous value (keep+warn at the command layer).
pub fn validate(key: &str, value: f64) -> Result<(), String> {
    let Some(def) = schema_defs().iter().find(|d| d.key == key) else {
        return Err(format!("unknown setting \"{key}\""));
    };
    let lo = def.min.unwrap_or(f64::NEG_INFINITY);
    let hi = def.max.unwrap_or(f64::INFINITY);
    if value.is_finite() && (lo..=hi).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{key} must be an integer in {}..={}",
            lo as i64, hi as i64
        ))
    }
}

/// set-005: pretty constraint message for an out-of-range value, or None if
/// `value` is valid for `key`. Thin wrapper over [`validate`] so callers get
/// Option-shaped ergonomics without re-deriving bounds.
pub fn constraint_error(key: &str, value: f64) -> Option<String> {
    validate(key, value).err()
}

/// set-009: remap-preview helper — validate then hand back the effective
/// value for a future remap UI. Bounds are inclusive integers, so any
/// accepted value is already its own effective form; rejection mirrors
/// [`validate`] exactly.
pub fn remap_check(key: &str, new_value: f64) -> Result<f64, String> {
    validate(key, new_value).map(|()| new_value)
}

/// set-014: validate every schema key against its live value in `current`,
/// collecting one human message per failing key in schema order; an empty
/// vec means all settings are within their documented bounds. Reuses
/// [`validate`] so bounds can never drift from the command path.
pub fn validate_all(current: &Settings) -> Vec<String> {
    schema_defs()
        .iter()
        .filter_map(|def| {
            // ponytail: match-on-key like diff_from_default; serde-serialize
            // round-trip would be slower and no less repetitive.
            let value = match def.key {
                "max_tool_rounds" => current.max_tool_rounds as f64,
                "approval_timeout_secs" => current.approval_timeout_secs as f64,
                "doom_threshold" => current.doom_threshold as f64,
                _ => return None,
            };
            validate(def.key, value).err()
        })
        .collect()
}

/// set-021: pretty JSON validation report for external UIs —
/// `{ "valid": bool, "errors": [...] }`, derived entirely from
/// [`validate_all`] so it can never drift from the schema bounds.
pub fn settings_validate_json(current: &Settings) -> String {
    let errors = validate_all(current);
    serde_json::to_string_pretty(&json!({
        "valid": errors.is_empty(),
        "errors": errors,
    }))
    .expect("validation report always serializes")
}

/// set-005: restore one key's documented default into `current` via [`apply`],
/// so resets can never drift from the command path's validation. Returns
/// Ok(false) for unknown keys (nothing to reset); Err only on internal
/// inconsistency (a schema default failing its own bounds).
pub fn reset_to_default(key: &str, current: &mut Settings) -> Result<bool, String> {
    let Some(def) = schema_defs().iter().find(|d| d.key == key) else {
        return Ok(false);
    };
    let value = match def.default {
        SettingDefault::U64(v) => serde_json::Value::from(v),
        SettingDefault::F32(v) => serde_json::Value::from(v),
        SettingDefault::Bool(v) => serde_json::Value::from(v),
        SettingDefault::String(v) => serde_json::Value::from(v),
    };
    *current = apply(current, key, &value)?;
    Ok(true)
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
    fn apply_rejects_bad_keys_and_values_without_mutating() {
        let base = Settings::default();
        assert!(apply(&base, "nope", &json!(5)).is_err(), "unknown key rejected");
        assert!(apply(&base, "max_tool_rounds", &json!(0)).is_err());
        assert!(apply(&base, "max_tool_rounds", &json!(201)).is_err());
        assert!(apply(&base, "max_tool_rounds", &json!("many")).is_err());
        assert!(apply(&base, "approval_timeout_secs", &json!(4)).is_err());
        assert!(apply(&base, "approval_timeout_secs", &json!(3601)).is_err());
        // Rejections leave the base untouched.
        assert_eq!(base, Settings::default());
    }

    #[test]
    fn applied_set_setting_round_trips_through_store_and_load() {
        let dir = unique_data_dir("set004");
        let updated = apply(&Settings::default(), "max_tool_rounds", &json!(2))
            .expect("in-range value applies");
        assert_eq!(updated.max_tool_rounds, 2);
        assert_eq!(updated.approval_timeout_secs, 300, "siblings untouched");
        let updated = apply(&updated, "approval_timeout_secs", &json!(60))
            .expect("second key applies");
        store(&dir, &updated).expect("store succeeds");
        assert_eq!(load(&dir), updated, "SetSetting survives store/load");
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

    // ---- set-002: schema population ------------------------------------

    #[test]
    fn schema_covers_every_settings_field() {
        // Compile-level coverage: each field name must appear exactly once.
        for key in ["max_tool_rounds", "approval_timeout_secs", "doom_threshold"] {
            assert_eq!(
                schema_defs().iter().filter(|d| d.key == key).count(),
                1,
                "schema_defs must define {key} exactly once"
            );
        }
        // Defaults in the schema match the live documented defaults.
        let d = Settings::default();
        for def in schema_defs() {
            match (def.key, def.default) {
                ("max_tool_rounds", SettingDefault::U64(v)) => {
                    assert_eq!(v as usize, d.max_tool_rounds)
                }
                ("approval_timeout_secs", SettingDefault::U64(v)) => {
                    assert_eq!(v, d.approval_timeout_secs)
                }
                ("doom_threshold", SettingDefault::U64(v)) => {
                    assert_eq!(v as usize, d.doom_threshold)
                }
                other => panic!("unexpected schema entry {other:?}"),
            }
        }
    }

    #[test]
    fn validate_enforces_schema_ranges_table() {
        // (key, value, expected-valid)
        let cases: &[(&str, f64, bool)] = &[
            ("max_tool_rounds", 1.0, true),
            ("max_tool_rounds", 24.0, true),
            ("max_tool_rounds", 200.0, true),
            ("max_tool_rounds", 0.0, false),
            ("max_tool_rounds", 200.5, false),
            ("max_tool_rounds", 201.0, false),
            ("max_tool_rounds", -1.0, false),
            ("max_tool_rounds", f64::NAN, false),
            ("max_tool_rounds", f64::INFINITY, false),
            ("approval_timeout_secs", 5.0, true),
            ("approval_timeout_secs", 300.0, true),
            ("approval_timeout_secs", 3600.0, true),
            ("approval_timeout_secs", 4.9, false),
            ("approval_timeout_secs", 3601.0, false),
            ("doom_threshold", 1.0, true),
            ("doom_threshold", 3.0, true),
            ("doom_threshold", 10.0, true),
            ("doom_threshold", 0.99, false),
            ("doom_threshold", 11.0, false),
        ];
        for &(key, value, ok) in cases {
            assert_eq!(
                validate(key, value).is_ok(),
                ok,
                "validate({key}, {value}) should be {ok:?}"
            );
        }
    }

    #[test]
    fn validate_rejects_unknown_keys_with_message() {
        let err = validate("nope", 5.0).unwrap_err();
        assert!(err.contains("unknown setting"), "got: {err}");
        assert!(err.contains("nope"), "error names the bad key: {err}");
    }

    #[test]
    fn validate_bounds_match_load_and_apply_bounds() {
        // The schema and the command path must agree on every boundary.
        for def in schema_defs() {
            if let (Some(min), Some(max)) = (def.min, def.max) {
                assert!(
                    validate(def.key, min).is_ok(),
                    "{} min bound valid",
                    def.key
                );
                assert!(
                    validate(def.key, max).is_ok(),
                    "{} max bound valid",
                    def.key
                );
                assert!(
                    validate(def.key, min - 1.0).is_err() && validate(def.key, max + 1.0).is_err(),
                    "{} rejects outside bounds",
                    def.key
                );
            }
        }
    }

    // ---- set-006: unknown-key keep+warn + defaults map -------------------

    #[test]
    fn apply_unknown_key_is_err_and_keeps_settings_unchanged() {
        let base = Settings { max_tool_rounds: 42, approval_timeout_secs: 60, doom_threshold: 5 };
        let res = apply(&base, "future_key", &json!(7));
        assert!(res.is_err(), "unknown SetSetting key => Err");
        let err = res.unwrap_err();
        assert!(err.contains("unknown setting"), "warn text names the problem: {err}");
        assert!(err.contains("future_key"), "error names the bad key: {err}");
        // keep: nothing about `base` changed by the rejected attempt.
        assert_eq!(base.max_tool_rounds, 42);
        assert_eq!(base.approval_timeout_secs, 60);
        assert_eq!(base.doom_threshold, 5);
    }

    #[test]
    fn known_keys_covers_every_schema_def() {
        let keys = known_keys();
        assert_eq!(keys.len(), schema_defs().len(), "one known key per def");
        for def in schema_defs() {
            assert!(keys.contains(&def.key), "{} missing from known_keys", def.key);
        }
        assert!(!keys.contains(&"nope"), "unknown keys stay unknown");
    }

    #[test]
    fn defaults_map_matches_settings_default() {
        let d = Settings::default();
        let map = defaults_map();
        assert_eq!(map.len(), schema_defs().len());
        for (key, rendered) in map {
            let expected = match key {
                "max_tool_rounds" => d.max_tool_rounds.to_string(),
                "approval_timeout_secs" => d.approval_timeout_secs.to_string(),
                "doom_threshold" => d.doom_threshold.to_string(),
                other => panic!("unexpected key {other}"),
            };
            assert_eq!(rendered, expected, "{key} default parity with Settings::default()");
        }
    }

    // ---- set-007: migration framework -----------------------------------

    #[test]
    fn migrate_same_version_is_byte_passthrough() {
        let raw = r#"{"version":1,"values":{"doom_threshold":7}}"#;
        let (out, v) = migrate(raw).expect("same version passes through");
        assert_eq!(v, SETTINGS_VERSION);
        assert_eq!(out, raw, "byte-for-byte passthrough");
    }

    #[test]
    fn migrate_older_version_upgrades_and_keeps_values() {
        let raw = r#"{"version":0,"values":{"max_tool_rounds":10}}"#;
        let (out, v) = migrate(raw).expect("older version migrates up");
        assert_eq!(v, SETTINGS_VERSION);
        let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(doc["values"]["max_tool_rounds"], 10, "values survive migration");
    }

    #[test]
    fn migrate_newer_version_is_err() {
        let err = migrate(r#"{"version":999,"values":{}}"#).unwrap_err();
        assert!(err.contains("newer"), "error explains the problem: {err}");
        assert!(err.contains("999"), "error names the file's version: {err}");
    }

    #[test]
    fn migrate_rejects_malformed_input() {
        assert!(migrate("{not json").is_err(), "bad JSON => Err");
        assert!(migrate(r#"{"values":{}}"#).is_err(), "missing version => Err");
        assert!(migrate(r#"{"version":"one"}"#).is_err(), "non-integer version => Err");
    }

    // ---- set-005: constraint messages + per-key reset --------------------

    #[test]
    fn constraint_error_names_key_and_bounds_when_invalid() {
        let err = constraint_error("max_tool_rounds", 5000.0)
            .expect("out-of-range value yields a message");
        assert!(err.contains("max_tool_rounds"), "names the key: {err}");
        assert!(
            err.contains('1') && err.contains("200"),
            "message carries the bounds: {err}"
        );
        assert!(
            constraint_error("max_tool_rounds", 24.0).is_none(),
            "valid value => no error"
        );
        // Unknown keys surface through the same Option path.
        let err = constraint_error("nope", 5.0).expect("unknown key yields a message");
        assert!(err.contains("unknown setting") && err.contains("nope"), "{err}");
    }

    #[test]
    fn reset_to_default_restores_one_key_via_apply() {
        let mut s =
            Settings { max_tool_rounds: 199, approval_timeout_secs: 60, doom_threshold: 9 };
        assert!(reset_to_default("max_tool_rounds", &mut s).unwrap());
        assert_eq!(s.max_tool_rounds, DEFAULT_MAX_TOOL_ROUNDS, "key reset");
        assert_eq!(s.approval_timeout_secs, 60, "siblings untouched");
        assert_eq!(s.doom_threshold, 9, "siblings untouched");
        // Every schema key resets to its documented default.
        let mut all = Settings { max_tool_rounds: 1, approval_timeout_secs: 5, doom_threshold: 1 };
        for def in schema_defs() {
            assert!(reset_to_default(def.key, &mut all).expect("schema default applies"));
        }
        assert_eq!(all, Settings::default());
    }

    #[test]
    fn reset_unknown_key_is_ok_false_without_mutating() {
        let mut s = Settings::default();
        assert_eq!(reset_to_default("future_key", &mut s), Ok(false));
        assert_eq!(s, Settings::default(), "nothing changed on unknown key");
    }

    // ---- set-008: settings search ----------------------------------------

    #[test]
    fn search_defs_matches_fragments_case_insensitively() {
        let hits = search_defs("tool_round");
        assert_eq!(hits.len(), 1, "fragment matches exactly one key");
        assert_eq!(hits[0].key, "max_tool_rounds");
        // Case insensitivity in both query and key casing.
        assert_eq!(
            search_defs("TOOL_ROUND")
                .iter()
                .map(|d| d.key)
                .collect::<Vec<_>>(),
            vec!["max_tool_rounds"],
            "uppercase query still matches"
        );
        assert_eq!(
            search_defs("Doom")
                .iter()
                .map(|d| d.key)
                .collect::<Vec<_>>(),
            vec!["doom_threshold"]
        );
    }

    #[test]
    fn search_defs_no_match_is_empty() {
        assert!(search_defs("zzz_no_such_setting").is_empty());
        // Substring must be contiguous — scattered letters don't match.
        assert!(search_defs("maxrounds").is_empty());
    }

    #[test]
    fn search_defs_empty_query_returns_all_defs() {
        let all = search_defs("");
        assert_eq!(all.len(), schema_defs().len(), "empty query ⇒ every def");
        for (got, want) in all.iter().zip(schema_defs()) {
            assert_eq!(got.key, want.key, "schema order preserved");
        }
        // Whitespace-only queries match nothing (keys have no spaces).
        assert!(search_defs("   ").is_empty());
    }

    // ---- set-009: remap preview ------------------------------------------

    #[test]
    fn remap_check_passes_in_range_values_through() {
        assert_eq!(remap_check("max_tool_rounds", 24.0), Ok(24.0));
        // Every schema boundary is inclusive passthrough.
        for def in schema_defs() {
            if let (Some(min), Some(max)) = (def.min, def.max) {
                assert_eq!(remap_check(def.key, min), Ok(min), "{} min bound", def.key);
                assert_eq!(remap_check(def.key, max), Ok(max), "{} max bound", def.key);
            }
        }
    }

    #[test]
    fn remap_check_rejects_out_of_range() {
        assert!(remap_check("max_tool_rounds", 201.0).is_err());
        assert!(remap_check("max_tool_rounds", 0.0).is_err());
        assert!(remap_check("approval_timeout_secs", 3601.0).is_err());
        assert!(remap_check("doom_threshold", 0.0).is_err());
        assert!(remap_check("doom_threshold", f64::NAN).is_err(), "NaN never valid");
    }

    #[test]
    fn remap_check_rejects_unknown_keys() {
        let err = remap_check("nope", 5.0).unwrap_err();
        assert!(err.contains("unknown setting"), "got: {err}");
        assert!(err.contains("nope"), "error names the bad key: {err}");
    }

    // ---- set-014: validate all ---------------------------------------------

    #[test]
    fn validate_all_defaults_pass_with_no_messages() {
        assert!(validate_all(&Settings::default()).is_empty(), "defaults are valid");
    }

    #[test]
    fn validate_all_reports_one_message_per_broken_field() {
        let mut s = Settings::default();
        s.approval_timeout_secs = 3601;
        let msgs = validate_all(&s);
        assert_eq!(msgs.len(), 1, "exactly one message for one bad field");
        assert!(
            msgs[0].contains("approval_timeout_secs"),
            "message names the key: {:?}",
            msgs[0]
        );
        // All three broken at once => three messages in schema order.
        let all_bad = Settings { max_tool_rounds: 0, approval_timeout_secs: 4, doom_threshold: 11 };
        let msgs = validate_all(&all_bad);
        assert_eq!(msgs.len(), 3, "one message per broken field: {msgs:?}");
        assert_eq!(
            msgs.iter().map(|m| m.split(' ').next().unwrap()).collect::<Vec<_>>(),
            known_keys(),
            "messages come in schema order"
        );
    }

    // ---- set-008 (ext): schema JSON export -------------------------------

    #[test]
    fn export_schema_json_is_valid_json_array() {
        let doc: serde_json::Value =
            serde_json::from_str(&export_schema_json()).expect("export parses as JSON");
        let arr = doc.as_array().expect("export is a JSON array");
        assert_eq!(arr.len(), schema_defs().len(), "one object per def");
        for obj in arr {
            for field in ["key", "kind", "min", "max", "default"] {
                assert!(obj.get(field).is_some(), "each entry has \"{field}\"");
            }
        }
    }

    #[test]
    fn export_schema_json_contains_every_key_and_bounds() {
        let raw = export_schema_json();
        let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for def in schema_defs() {
            let entry = doc
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["key"] == def.key)
                .unwrap_or_else(|| panic!("{} missing from export", def.key));
            assert_eq!(entry["kind"], "u64", "{} kind rendered", def.key);
            assert_eq!(entry["min"], json!(def.min), "{} min", def.key);
            assert_eq!(entry["max"], json!(def.max), "{} max", def.key);
        }
    }

    #[test]
    fn export_schema_json_renders_documented_defaults() {
        let doc: serde_json::Value = serde_json::from_str(&export_schema_json()).unwrap();
        let expected = [
            ("max_tool_rounds", 24),
            ("approval_timeout_secs", 300),
            ("doom_threshold", 3),
        ];
        for (key, default) in expected {
            let entry = doc.as_array().unwrap().iter().find(|o| o["key"] == key).unwrap();
            assert_eq!(entry["default"], json!(default), "{key} default rendered");
        }
    }

    // ---- set-010: user settings JSON export -------------------------------

    #[test]
    fn export_user_json_round_trips_through_migrate() {
        let s = Settings { max_tool_rounds: 42, approval_timeout_secs: 60, doom_threshold: 5 };
        let raw = export_user_json(&s);
        let (out, v) = migrate(&raw).expect("export is a valid versioned document");
        assert_eq!(v, SETTINGS_VERSION);
        assert_eq!(out, raw, "current-version export passes through byte-for-byte");
        let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(doc.get("version").is_some(), "version field present");
        // The exported values are the live ones.
        assert_eq!(doc["values"]["max_tool_rounds"], 42);
        assert_eq!(doc["values"]["approval_timeout_secs"], 60);
        assert_eq!(doc["values"]["doom_threshold"], 5);
    }

    #[test]
    fn export_user_json_contains_every_schema_key_and_nothing_extra() {
        let doc: serde_json::Value =
            serde_json::from_str(&export_user_json(&Settings::default())).unwrap();
        let values = doc["values"].as_object().expect("values is an object");
        for def in schema_defs() {
            assert!(values.contains_key(def.key), "{} exported", def.key);
        }
        assert_eq!(values.len(), schema_defs().len(), "no non-schema fields exported");
    }

    #[test]
    fn export_user_json_matches_defaults_on_fresh_struct() {
        let d = Settings::default();
        let doc: serde_json::Value = serde_json::from_str(&export_user_json(&d)).unwrap();
        assert_eq!(doc["version"], json!(SETTINGS_VERSION));
        assert_eq!(doc["values"]["max_tool_rounds"], json!(d.max_tool_rounds));
        assert_eq!(doc["values"]["approval_timeout_secs"], json!(d.approval_timeout_secs));
        assert_eq!(doc["values"]["doom_threshold"], json!(d.doom_threshold));
    }

    // ---- set-017: settings_export_json ------------------------------------

    #[test]
    fn settings_export_json_round_trips_through_migrate() {
        let s = Settings {
            max_tool_rounds: 9,
            approval_timeout_secs: 120,
            doom_threshold: 4,
        };
        let raw = settings_export_json(&s);
        let (out, v) = migrate(&raw).expect("export is a valid versioned document");
        assert_eq!(v, SETTINGS_VERSION);
        assert_eq!(out, raw, "current-version export passes through byte-for-byte");
        let doc: serde_json::Value =
            serde_json::from_str(&out).expect("migrated payload parses as valid JSON");
        assert_eq!(doc["version"], json!(SETTINGS_VERSION));
        assert_eq!(doc["values"]["max_tool_rounds"], json!(9));
        assert_eq!(doc["values"]["approval_timeout_secs"], json!(120));
        assert_eq!(doc["values"]["doom_threshold"], json!(4));
    }

    #[test]
    fn settings_export_json_is_pretty_and_identical_to_export_user_json() {
        let raw = settings_export_json(&Settings::default());
        assert_eq!(raw, export_user_json(&Settings::default()), "alias parity");
        let doc: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
        assert_eq!(
            doc["values"].as_object().map(|o| o.len()),
            Some(schema_defs().len()),
            "one value entry per schema def"
        );
    }

    #[test]
    fn search_index_covers_every_def_with_tokens() {
        let index = schema_search_index();
        assert_eq!(index.len(), schema_defs().len(), "one entry per def");
        assert_eq!(
            index.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            known_keys().iter().map(|k| *k).collect::<Vec<_>>(),
            "order mirrors schema_defs"
        );
        for (key, tokens) in &index {
            assert!(tokens.len() >= 2, "{key} tokenizes to >= 2 terms");
            assert!(tokens.iter().all(|t| !t.is_empty()), "{key} has no empty tokens");
        }
        // Known key: key tokens split on '_' plus the kind name.
        let (_, max_rounds) = index.iter().find(|(k, _)| k == "max_tool_rounds").expect("known key present");
        assert_eq!(max_rounds, &["max", "tool", "rounds", "u64"]);
    }

    // ---- set-012: markdown schema table -----------------------------------

    #[test]
    fn export_schema_markdown_has_header_and_every_key_row() {
        let md = export_schema_markdown();
        assert!(
            md.contains("| key | kind | min | max | default |"),
            "header row present: {md}"
        );
        for def in schema_defs() {
            assert!(md.contains(&format!("| {} |", def.key)), "{} has a row", def.key);
        }
        // Spot-check one full row renders bounds and default.
        let row = md
            .lines()
            .find(|l| l.starts_with("| max_tool_rounds "))
            .expect("max_tool_rounds row");
        assert_eq!(row.trim(), "| max_tool_rounds | u64 | 1 | 200 | 24 |");
    }

    #[test]
    fn export_schema_markdown_rows_have_five_columns() {
        let md = export_schema_markdown();
        let mut rows = 0;
        for line in md.lines() {
            let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
            assert_eq!(inner.split('|').count(), 5, "5 columns per row: {line}");
            rows += 1;
        }
        // Header + separator + one row per def.
        assert_eq!(rows, schema_defs().len() + 2);
    }

    // ---- set-013: diff from default ---------------------------------------

    #[test]
    fn diff_from_default_fresh_settings_is_empty() {
        assert!(diff_from_default(&Settings::default()).is_empty());
    }

    #[test]
    fn diff_from_default_reports_only_changed_field() {
        let mut s = Settings::default();
        s.doom_threshold = 5;
        assert_eq!(
            diff_from_default(&s),
            vec![("doom_threshold".to_string(), "5".to_string())]
        );
    }

    #[test]
    fn diff_from_default_all_changed_lists_all_keys_in_schema_order() {
        let s = Settings {
            max_tool_rounds: 1,       // default 24
            approval_timeout_secs: 2, // default 300
            doom_threshold: 7,        // default 3
        };
        let d = diff_from_default(&s);
        assert_eq!(d.len(), schema_defs().len(), "one pair per changed key");
        assert_eq!(
            d.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            known_keys(),
            "order mirrors schema_defs"
        );
        assert_eq!(
            d,
            vec![
                ("max_tool_rounds".to_string(), "1".to_string()),
                ("approval_timeout_secs".to_string(), "2".to_string()),
                ("doom_threshold".to_string(), "7".to_string()),
            ]
        );
    }

    // ---- set-025: diff as pretty JSON --------------------------------------

    #[test]
    fn settings_diff_json_fresh_defaults_is_empty_array() {
        assert_eq!(settings_diff_json(&Settings::default()), "[]");
    }

    #[test]
    fn settings_diff_json_one_changed_field_is_exact_row() {
        let mut s = Settings::default();
        s.doom_threshold = 5;
        assert_eq!(
            settings_diff_json(&s),
            "[\n  {\n    \"key\": \"doom_threshold\",\n    \"value\": \"5\"\n  }\n]"
        );
    }

    // ---- set-015: one-line summary -----------------------------------------

    #[test]
    fn settings_summary_fresh_defaults_report_zero_changed() {
        let s = settings_summary(&Settings::default());
        assert_eq!(
            s,
            format!("{} settings, 0 changed from default", schema_defs().len())
        );
        assert_eq!(s, "3 settings, 0 changed from default", "documented count");
    }

    #[test]
    fn settings_summary_counts_one_changed_field() {
        let mut s = Settings::default();
        s.approval_timeout_secs = 60;
        let out = settings_summary(&s);
        assert_eq!(out, "3 settings, 1 changed from default");
    }

    // ---- set-016: health line ----------------------------------------------

    #[test]
    fn settings_health_line_defaults_report_zero_errors() {
        let line = settings_health_line(&Settings::default());
        assert_eq!(
            line,
            format!(
                "{} | 0 validation errors",
                settings_summary(&Settings::default())
            )
        );
        assert!(line.ends_with(" | 0 validation errors"), "{line}");
        assert!(line.starts_with("3 settings"), "{line}");
    }

    #[test]
    fn settings_health_line_counts_broken_fields() {
        let mut s = Settings::default();
        s.approval_timeout_secs = 3601;
        assert_eq!(
            settings_health_line(&s),
            "3 settings, 1 changed from default | 1 validation errors"
        );
        // Two broken fields => count tracks validate_all.
        s.doom_threshold = 11;
        assert_eq!(
            settings_health_line(&s),
            "3 settings, 2 changed from default | 2 validation errors"
        );
    }

    // ---- set-023: health JSON ------------------------------------------------

    #[test]
    fn settings_health_json_defaults_report_zero_errors() {
        let doc: serde_json::Value =
            serde_json::from_str(&settings_health_json(&Settings::default()))
                .expect("health JSON parses");
        assert_eq!(
            doc["summary"],
            json!(settings_summary(&Settings::default())),
            "summary half comes from settings_summary"
        );
        assert_eq!(doc["errors"], json!([]), "defaults => 0 errors");
    }

    #[test]
    fn settings_health_json_reports_one_error_for_broken_field() {
        let mut s = Settings::default();
        s.approval_timeout_secs = 3601;
        let doc: serde_json::Value =
            serde_json::from_str(&settings_health_json(&s)).expect("health JSON parses");
        assert_eq!(doc["summary"], json!("3 settings, 1 changed from default"));
        let errs = doc["errors"].as_array().expect("errors is an array");
        assert_eq!(errs.len(), 1, "one broken field => one error");
        assert!(
            errs[0].as_str().is_some_and(|m| m.contains("approval_timeout_secs")),
            "message names the key: {errs:?}"
        );
    }

    // ---- set-018: settings by prefix ---------------------------------------

    #[test]
    fn settings_by_prefix_matches_known_prefixes_case_insensitively() {
        assert_eq!(settings_by_prefix("max_"), vec!["max_tool_rounds"]);
        assert_eq!(settings_by_prefix("doom"), vec!["doom_threshold"]);
        assert_eq!(
            settings_by_prefix("APPROVAL"),
            vec!["approval_timeout_secs"],
            "uppercase prefix still matches"
        );
    }

    #[test]
    fn settings_by_prefix_empty_prefix_returns_all_keys() {
        assert_eq!(settings_by_prefix(""), known_keys(), "empty prefix ⇒ every key");
    }

    #[test]
    fn settings_by_prefix_no_match_is_empty() {
        assert!(settings_by_prefix("zzz_").is_empty());
    }

    // ---- set-019: schema kind counts ---------------------------------------

    #[test]
    fn schema_kind_counts_matches_current_schema_exactly() {
        // Current schema is all-u64: u64 leads, zero-count kinds follow in
        // DefKind declaration order.
        let u64_count = schema_defs().iter().filter(|d| d.kind == DefKind::U64).count();
        assert_eq!(
            schema_kind_counts(),
            vec![
                ("u64".to_string(), u64_count),
                ("f32".to_string(), 0),
                ("bool".to_string(), 0),
                ("string".to_string(), 0),
            ]
        );
    }

    #[test]
    fn schema_kind_counts_is_non_empty_and_sums_to_schema_len() {
        let counts = schema_kind_counts();
        assert!(!counts.is_empty());
        assert_eq!(counts.iter().map(|(_, n)| n).sum::<usize>(), schema_defs().len());
        // Sorted descending by count.
        let ns: Vec<usize> = counts.iter().map(|(_, n)| *n).collect();
        let mut sorted = ns.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(ns, sorted);
    }

    // ---- set-024: schema stats JSON -----------------------------------------

    #[test]
    fn settings_schema_stats_json_matches_schema_kind_counts_exactly() {
        let counts = schema_kind_counts();
        let doc: serde_json::Value =
            serde_json::from_str(&settings_schema_stats_json()).expect("stats JSON parses");
        assert_eq!(
            doc["total"],
            json!(counts.iter().map(|(_, n)| n).sum::<usize>()),
            "total equals the sum of kind counts"
        );
        assert_eq!(
            doc["total"],
            json!(schema_defs().len()),
            "total equals schema size"
        );
        let by_kind = doc["by_kind"].as_object().expect("by_kind is an object");
        assert_eq!(by_kind.len(), counts.len(), "one entry per kind");
        for (kind, n) in &counts {
            assert_eq!(by_kind[kind.as_str()], json!(n), "{kind} count matches");
        }
    }

    #[test]
    fn settings_schema_stats_json_parses_and_has_expected_shape() {
        let doc: serde_json::Value =
            serde_json::from_str(&settings_schema_stats_json()).expect("valid JSON");
        assert!(doc.get("total").is_some(), "total field present");
        assert!(doc["total"].is_u64(), "total is a number");
        assert!(doc["by_kind"].is_object(), "by_kind is an object");
    }

    // ---- set-020: defaults JSON ---------------------------------------------

    #[test]
    fn settings_defaults_json_parses_as_an_object() {
        let doc: serde_json::Value = serde_json::from_str(&settings_defaults_json())
            .expect("settings_defaults_json is valid JSON");
        assert!(
            doc.as_object().is_some(),
            "top-level shape must be an object"
        );
    }

    #[test]
    fn settings_defaults_json_contains_every_schema_key() {
        let doc: serde_json::Value =
            serde_json::from_str(&settings_defaults_json()).expect("valid JSON");
        assert_eq!(
            doc.as_object().map(|o| o.len()),
            Some(known_keys().len()),
            "one entry per schema key"
        );
        for key in known_keys() {
            assert!(doc.get(key).is_some(), "missing schema key {key}");
        }
    }

    #[test]
    fn settings_defaults_json_matches_settings_default_values() {
        let d = Settings::default();
        let doc: serde_json::Value =
            serde_json::from_str(&settings_defaults_json()).expect("valid JSON");
        assert_eq!(
            doc.get("max_tool_rounds").and_then(|v| v.as_str()),
            Some(d.max_tool_rounds.to_string()).as_deref()
        );
        assert_eq!(
            doc.get("approval_timeout_secs").and_then(|v| v.as_str()),
            Some(d.approval_timeout_secs.to_string()).as_deref()
        );
        assert_eq!(
            doc.get("doom_threshold").and_then(|v| v.as_str()),
            Some(d.doom_threshold.to_string()).as_deref()
        );
    }

    // ---- set-021: validation JSON ------------------------------------------

    #[test]
    fn settings_validate_json_defaults_report_valid_with_empty_errors() {
        let doc: serde_json::Value =
            serde_json::from_str(&settings_validate_json(&Settings::default()))
                .expect("report parses as valid JSON");
        assert_eq!(doc["valid"], json!(true));
        assert_eq!(doc["errors"], json!([]));
    }

    #[test]
    fn settings_validate_json_broken_field_reports_invalid_with_message() {
        let mut s = Settings::default();
        s.approval_timeout_secs = 3601;
        let doc: serde_json::Value =
            serde_json::from_str(&settings_validate_json(&s)).expect("report parses as valid JSON");
        assert_eq!(doc["valid"], json!(false));
        let errs = doc["errors"].as_array().expect("errors is an array");
        assert_eq!(errs.len(), 1, "one message for one broken field");
        assert!(
            errs[0].as_str().is_some_and(|m| m.contains("approval_timeout_secs")),
            "message names the key: {errs:?}"
        );
    }

    // ---- set-022: schema JSONL ----------------------------------------------

    #[test]
    fn schema_defs_jsonl_has_one_valid_line_per_def() {
        let out = schema_defs_jsonl();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), schema_defs().len(), "one line per def");
        for line in &lines {
            let obj: serde_json::Value = serde_json::from_str(line).expect("line parses as JSON");
            assert!(obj.is_object(), "each line is a JSON object: {line}");
            for field in ["key", "kind", "min", "max", "default"] {
                assert!(obj.get(field).is_some(), "line carries \"{field}\": {line}");
            }
        }
    }

    #[test]
    fn schema_defs_jsonl_parses_back_to_same_count_in_schema_order() {
        let parsed: Vec<serde_json::Value> = schema_defs_jsonl()
            .lines()
            .map(|l| serde_json::from_str(l).expect("valid JSONL"))
            .collect();
        assert_eq!(parsed.len(), schema_defs().len(), "round-trip count matches");
        assert_eq!(
            parsed.iter().filter_map(|o| o["key"].as_str()).collect::<Vec<_>>(),
            known_keys(),
            "keys round-trip in schema order"
        );
    }

    #[test]
    fn schema_defs_jsonl_rows_match_export_schema_json_exactly() {
        // Same rows as the pretty-array export, just compact — one shared
        // renderer means the two exports cannot drift.
        let doc: serde_json::Value = serde_json::from_str(&export_schema_json()).unwrap();
        let rows: Vec<serde_json::Value> = schema_defs_jsonl()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(doc.as_array().expect("pretty export is an array"), &rows);
    }
}
