//! Tool runtime — built-in tools with risk classification and scope checks.
//!
//! Every tool declares its [`Risk`]. Read-only tools are auto-allowed inside
//! the project root; writes and command execution require explicit approval
//! through the runtime's approval gate. Paths are canonicalised and checked
//! against the project root so a model-supplied `..\..` cannot escape scope.

use crate::provider::ToolDef;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use z_protocol::Risk;

pub struct ToolOutput {
    pub ok: bool,
    /// What the model sees. Kept bounded so one tool call cannot eat the
    /// context budget.
    pub text: String,
}

/// Cap on tool output fed back to the model (characters).
const MAX_OUTPUT_CHARS: usize = 12_000;

fn bound(text: String) -> String {
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        return text;
    }
    let head: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
    format!("{head}

…[output truncated]")
}

// tok-003: process-wide tool-output cache. Cache ONLY fs_read — fs_search and
// fs_list results depend on many files and stay uncached until per-directory
// invalidation exists. Key is (tool, args-key, fingerprint-of-current-bytes):
// because the fingerprint is recomputed on every call, a changed file lands on
// a different key (tok-005's invalidation is structural — no watcher, no
// delete logic, no stale serve possible).
const TOOL_CACHE_CAP: usize = 128;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

type ToolCacheKey = (String, String, u64); // (tool, root+raw-path arg, fingerprint)

fn tool_cache() -> &'static Mutex<HashMap<ToolCacheKey, String>> {
    static CACHE: OnceLock<Mutex<HashMap<ToolCacheKey, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// tok-012: cumulative fs_read cache hit/miss counters (monotonic, lock-free).
#[derive(Debug)]
pub struct CacheMetrics {
    pub hits: u64,
    pub misses: u64,
}

static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

pub fn cache_metrics() -> CacheMetrics {
    CacheMetrics {
        hits: CACHE_HITS.load(Ordering::Relaxed),
        misses: CACHE_MISSES.load(Ordering::Relaxed),
    }
}

/// A resolved tool execution request.
pub struct ToolInvocation<'a> {
    pub name: &'a str,
    pub args: Value,
    pub project_root: &'a Path,
    /// Owning conversation thread, for per-thread read fingerprints
    /// (edit-002). Empty string in tests that don't care.
    pub thread_id: &'a str,
}

/// Classify a tool call's risk before it runs.
pub fn classify(name: &str, args: &Value) -> Risk {
    match name {
        "fs_read" | "fs_list" | "fs_search" | "git_status" | "git_diff" | "git_log" => {
            Risk::ReadOnly
        }
        "fs_write" | "edit_patch" => Risk::Write,
        "terminal_exec" => Risk::Execute,
        _ => {
            // Unknown tools are treated as execute-risk: fail closed.
            let _ = args;
            Risk::Execute
        }
    }
}

/// Human-readable detail for events and approval prompts.
pub fn describe(name: &str, args: &Value) -> String {
    match name {
        "fs_read" => fmt_arg(args, "path"),
        "fs_list" => fmt_arg(args, "path"),
        "fs_search" => format!("{} in {}", fmt_arg(args, "query"), fmt_arg(args, "path")),
        "fs_write" => fmt_arg(args, "path"),
        "edit_patch" => {
            let n = args.get("blocks").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
            format!("{} block(s) in {}", n, fmt_arg(args, "path"))
        }
        "terminal_exec" => fmt_arg(args, "command"),
        "git_status" | "git_diff" => fmt_arg(args, "path"),
        "git_log" => {
            let n = args.get("limit").and_then(Value::as_u64).unwrap_or(20);
            format!("last {n} commits")
        }
        _ => name.to_string(),
    }
}

fn fmt_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(Value::as_str).unwrap_or("?").to_string()
}

/// JSON-schema definitions advertised to the model.
pub fn definitions() -> Vec<ToolDef> {
    fn obj(props: Value, required: &[&str]) -> Value {
        json!({"type":"object","properties":props,"required":required})
    }
    vec![
        ToolDef {
            name: "fs_read".into(),
            description: "Read a text file inside the project. Returns the file content.".into(),
            parameters: obj(
                json!({"path":{"type":"string","description":"Project-relative or absolute path"}}),
                &["path"],
            ),
        },
        ToolDef {
            name: "fs_list".into(),
            description: "List a directory inside the project: entries with sizes.".into(),
            parameters: obj(
                json!({"path":{"type":"string","description":"Directory path, '.' for the project root"}}),
                &["path"],
            ),
        },
        ToolDef {
            name: "fs_search".into(),
            description:
                "Search file contents for a literal query; returns matching lines with paths."
                    .into(),
            parameters: obj(
                json!({
                    "query":{"type":"string"},
                    "path":{"type":"string","description":"Directory to search, default '.'"}
                }),
                &["query"],
            ),
        },
        ToolDef {
            name: "fs_write".into(),
            description:
                "Create or overwrite a text file inside the project with the given content.".into(),
            parameters: obj(
                json!({"path":{"type":"string"},"content":{"type":"string"}}),
                &["path", "content"],
            ),
        },
        ToolDef {
            name: "edit_patch".into(),
            description:
                "Apply sequential search/replace blocks to a text file inside the project. \
                 Each block's old text must match exactly (whitespace-tolerant fallback with a \
                 note); if any block fails to match nothing is written."
                    .into(),
            parameters: obj(
                json!({
                    "path":{"type":"string"},
                    "blocks":{"type":"array","items":{
                        "type":"object",
                        "properties":{"old":{"type":"string","description":"Exact existing text to find"},
                                      "new":{"type":"string","description":"Replacement text"}},
                        "required":["old","new"]}
                    }
                }),
                &["path", "blocks"],
            ),
        },
        ToolDef {
            name: "terminal_exec".into(),
            description:
                "Run a shell command in the project directory. Returns stdout/stderr and the exit code. The process tree is killed automatically if it exceeds the timeout."
                    .into(),
            parameters: obj(
                json!({
                    "command":{"type":"string","description":"Shell command line"},
                    "timeout_ms":{"type":"integer","description":"Optional wall-clock budget in milliseconds; default 120000, hard maximum 600000"}
                }),
                &["command"],
            ),
        },
        ToolDef {
            name: "git_status".into(),
            description:
                "Show the current git branch and working-tree changes (read-only summary).".into(),
            parameters: obj(
                json!({"path":{"type":"string","description":"Repo directory inside the project, default '.'"}}),
                &[],
            ),
        },
        ToolDef {
            name: "git_diff".into(),
            description:
                "Summarise unstaged changes against the index: added/deleted line counts per file."
                    .into(),
            parameters: obj(
                json!({"path":{"type":"string","description":"Repo directory inside the project, default '.'"}}),
                &[],
            ),
        },
        ToolDef {
            name: "git_log".into(),
            description: "List recent commits: short hash, author, unix timestamp, subject.".into(),
            parameters: obj(
                json!({"limit":{"type":"integer","description":"Max commits, clamped to 1..=100, default 20"}}),
                &[],
            ),
        },
    ]
}

/// tok-021: per-turn lazy manifest filter. The caller passes the user's
/// current request as `ctx_hint`; a tool whose name appears in the hint
/// stays listed, and an empty hint lists every tool. Everything else is
/// filtered out so irrelevant tool schemas never enter the prompt
/// (token economy).
pub fn should_list(tool_name: &str, ctx_hint: &str) -> bool {
    ctx_hint.is_empty() || ctx_hint.to_lowercase().contains(&tool_name.to_lowercase())
}

/// tok-021: filter a manifest of tool names against `ctx_hint`,
/// preserving input order.
pub fn filter_manifest<'a>(tools: &[&'a str], ctx_hint: &str) -> Vec<&'a str> {
    tools.iter().copied().filter(|t| should_list(t, ctx_hint)).collect()
}

/// Resolve `path` against the project root and refuse escapes. Symlinks that
/// resolve outside the root are rejected too — the check runs after
/// canonicalisation, not on the raw string.
fn scoped(root: &Path, raw: &str) -> Result<PathBuf, String> {
    let root_canonical =
        root.canonicalize().map_err(|e| format!("project root invalid: {e}"))?;
    // Work against the readable form of the root so results compare cleanly
    // with user-supplied paths instead of Windows verbatim paths.
    let root_norm = strip_verbatim(&root_canonical);

    let candidate = Path::new(raw);
    let base =
        if candidate.is_absolute() { candidate.to_path_buf() } else { root_norm.join(candidate) };

    // Lexical normalisation: resolve `.` and `..` without touching the
    // filesystem, so a not-yet-existing write target is checkable too and
    // traversal can never survive into the final answer.
    let mut normalized = PathBuf::new();
    for component in base.components() {
        match component {
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!("path {raw:?} is outside the project scope"));
                }
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }

    if !normalized.starts_with(&root_norm) {
        return Err(format!("path {raw:?} is outside the project scope"));
    }
    Ok(normalized)
}

/// Remove the `\\?\` prefix Windows canonicalisation adds.
fn strip_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    // Build the verbatim prefix from char codes so the literal can never
    // be corrupted by editor or tooling escape handling.
    let prefix: String =
        [char::from(92), char::from(92), '?', char::from(92)].iter().collect();
    match text.strip_prefix(prefix.as_str()) {
        Some(rest) => PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

/// Canonical registry key for a model-supplied path (edit-016): scope-checked
/// first so escapes are refused, then canonicalised when the file exists so
/// symlink aliases collide into one key. Not-yet-existing targets fall back
/// to the lexical form, which is still traversal-free after `scoped`.
pub(crate) fn canonical_key(root: &Path, raw: &str) -> Result<PathBuf, String> {
    let path = scoped(root, raw)?;
    Ok(path.canonicalize().unwrap_or(path))
}

/// Sibling staging file holding the pre-write bytes of `target` (edit-014).
/// pid-suffixed like the atomic_write temps; one generation per target.
fn rollback_temp_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "target".into());
    target.with_file_name(format!(".{name}.{}.rollback.tmp", std::process::id()))
}

/// edit-014: restore the staged old bytes over `target`, consuming the
/// staging copy. Errors when nothing is staged (no prior write over an
/// existing file in this process).
pub fn rollback_last(target: &Path) -> Result<(), String> {
    let tmp = rollback_temp_path(target);
    if !tmp.exists() {
        return Err(format!("no staged rollback for {}", target.display()));
    }
    std::fs::rename(&tmp, target).map_err(|e| e.to_string())
}

/// Execute an approved tool call. This is the only place the core touches the
/// filesystem or spawns processes.
pub fn execute(inv: ToolInvocation) -> ToolOutput {
    match inv.name {
        "fs_read" => fs_read(&inv),
        "fs_list" => fs_list(&inv),
        "fs_search" => fs_search(&inv),
        "fs_write" => fs_write(&inv),
        "edit_patch" => edit_patch(&inv),
        "terminal_exec" => terminal_exec(&inv),
        "git_status" => git_status(&inv),
        "git_diff" => git_diff(&inv),
        "git_log" => git_log(&inv),
        other => ToolOutput { ok: false, text: format!("unknown tool {other:?}") },
    }
}

fn fs_read(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let raw = fmt_arg(&inv.args, "path");
        let path = scoped(inv.project_root, raw.as_str())?;
        let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if meta.len() > 512 * 1024 {
            return Err("file larger than 512 KiB; read it in parts via terminal_exec".into());
        }
        // tok-003/005: the fingerprint of the CURRENT bytes is recomputed on
        // every call and is part of the cache key — a changed file simply
        // lands on a different key, so a stale serve is structurally
        // impossible (the fingerprint check IS the invalidation).
        let fp = crate::fingerprint::file_fingerprint(&path)?;
        // tok-020: the registry already holding this thread's identical
        // fingerprint means the model has seen exactly these bytes before.
        let duplicate = !inv.thread_id.is_empty()
            && crate::fingerprint::peek_fingerprint(inv.thread_id, &raw) == Some(fp);
        let key = (
            "fs_read".to_string(),
            format!("{}\u{0}{}", inv.project_root.display(), raw),
            fp,
        );
        // Bind first: a match scrutinee's temporaries (the lock guard here)
        // would otherwise live to the end of the match and deadlock the
        // miss path's re-lock below.
        let cached = tool_cache().lock().unwrap().get(&key).cloned();
        let mut text = match cached {
            Some(cached) => {
                CACHE_HITS.fetch_add(1, Ordering::Relaxed);
                format!("[cached] {cached}")
            }
            None => {
                CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
                let content =
                    bound(std::fs::read_to_string(&path).map_err(|e| e.to_string())?);
                let mut cache = tool_cache().lock().unwrap();
                // ponytail: clear-all on overflow instead of oldest-entry
                // eviction — simplest correct cap; upgrade if churn ever
                // measurably hurts hit rate at personal scale.
                if cache.len() >= TOOL_CACHE_CAP && !cache.contains_key(&key) {
                    cache.clear();
                }
                cache.insert(key, content.clone());
                content
            }
        };
        // edit-002 recording preserved verbatim on both paths so the
        // write-refusal pipeline is untouched.
        if !inv.thread_id.is_empty() {
            crate::fingerprint::record_fingerprint(inv.thread_id, &raw, fp);
        }
        if duplicate {
            text.push_str(" (duplicate read of unchanged file)");
        }
        Ok(text)
    })();
    match result {
        Ok(content) => ToolOutput { ok: true, text: content },
        Err(e) => ToolOutput { ok: false, text: format!("fs_read failed: {e}") },
    }
}

fn fs_list(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let raw = inv.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = scoped(inv.project_root, raw)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = if entry.path().is_dir() { "dir" } else { "file" };
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            entries.push(format!("{kind}  {name}  ({size} B)"));
        }
        entries.sort();
        Ok(entries.join("
"))
    })();
    match result {
        Ok(listing) => ToolOutput { ok: true, text: bound(listing) },
        Err(e) => ToolOutput { ok: false, text: format!("fs_list failed: {e}") },
    }
}

fn fs_search(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let query = fmt_arg(&inv.args, "query");
        if query.is_empty() {
            return Err("empty query".into());
        }
        let raw = inv.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let dir = scoped(inv.project_root, raw)?;
        let mut hits = Vec::new();
        walk_files(&dir, &mut |path| {
            if hits.len() >= 200 {
                return;
            }
            if let Ok(content) = std::fs::read_to_string(path) {
                for (i, line) in content.lines().enumerate() {
                    if line.contains(&query) {
                        let rel = path.strip_prefix(inv.project_root).unwrap_or(path);
                        // Forward slashes keep results uniform across platforms.
                        let rel_text = rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
                        hits.push(format!("{}:{}: {}", rel_text, i + 1, line.trim()));
                        if hits.len() >= 200 {
                            break;
                        }
                    }
                }
            }
        });
        Ok(if hits.is_empty() {
            format!("no matches for {query:?}")
        } else {
            hits.join("
")
        })
    })();
    match result {
        Ok(text) => ToolOutput { ok: true, text: bound(text) },
        Err(e) => ToolOutput { ok: false, text: format!("fs_search failed: {e}") },
    }
}

/// Shared safety path behind every content-writing tool (ADR-0010 #1/#2):
/// scope check → fingerprint stale check (ZD-E-0060) → parent-dir creation →
/// atomic temp+rename write (edit-004/005) → fingerprint re-arm. `raw_path`
/// must be the model-supplied string so the registry lookup matches fs_read's.
fn checked_write(inv: &ToolInvocation, raw_path: &str, bytes: &[u8]) -> Result<(), String> {
    let path = scoped(inv.project_root, raw_path)?;
    // edit-003 (ZD-E-0060): if this thread read the file before, the
    // on-disk content must still match what it saw. Never-read files
    // stay writable for now (blind writes; edit-018 flags them later).
    if !inv.thread_id.is_empty() {
        if let Some(expected) = crate::fingerprint::take_fingerprint(inv.thread_id, raw_path) {
            match crate::fingerprint::file_fingerprint(&path) {
                Ok(current) if current == expected => {}
                _ => {
                    return Err(
                        "This file changed since it was read. Re-read it before editing.".into(),
                    );
                }
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // edit-014: stage the current bytes BEFORE touching the target so a
    // later rollback_last can restore them exactly. Rewriting the staging
    // file through its own atomic write replaces any previous generation,
    // so exactly one rollback copy stays alive per target. A read failure
    // here refuses the write: unstageable data must not be destroyed.
    if path.exists() {
        let old = std::fs::read(&path).map_err(|e| e.to_string())?;
        crate::atomic_write::atomic_write(&rollback_temp_path(&path), &old)?;
    }
    // edit-005: route through the atomic temp+rename helper (ADR-0010)
    // so a crash mid-write leaves old-or-new, never a truncated file.
    crate::atomic_write::atomic_write(&path, bytes)?;
    // Re-arm so consecutive agent writes don't trip against their own output.
    if !inv.thread_id.is_empty() {
        if let Ok(fp) = crate::fingerprint::file_fingerprint(&path) {
            crate::fingerprint::record_fingerprint(inv.thread_id, raw_path, fp);
        }
    }
    Ok(())
}

fn fs_write(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let raw = fmt_arg(&inv.args, "path");
        let content = inv.args.get("content").and_then(Value::as_str).unwrap_or("");
        checked_write(inv, &raw, content.as_bytes())?;
        Ok(format!("wrote {} bytes to {}", content.len(), raw))
    })();
    match result {
        Ok(text) => ToolOutput { ok: true, text },
        Err(e) => ToolOutput { ok: false, text: format!("fs_write failed: {e}") },
    }
}

/// edit_patch (edit-008..013, ADR-0010 #2): search/replace blocks applied
/// sequentially against an IN-MEMORY copy of the file; any failed anchor
/// aborts the whole patch before disk contact, so failure leaves zero
/// partial state. Only after every block resolves does the buffer go
/// through the same checked_write safety path as fs_write.
fn edit_patch(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let raw = fmt_arg(&inv.args, "path");
        let arr = inv
            .args
            .get("blocks")
            .and_then(Value::as_array)
            .ok_or("missing blocks array")?;
        let mut blocks: Vec<(&str, &str)> = Vec::with_capacity(arr.len());
        for b in arr {
            let old = b.get("old").and_then(Value::as_str).ok_or("each block needs \"old\"")?;
            let new = b.get("new").and_then(Value::as_str).ok_or("each block needs \"new\"")?;
            blocks.push((old, new));
        }
        let path = scoped(inv.project_root, &raw)?;
        let mut content = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {raw}: {e}"))?;
        let mut normalized_anywhere = false;
        for (i, (old, new)) in blocks.iter().enumerate() {
            match apply_block(&content, old, new) {
                Some((next, used_normalization)) => {
                    content = next;
                    normalized_anywhere |= used_normalization;
                }
                None => {
                    // ZD-E-0061: absent anchor fails the WHOLE patch; nothing
                    // has touched disk because application was pure string work.
                    return Err(format!(
                        "Patch failed: block {} anchor not found — \
                         The text to replace was not found in the file. No changes were applied.",
                        i + 1
                    ));
                }
            }
        }
        checked_write(inv, &raw, content.as_bytes())?;
        let note = if normalized_anywhere { " (whitespace-normalized)" } else { "" };
        Ok(format!("applied {} block(s){note} to {}", blocks.len(), raw))
    })();
    match result {
        Ok(text) => ToolOutput { ok: true, text },
        Err(e) => ToolOutput { ok: false, text: e },
    }
}

/// Apply one search/replace block. Exact substring match first; otherwise a
/// whitespace-normalised line match whose splice uses the ORIGINAL span
/// boundaries, so untouched bytes keep their exact formatting. Returns
/// `(new_content, used_normalization)` or `None` when the anchor is absent.
fn apply_block(content: &str, old: &str, new: &str) -> Option<(String, bool)> {
    if old.is_empty() {
        // An empty anchor identifies nothing; refusing beats replacing at 0.
        return None;
    }
    if let Some(pos) = content.find(old) {
        let mut next = String::with_capacity(content.len());
        next.push_str(&content[..pos]);
        next.push_str(new);
        next.push_str(&content[pos + old.len()..]);
        return Some((next, false));
    }
    // Whitespace-normalised fallback: compare per line with runs of
    // spaces/tabs collapsed and edges trimmed (ADR-0010 option 2c), then
    // splice over the matched line window's original span.
    let anchor: Vec<String> = old.lines().map(norm_line).collect();
    let n = anchor.len();
    if n == 0 {
        return None;
    }
    let total_len = content.len();
    let mut starts: Vec<usize> = vec![0];
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    let line_count = starts.len();
    // Byte end of line k, excluding its newline (or EOF).
    let line_end = |k: usize| if k + 1 < line_count { starts[k + 1] - 1 } else { total_len };
    for w in 0..line_count.saturating_sub(n - 1) {
        if (0..n).all(|k| norm_line(&content[starts[w + k]..line_end(w + k)]) == anchor[k]) {
            let s = starts[w];
            let e = line_end(w + n - 1);
            let mut next = String::with_capacity(content.len());
            next.push_str(&content[..s]);
            next.push_str(new);
            next.push_str(&content[e..]);
            return Some((next, true));
        }
    }
    None
}

/// Collapse runs of spaces/tabs to one space and trim edges; other
/// characters compare exactly.
fn norm_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut sep = false;
    for tok in line.split(|c| c == ' ' || c == '\t') {
        if tok.is_empty() {
            continue;
        }
        if sep {
            out.push(' ');
        }
        out.push_str(tok);
        sep = true;
    }
    out
}

fn terminal_exec(inv: &ToolInvocation) -> ToolOutput {
    let command = fmt_arg(&inv.args, "command");
    if command.trim().is_empty() {
        return ToolOutput { ok: false, text: "terminal_exec failed: empty command".into() };
    }
    // Optional per-call budget, clamped by the sandbox's hard ceiling.
    let timeout = inv
        .args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .map(std::time::Duration::from_millis);
    match crate::sandbox::run(&command, inv.project_root, timeout) {
        Ok(outcome) => {
            // Command output can echo environment secrets; nothing leaves the
            // tool boundary unredacted.
            let stdout = crate::redact::redact(&outcome.stdout);
            let stderr = crate::redact::redact(&outcome.stderr);
            let mut text = String::new();
            if !stdout.trim().is_empty() {
                text.push_str(stdout.trim_end());
                text.push('\n');
            }
            if !stderr.trim().is_empty() {
                text.push_str("[stderr] ");
                text.push_str(stderr.trim_end());
                text.push('\n');
            }
            if outcome.timed_out {
                text.push_str("[killed: process tree exceeded its time budget]");
                text.push(char::from(10));
            }
            text.push_str(&format!("[exit code: {}]", outcome.code.unwrap_or(-1)));
            ToolOutput { ok: !outcome.timed_out && outcome.code == Some(0), text: bound(text) }
        }
        Err(e) => ToolOutput { ok: false, text: format!("terminal_exec failed: {e}") },
    }
}

/// Single git facade (ADR-0008): direct argv via `std::process::Command`,
/// never shell strings; machine-readable output flags only; `LC_ALL=C` set
/// defensively; the exit code is authoritative. This is the sole place the
/// core spawns git (edit-028 will pin that invariant).
fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        // Reads must never refresh or take the index lock (ADR-0008 §3).
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|_| "not a git repository (or git not found)".to_string())?;
    if !out.status.success() {
        let detail = String::from_utf8_lossy(&out.stderr);
        let detail = detail.trim();
        return Err(if detail.is_empty() {
            "not a git repository (or git not found)".into()
        } else {
            detail.to_string()
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn git_status(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let raw = inv.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let dir = scoped(inv.project_root, raw)?;
        let out = run_git(&dir, &["status", "--porcelain=v2", "--branch", "-z"])?;
        // edit-022: decode `--porcelain=v2 -z`. Records are NUL-separated;
        // rename/unmerged entries carry extra path fields as their own NUL
        // tokens, which fail the shape checks below and are skipped.
        let mut branch = String::from("(unknown)");
        let mut ahead_behind = String::new();
        let mut entries: Vec<String> = Vec::new();
        for rec in out.split('\0').filter(|r| !r.is_empty()) {
            if let Some(rest) = rec.strip_prefix("# ") {
                if let Some(name) = rest.strip_prefix("branch.head ") {
                    branch = name.to_string();
                } else if let Some(ab) = rest.strip_prefix("branch.ab ") {
                    // "+1 -2" -> "ahead 1, behind 2"
                    ahead_behind = ab.replace('+', "ahead ").replace('-', ", behind ");
                }
                continue;
            }
            let (code, path) = match rec.split_once(' ') {
                Some(("?", p)) | Some(("!", p)) => ("??", p),
                Some((kind @ ("1" | "2" | "u"), _)) => {
                    // Fixed-field prefix lengths before the path:
                    // 1 -> 9 fields, 2 (rename) -> 10, u (unmerged) -> 11.
                    let fixed = match kind {
                        "1" => 9,
                        "2" => 10,
                        _ => 11,
                    };
                    let mut parts = rec.splitn(fixed, ' ');
                    let xy = parts.nth(1).unwrap_or("");
                    (xy, parts.last().unwrap_or(""))
                }
                _ => continue,
            };
            entries.push(format!("{} {}", code.replace('.', " "), path));
        }
        let mut text = format!("branch {branch}");
        if !ahead_behind.is_empty() {
            text.push_str(&format!(" [{ahead_behind}]"));
        }
        if entries.is_empty() {
            text.push_str("\nclean");
        } else {
            for e in entries.iter().take(100) {
                text.push('\n');
                text.push_str(e);
            }
            if entries.len() > 100 {
                text.push_str(&format!("\n+{} more", entries.len() - 100));
            }
        }
        Ok(text)
    })();
    match result {
        Ok(text) => ToolOutput { ok: true, text },
        Err(e) => ToolOutput { ok: false, text: format!("git_status failed: {e}") },
    }
}

fn is_numstat_count(s: &str) -> bool {
    s == "-" || s.bytes().all(|b| b.is_ascii_digit())
}

fn git_diff(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let raw = inv.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let dir = scoped(inv.project_root, raw)?;
        let out = run_git(&dir, &["diff", "--numstat", "-z"])?;
        // edit-023: numstat -z records are "<added>\t<deleted>\t<path>",
        // NUL-terminated; rename records repeat the original path as an
        // extra NUL token without count fields, failing the shape check.
        let mut lines: Vec<String> = Vec::new();
        for rec in out.split('\0').filter(|r| !r.is_empty()) {
            let f: Vec<&str> = rec.split('\t').collect();
            if f.len() < 3 || !is_numstat_count(f[0]) || !is_numstat_count(f[1]) {
                continue;
            }
            lines.push(format!("{} {} {}", f[0], f[1], f[2]));
        }
        let mut text = String::new();
        for l in lines.iter().take(200) {
            text.push_str(l);
            text.push('\n');
        }
        text.push_str(&format!("{} files changed", lines.len()));
        Ok(text)
    })();
    match result {
        Ok(text) => ToolOutput { ok: true, text },
        Err(e) => ToolOutput { ok: false, text: format!("git_diff failed: {e}") },
    }
}

fn git_log(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let raw = inv.args.get("path").and_then(Value::as_str).unwrap_or(".");
        let dir = scoped(inv.project_root, raw)?;
        let limit =
            inv.args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 100).to_string();
        let out =
            run_git(&dir, &["log", "-n", limit.as_str(), "--format=%H%x00%h%x00%an%x00%at%x00%s%x00"])?;
        // edit-024: each commit emits five NUL-separated tokens (full hash,
        // short hash, author, unix time, subject) and the trailing %x00 ends
        // the record, so chunks-of-five decode it directly.
        let mut lines: Vec<String> = Vec::new();
        for c in out.split('\0').collect::<Vec<_>>().chunks(5) {
            if c.len() == 5 && !c[0].is_empty() {
                lines.push(format!("{} {} {} {}", c[1], c[2], c[3], c[4]));
            }
        }
        Ok(lines.join("\n"))
    })();
    match result {
        Ok(text) => ToolOutput { ok: true, text },
        Err(e) => ToolOutput { ok: false, text: format!("git_log failed: {e}") },
    }
}

/// Recursive file walker used by search. Skips dependency/build directories.
pub fn walk_files(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if matches!(
                name.as_str(),
                ".git" | "node_modules" | "target" | "dist" | "build" | "__pycache__" | ".next"
                    | "venv" | ".venv"
            ) {
                continue;
            }
            walk_files(&path, visit);
        } else if path.is_file() {
            visit(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zcore-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scoped_rejects_path_traversal_outside_the_project() {
        let root = temp_root("scope");
        let err = scoped(&root, "../outside.txt").unwrap_err();
        assert!(err.contains("outside the project scope"), "{err}");
        assert!(scoped(&root, ".").is_ok());
    }

    #[test]
    fn unknown_tools_fail_closed_as_execute_risk() {
        assert_eq!(classify("mystery_tool", &json!({})), Risk::Execute);
        assert_eq!(classify("fs_read", &json!({})), Risk::ReadOnly);
        assert_eq!(classify("fs_write", &json!({})), Risk::Write);
    }

    #[test]
    fn write_then_read_round_trips_inside_scope() {
        let root = temp_root("rw");
        std::fs::write(root.join("hello.txt"), "hi").unwrap();
        let out = execute(ToolInvocation {
            name: "fs_read",
            args: json!({"path": "hello.txt"}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert_eq!(out.text, "hi");

        let out = execute(ToolInvocation {
            name: "fs_write",
            args: json!({"path": "sub/new.txt", "content": "created"}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert_eq!(std::fs::read_to_string(root.join("sub/new.txt")).unwrap(), "created");
    }

    #[test]
    fn search_finds_matches_with_paths_and_lines() {
        let root = temp_root("search");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn main() {}
// TODO fix
").unwrap();
        let out = execute(ToolInvocation {
            name: "fs_search",
            args: json!({"query": "TODO"}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok);
        assert!(out.text.contains("src/a.rs:2"), "{}", out.text);
    }

    #[test]
    fn oversized_output_is_bounded_not_dropped() {
        let big = "x".repeat(MAX_OUTPUT_CHARS + 5_000);
        let text = bound(big);
        assert!(text.chars().count() < MAX_OUTPUT_CHARS + 200);
        assert!(text.contains("[output truncated]"));
    }

    #[test]
    fn terminal_exec_runs_through_the_sandbox_and_reports_timeouts() {
        let root = temp_root("exec");
        let out = execute(ToolInvocation {
            name: "terminal_exec",
            args: json!({"command": "echo sandboxed"}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert!(out.text.contains("sandboxed"));
        assert!(out.text.contains("[exit code: 0]"));

        #[cfg(windows)]
        let slow = "ping -n 30 127.0.0.1 > nul";
        #[cfg(not(windows))]
        let slow = "sleep 30";
        let out = execute(ToolInvocation {
            name: "terminal_exec",
            args: json!({"command": slow, "timeout_ms": 300}),
            project_root: &root,
            thread_id: "",
        });
        assert!(!out.ok);
        assert!(out.text.contains("[killed:"), "{}", out.text);
    }

    #[test]
    fn fs_read_records_a_fingerprint_for_the_thread() {
        // edit-002: after a read, the thread's fingerprint is queryable.
        let root = temp_root("fp-read");
        std::fs::write(root.join("doc.txt"), "hello fp").unwrap();

        execute(ToolInvocation {
            name: "fs_read",
            args: json!({"path": "doc.txt"}),
            project_root: &root,
            thread_id: "t-fp",
        });
        let expected =
            crate::fingerprint::file_fingerprint(&root.join("doc.txt")).unwrap();
        assert_eq!(
            crate::fingerprint::take_fingerprint("t-fp", "doc.txt"),
            Some(expected)
        );
        // Empty thread_id never records.
        std::fs::write(root.join("other.txt"), "x").unwrap();
        execute(ToolInvocation {
            name: "fs_read",
            args: json!({"path": "other.txt"}),
            project_root: &root,
            thread_id: "",
        });
        assert_eq!(crate::fingerprint::take_fingerprint("", "other.txt"), None);
    }

    #[test]
    fn stale_write_is_refused_until_the_file_is_reread() {
        // edit-003 (ZD-E-0060): file changed under us -> write refused.
        let root = temp_root("fp-stale");
        std::fs::write(root.join("code.rs"), "fn original() {}").unwrap();
        execute(ToolInvocation {
            name: "fs_read",
            args: json!({"path": "code.rs"}),
            project_root: &root,
            thread_id: "t-stale",
        });
        // The user edits the file behind the agent's back.
        std::fs::write(root.join("code.rs"), "fn user_edit() {}").unwrap();
        let out = execute(ToolInvocation {
            name: "fs_write",
            args: json!({"path": "code.rs", "content": "fn agent() {}"}),
            project_root: &root,
            thread_id: "t-stale",
        });
        assert!(!out.ok, "stale write must be refused");
        assert!(out.text.contains("changed since it was read"), "{}", out.text);
        assert_eq!(
            std::fs::read_to_string(root.join("code.rs")).unwrap(),
            "fn user_edit() {}",
            "user work must be untouched"
        );

        // Re-read arms a fresh fingerprint; the same write now succeeds.
        execute(ToolInvocation {
            name: "fs_read",
            args: json!({"path": "code.rs"}),
            project_root: &root,
            thread_id: "t-stale",
        });
        let out = execute(ToolInvocation {
            name: "fs_write",
            args: json!({"path": "code.rs", "content": "fn agent() {}"}),
            project_root: &root,
            thread_id: "t-stale",
        });
        assert!(out.ok, "{}", out.text);
        assert_eq!(std::fs::read_to_string(root.join("code.rs")).unwrap(), "fn agent() {}");

        // Consecutive writes succeed without re-reading (write re-arms).
        let out = execute(ToolInvocation {
            name: "fs_write",
            args: json!({"path": "code.rs", "content": "fn again() {}"}),
            project_root: &root,
            thread_id: "t-stale",
        });
        assert!(out.ok, "{}", out.text);

        // A thread that never read the file may still blind-write.
        let out = execute(ToolInvocation {
            name: "fs_write",
            args: json!({"path": "blind.txt", "content": "new"}),
            project_root: &root,
            thread_id: "t-other",
        });
        assert!(out.ok, "{}", out.text);
    }

    // ---- edit-008..013: edit_patch (ADR-0010 patch model) ----

    #[test]
    fn edit_patch_is_write_class_and_described_as_blocks_in_path() {
        assert_eq!(classify("edit_patch", &json!({})), Risk::Write);
        assert_eq!(
            describe("edit_patch", &json!({"path": "a.rs", "blocks": [{"old": "x"}, {"old": "y"}]})),
            "2 block(s) in a.rs"
        );
    }

    #[test]
    fn edit_patch_exact_single_block_replaces_in_place() {
        let root = temp_root("ep-exact");
        std::fs::write(root.join("code.rs"), "fn main() {\n    old_line();\n}\n").unwrap();
        let out = execute(ToolInvocation {
            name: "edit_patch",
            args: json!({"path": "code.rs", "blocks": [
                {"old": "old_line();", "new": "new_line();"}
            ]}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert_eq!(
            std::fs::read_to_string(root.join("code.rs")).unwrap(),
            "fn main() {\n    new_line();\n}\n"
        );
        assert!(!out.text.contains("whitespace-normalized"), "{}", out.text);
    }

    #[test]
    fn edit_patch_multi_block_applies_sequentially_in_memory() {
        // Block 2's anchor only exists AFTER block 1's replacement, so this
        // fails unless blocks apply in order against the evolving buffer.
        let root = temp_root("ep-multi");
        std::fs::write(root.join("seq.txt"), "one\ntwo\n").unwrap();
        let out = execute(ToolInvocation {
            name: "edit_patch",
            args: json!({"path": "seq.txt", "blocks": [
                {"old": "one", "new": "two"},
                {"old": "two\ntwo", "new": "three"}
            ]}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert_eq!(std::fs::read_to_string(root.join("seq.txt")).unwrap(), "three\n");
    }

    #[test]
    fn edit_patch_whitespace_fallback_matches_on_indent_drift_and_says_so() {
        let root = temp_root("ep-ws");
        std::fs::write(root.join("indented.rs"), "fn main() {\n    let x = 1;\n}\n").unwrap();
        // Anchor written with tab indentation and trailing spaces: no exact
        // match exists, but the normalized lines do.
        let out = execute(ToolInvocation {
            name: "edit_patch",
            args: json!({"path": "indented.rs", "blocks": [
                {"old": "\tlet x = 1;   ", "new": "    let x = 2;"}
            ]}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert!(out.text.contains("(whitespace-normalized)"), "{}", out.text);
        assert_eq!(
            std::fs::read_to_string(root.join("indented.rs")).unwrap(),
            "fn main() {\n    let x = 2;\n}\n"
        );
    }

    #[test]
    fn edit_patch_missing_anchor_aborts_whole_patch_with_no_partial_state() {
        let root = temp_root("ep-abort");
        let original = "alpha\nbeta\n";
        std::fs::write(root.join("f.txt"), original).unwrap();
        let out = execute(ToolInvocation {
            name: "edit_patch",
            args: json!({"path": "f.txt", "blocks": [
                {"old": "alpha", "new": "CHANGED"},
                {"old": "this anchor is absent", "new": "whatever"}
            ]}),
            project_root: &root,
            thread_id: "",
        });
        assert!(!out.ok);
        assert!(
            out.text.contains("Patch failed: block 2 anchor not found"),
            "{}",
            out.text
        );
        assert!(
            out.text.contains("The text to replace was not found in the file."),
            "{}",
            out.text
        );
        assert!(out.text.contains("No changes were applied"), "{}", out.text);
        // Block 1 must NOT have leaked to disk: all-or-nothing.
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), original);

        // Empty block list is a clean no-op success.
        let out = execute(ToolInvocation {
            name: "edit_patch",
            args: json!({"path": "f.txt", "blocks": []}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), original);
    }

    #[test]
    fn edit_patch_stale_write_is_refused_through_the_shared_safety_path() {
        // edit-003 rides along: read -> external modification -> patch refused.
        // The external edit keeps the anchor text present, so application
        // succeeds and the refusal comes from the fingerprint check itself.
        let root = temp_root("ep-stale");
        std::fs::write(root.join("guarded.txt"), "keep me\n").unwrap();
        execute(ToolInvocation {
            name: "fs_read",
            args: json!({"path": "guarded.txt"}),
            project_root: &root,
            thread_id: "t-ep",
        });
        std::fs::write(root.join("guarded.txt"), "keep me\n// user note\n").unwrap();
        let out = execute(ToolInvocation {
            name: "edit_patch",
            args: json!({"path": "guarded.txt", "blocks": [
                {"old": "keep me", "new": "replaced"}
            ]}),
            project_root: &root,
            thread_id: "t-ep",
        });
        assert!(!out.ok, "stale patch must be refused");
        assert!(out.text.contains("changed since it was read"), "{}", out.text);
        assert_eq!(
            std::fs::read_to_string(root.join("guarded.txt")).unwrap(),
            "keep me\n// user note\n",
            "user work must be untouched"
        );

        // Re-read arms the fingerprint and the same patch now applies,
        // preserving the user's added line.
        execute(ToolInvocation {
            name: "fs_read",
            args: json!({"path": "guarded.txt"}),
            project_root: &root,
            thread_id: "t-ep",
        });
        let out = execute(ToolInvocation {
            name: "edit_patch",
            args: json!({"path": "guarded.txt", "blocks": [
                {"old": "keep me", "new": "replaced"}
            ]}),
            project_root: &root,
            thread_id: "t-ep",
        });
        assert!(out.ok, "{}", out.text);
        assert_eq!(
            std::fs::read_to_string(root.join("guarded.txt")).unwrap(),
            "replaced\n// user note\n"
        );
    }

    // ---- edit-014: rollback staging via captured old bytes ----

    #[test]
    fn fs_write_over_existing_file_stages_old_bytes_for_rollback() {
        let root = temp_root("rb-stage");
        std::fs::write(root.join("doc.txt"), "v1").unwrap();
        let out = execute(ToolInvocation {
            name: "fs_write",
            args: json!({"path": "doc.txt", "content": "v2-longer-content"}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert_eq!(std::fs::read_to_string(root.join("doc.txt")).unwrap(), "v2-longer-content");

        // Rollback restores v1 byte-exactly and consumes the staging copy.
        rollback_last(&root.join("doc.txt")).expect("staged rollback exists");
        assert_eq!(std::fs::read_to_string(root.join("doc.txt")).unwrap(), "v1");
        // One generation only: a second rollback has nothing to restore.
        let err = rollback_last(&root.join("doc.txt")).unwrap_err();
        assert!(err.contains("no staged rollback"), "{err}");
    }

    #[test]
    fn fs_write_to_a_new_file_stages_nothing_to_roll_back() {
        let root = temp_root("rb-fresh");
        let out = execute(ToolInvocation {
            name: "fs_write",
            args: json!({"path": "fresh.txt", "content": "created"}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        // No prior bytes existed, so there is nothing to roll back to.
        let err = rollback_last(&root.join("fresh.txt")).unwrap_err();
        assert!(err.contains("no staged rollback"), "{err}");
    }

    // ---- edit-022..024: read-only git tools (real temp repos) ----

    fn git_available() -> bool {
        std::process::Command::new("git").arg("--version").output().is_ok()
    }

    /// Direct-argv git runner for test setup (same discipline as the facade).
    fn git(repo: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("LC_ALL", "C")
            .output()
            .expect("git binary should exist");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Throwaway repo with one commit; None makes callers skip gracefully
    /// when no git binary is installed.
    fn temp_repo(tag: &str) -> Option<PathBuf> {
        if !git_available() {
            return None;
        }
        let dir = temp_root(tag);
        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "agent@example.com"]);
        git(&dir, &["config", "user.name", "Agent"]);
        std::fs::write(dir.join("file.txt"), "one\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "initial commit"]);
        Some(dir)
    }

    #[test]
    fn git_read_tools_are_classified_read_only_and_described() {
        for name in ["git_status", "git_diff", "git_log"] {
            assert_eq!(classify(name, &json!({})), Risk::ReadOnly, "{name}");
        }
        assert_eq!(describe("git_status", &json!({"path": "."})), ".");
        assert_eq!(
            describe("git_log", &json!({"limit": 7})),
            "last 7 commits"
        );
    }

    #[test]
    fn git_status_reports_branch_then_clean_tree_after_commit() {
        let Some(root) = temp_repo("gstatus") else { return };
        let out = execute(ToolInvocation {
            name: "git_status",
            args: json!({}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert!(out.text.starts_with("branch "), "{}", out.text);
        assert!(out.text.contains("clean"), "{}", out.text);

        // Untracked and modified files both surface as entries.
        std::fs::write(root.join("new.txt"), "x").unwrap();
        std::fs::write(root.join("file.txt"), "two\n").unwrap();
        let out = execute(ToolInvocation {
            name: "git_status",
            args: json!({"path": "."}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert!(out.text.contains("?? new.txt"), "{}", out.text);
        assert!(out.text.contains("file.txt"), "{}", out.text);
    }

    #[test]
    fn git_status_fails_cleanly_outside_a_repository() {
        let root = temp_root("nonrepo"); // never git-inited
        let out = execute(ToolInvocation {
            name: "git_status",
            args: json!({}),
            project_root: &root,
            thread_id: "",
        });
        assert!(!out.ok);
        assert!(out.text.starts_with("git_status failed:"), "{}", out.text);
    }

    #[test]
    fn git_diff_is_empty_after_commit_and_counts_unstaged_edits() {
        let Some(root) = temp_repo("gdiff") else { return };
        let out = execute(ToolInvocation {
            name: "git_diff",
            args: json!({}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert!(out.text.contains("0 files changed"), "{}", out.text);

        // A +2/-0 unstaged edit shows as "2 0 <path>".
        std::fs::write(root.join("file.txt"), "one\ntwo\nthree\n").unwrap();
        let out = execute(ToolInvocation {
            name: "git_diff",
            args: json!({"path": "."}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert!(out.text.contains("2 0 file.txt"), "{}", out.text);
        assert!(out.text.ends_with("files changed"), "{}", out.text);
    }

    #[test]
    fn git_log_lists_commits_and_clamps_limit() {
        let Some(root) = temp_repo("glog") else { return };
        let out = execute(ToolInvocation {
            name: "git_log",
            args: json!({"limit": 5}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert_eq!(out.text.lines().count(), 1, "{}", out.text);
        assert!(out.text.contains("initial commit"), "{}", out.text);

        // An absurd limit clamps into the 1..=100 window instead of erroring.
        let out = execute(ToolInvocation {
            name: "git_log",
            args: json!({"limit": 99_999}),
            project_root: &root,
            thread_id: "",
        });
        assert!(out.ok, "{}", out.text);
        assert!(out.text.lines().count() <= 100);
    }

    // ---- tok-003/004/005/020: fs_read result cache & redundant-read marker ----

    /// The tool cache is process-global; serialize the cache-behaviour tests
    /// so cargo's parallel test threads cannot evict each other's entries.
    fn cache_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    fn read_inv<'a>(root: &'a PathBuf, path: &str, thread: &'a str) -> ToolInvocation<'a> {
        ToolInvocation {
            name: "fs_read",
            args: json!({ "path": path }),
            project_root: root,
            thread_id: thread,
        }
    }

    #[test]
    fn second_identical_fs_read_is_served_from_cache() {
        let _g = cache_test_lock().lock().unwrap();
        let root = temp_root("tok-hit");
        std::fs::write(root.join("c.txt"), "cache me").unwrap();

        let first = execute(read_inv(&root, "c.txt", ""));
        assert_eq!(first.text, "cache me");
        // tok-004 hit path: same path + unchanged fingerprint => served with
        // the visible [cached] prefix instead of a fresh disk read.
        let second = execute(read_inv(&root, "c.txt", ""));
        assert!(second.ok);
        assert_eq!(second.text, "[cached] cache me", "{}", second.text);
    }

    // tok-012: counters are process-global and only grow; assert monotonic
    // deltas around a known miss→hit pair (exact equality would flake against
    // sibling tests' concurrent fs_reads in the same process).
    #[test]
    fn cache_metrics_reflect_a_known_miss_then_hit() {
        let _g = cache_test_lock().lock().unwrap();
        let root = temp_root("tok-metrics");
        std::fs::write(root.join("k.txt"), "metrics").unwrap();

        let before = cache_metrics();
        let first = execute(read_inv(&root, "k.txt", ""));
        let second = execute(read_inv(&root, "k.txt", ""));
        let after = cache_metrics();

        assert_eq!(first.text, "metrics", "unique path must be a miss");
        assert_eq!(second.text, "[cached] metrics", "second read must hit");
        assert!(after.misses >= before.misses + 1, "{before:?} -> {after:?}");
        assert!(after.hits >= before.hits + 1, "{before:?} -> {after:?}");
    }

    #[test]
    fn changed_content_misses_the_cache_and_refreshes_it() {
        let _g = cache_test_lock().lock().unwrap();
        let root = temp_root("tok-miss");
        std::fs::write(root.join("m.txt"), "v1").unwrap();

        assert_eq!(execute(read_inv(&root, "m.txt", "t-tok-miss")).text, "v1");
        std::fs::write(root.join("m.txt"), "v2").unwrap();

        // tok-005: changed bytes land on a different key — fresh content is
        // served (no stale hit) and the duplicate marker must NOT appear
        // because the thread has never seen these bytes.
        let second = execute(read_inv(&root, "m.txt", "t-tok-miss"));
        assert_eq!(second.text, "v2", "{}", second.text);
        assert!(!second.text.contains("[cached]"));
        assert!(!second.text.contains("duplicate read"));
        // The cache now serves the new version.
        assert_eq!(execute(read_inv(&root, "m.txt", "")).text, "[cached] v2");
    }

    #[test]
    fn duplicate_unchanged_reread_is_marked_and_coexists_with_cached() {
        let _g = cache_test_lock().lock().unwrap();
        let root = temp_root("tok-dup");
        std::fs::write(root.join("d.txt"), "body").unwrap();

        let first = execute(read_inv(&root, "d.txt", "t-tok-dup"));
        assert_eq!(first.text, "body", "first sight: no marker");
        // tok-020: same thread re-reads unchanged bytes => observable marker,
        // and it coexists with the tok-004 [cached] prefix.
        let second = execute(read_inv(&root, "d.txt", "t-tok-dup"));
        assert!(second.text.starts_with("[cached] body"), "{}", second.text);
        assert!(second.text.contains("(duplicate read of unchanged file)"));
        // A different thread reading the same file gets the hit, no marker.
        let other = execute(read_inv(&root, "d.txt", "t-tok-other"));
        assert_eq!(other.text, "[cached] body");
    }

    #[test]
    fn cache_cap_churn_evicts_old_entries_without_panicking() {
        let _g = cache_test_lock().lock().unwrap();
        let root = temp_root("tok-cap");
        let n = TOOL_CACHE_CAP + 10;
        for i in 0..n {
            std::fs::write(root.join(format!("f{i}.txt")), format!("content-{i}")).unwrap();
        }
        for i in 0..n {
            let out = execute(read_inv(&root, &format!("f{i}.txt"), ""));
            assert_eq!(out.text, format!("content-{i}"), "iteration {i}: first pass must miss");
        }
        // The early entry did not survive the cap churn: it is re-read fresh.
        let again = execute(read_inv(&root, "f0.txt", ""));
        assert_eq!(again.text, "content-0", "{}", again.text);
    }

    // ---- tok-021: lazy tool manifest filtering ----

    #[test]
    fn hint_match_lists_tool_and_others_are_filtered() {
        let hint = "run fs_read on config.toml";
        assert!(should_list("fs_read", hint));
        assert!(!should_list("terminal_exec", hint));
        assert!(!should_list("fs_write", hint));
    }

    #[test]
    fn empty_hint_lists_every_tool() {
        for name in ["fs_read", "terminal_exec", "git_log"] {
            assert!(should_list(name, ""), "{name}");
        }
    }

    #[test]
    fn filter_manifest_preserves_input_order() {
        let tools = vec!["terminal_exec", "fs_read", "fs_write"];
        assert_eq!(filter_manifest(&tools, "use fs_write"), vec!["fs_write"]);
        // Multiple hits keep their relative order; empty hint keeps all.
        assert_eq!(
            filter_manifest(&tools, "fs_read and fs_write"),
            vec!["fs_read", "fs_write"]
        );
        assert_eq!(filter_manifest(&tools, ""), vec!["terminal_exec", "fs_read", "fs_write"]);
    }

    #[test]
    fn manifest_match_is_case_insensitive() {
        let tools = vec!["fs_read"];
        assert_eq!(filter_manifest(&tools, "please FS_READ it"), vec!["fs_read"]);
        assert_eq!(filter_manifest(&tools, "FS_SEARCH it"), Vec::<&str>::new());
    }
}
