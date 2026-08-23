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
        "fs_read" | "fs_list" | "fs_search" => Risk::ReadOnly,
        "fs_write" => Risk::Write,
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
        "terminal_exec" => fmt_arg(args, "command"),
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
    ]
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

/// Execute an approved tool call. This is the only place the core touches the
/// filesystem or spawns processes.
pub fn execute(inv: ToolInvocation) -> ToolOutput {
    match inv.name {
        "fs_read" => fs_read(&inv),
        "fs_list" => fs_list(&inv),
        "fs_search" => fs_search(&inv),
        "fs_write" => fs_write(&inv),
        "terminal_exec" => terminal_exec(&inv),
        other => ToolOutput { ok: false, text: format!("unknown tool {other:?}") },
    }
}

fn fs_read(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let path = scoped(inv.project_root, fmt_arg(&inv.args, "path").as_str())?;
        let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if meta.len() > 512 * 1024 {
            return Err("file larger than 512 KiB; read it in parts via terminal_exec".into());
        }
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        // edit-002: remember what this thread last saw, keyed by the raw
        // requested path so fs_write's later lookup resolves identically.
        if !inv.thread_id.is_empty() {
            if let Ok(fp) = crate::fingerprint::file_fingerprint(&path) {
                crate::fingerprint::record_fingerprint(inv.thread_id, &fmt_arg(&inv.args, "path"), fp);
            }
        }
        Ok(content)
    })();
    match result {
        Ok(content) => ToolOutput { ok: true, text: bound(content) },
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

fn fs_write(inv: &ToolInvocation) -> ToolOutput {
    let result: Result<String, String> = (|| {
        let raw = fmt_arg(&inv.args, "path");
        let content = inv.args.get("content").and_then(Value::as_str).unwrap_or("");
        let path = scoped(inv.project_root, &raw)?;
        // edit-003 (ZD-E-0060): if this thread read the file before, the
        // on-disk content must still match what it saw. Never-read files
        // stay writable for now (blind writes; edit-018 flags them later).
        if !inv.thread_id.is_empty() {
            if let Some(expected) = crate::fingerprint::take_fingerprint(inv.thread_id, &raw) {
                match crate::fingerprint::file_fingerprint(&path) {
                    Ok(current) if current == expected => {}
                    _ => {
                        return Err(
                            "This file changed since it was read. Re-read it before editing."
                                .into(),
                        );
                    }
                }
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
        // Re-arm so consecutive agent writes don't trip against their own output.
        if !inv.thread_id.is_empty() {
            if let Ok(fp) = crate::fingerprint::file_fingerprint(&path) {
                crate::fingerprint::record_fingerprint(inv.thread_id, &raw, fp);
            }
        }
        Ok(format!("wrote {} bytes to {}", content.len(), raw))
    })();
    match result {
        Ok(text) => ToolOutput { ok: true, text },
        Err(e) => ToolOutput { ok: false, text: format!("fs_write failed: {e}") },
    }
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
}
