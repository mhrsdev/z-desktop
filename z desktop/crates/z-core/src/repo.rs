//! Repository intelligence — an incremental index of the project.
//!
//! The index gives the agent a stable, cheap map of the repository so it does
//! not re-read the tree on every task. Files are fingerprinted by
//! (mtime, size); unchanged files keep their cached symbols. The map text is
//! part of the stable prompt prefix, so it changes only when the index does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub rel_path: String,
    pub size: u64,
    /// (mtime secs, mtime nanos) — the cheap change signal.
    pub stamp: (i64, u32),
    pub symbols: Vec<String>,
}

#[derive(Default)]
pub struct RepoIndex {
    pub root: Option<PathBuf>,
    pub files: HashMap<String, FileEntry>,
}

/// Directories that never carry source worth indexing.
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "build", "__pycache__", ".next", "venv",
    ".venv", ".idea", ".vscode",
];

impl RepoIndex {
    pub fn open(root: &Path) -> Self {
        let mut index = Self { root: Some(root.to_path_buf()), files: HashMap::new() };
        index.rescan();
        index
    }

    /// Incremental rescan: new/changed files are parsed, unchanged ones are
    /// kept from cache. Returns (files indexed, symbols found).
    pub fn rescan(&mut self) -> (u64, u64) {
        let Some(root) = self.root.clone() else { return (0, 0) };
        let mut seen: Vec<String> = Vec::new();
        let mut parsed = 0u64;
        super::tools::walk_files(&root, &mut |path| {
            let Ok(meta) = std::fs::metadata(path) else { return };
            let rel = match path.strip_prefix(&root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => return,
            };
            let stamp = file_stamp(&meta);
            seen.push(rel.clone());
            let entry = self.files.entry(rel.clone()).or_insert_with(|| FileEntry {
                rel_path: String::new(),
                size: 0,
                stamp,
                symbols: Vec::new(),
            });
            if entry.stamp != stamp || entry.rel_path.is_empty() {
                entry.rel_path = rel.clone();
                entry.size = meta.len();
                entry.stamp = stamp;
                entry.symbols = extract_symbols(path);
                parsed += 1;
            }
        });
        // Drop entries for deleted files so the map never ghosts them.
        self.files.retain(|k, _| seen.iter().any(|s| s == k));
        let symbols = self.files.values().map(|f| f.symbols.len() as u64).sum();
        (parsed, symbols)
    }

    pub fn file_count(&self) -> u64 {
        self.files.len() as u64
    }

    pub fn symbol_count(&self) -> u64 {
        self.files.values().map(|f| f.symbols.len() as u64).sum()
    }

    /// Compact textual map for the model's system context. Bounded so a huge
    /// repository cannot blow the prefix budget; directories are summarised by
    /// their largest symbol-bearing files first.
    pub fn map_text(&self, max_lines: usize) -> String {
        let mut lines: Vec<String> = Vec::new();
        let mut dirs: std::collections::BTreeMap<&str, Vec<&FileEntry>> =
            std::collections::BTreeMap::new();
        for f in self.files.values() {
            let dir = match f.rel_path.rfind('/') {
                Some(i) => &f.rel_path[..i],
                None => ".",
            };
            dirs.entry(dir).or_default().push(f);
        }
        for (dir, entries) in dirs {
            lines.push(format!("{dir}/"));
            let mut sorted = entries;
            sorted.sort_by(|a, b| b.symbols.len().cmp(&a.symbols.len()).then(a.rel_path.cmp(&b.rel_path)));
            for e in sorted.iter().take(12) {
                let name = e.rel_path.rsplit('/').next().unwrap_or(&e.rel_path);
                if e.symbols.is_empty() {
                    lines.push(format!("  {name}"));
                } else {
                    let syms: Vec<&str> = e.symbols.iter().take(6).map(String::as_str).collect();
                    lines.push(format!("  {name}: {}", syms.join(", ")));
                }
            }
            if sorted.len() > 12 {
                lines.push(format!("  … {} more files", sorted.len() - 12));
            }
            if lines.len() >= max_lines {
                break;
            }
        }
        lines.truncate(max_lines);
        lines.join("
")
    }
}

fn file_stamp(meta: &std::fs::Metadata) -> (i64, u32) {
    use std::time::UNIX_EPOCH;
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| (d.as_secs() as i64, d.subsec_nanos()))
        .unwrap_or((0, 0))
}

/// Lightweight symbol extraction — regex-free heuristics per language family.
/// This is navigation data, not ground truth: tools always read real files
/// before editing them.
fn extract_symbols(path: &Path) -> Vec<String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let prefixes: &[&str] = match ext {
        "rs" => &["fn ", "struct ", "enum ", "trait ", "impl "],
        "ts" | "tsx" | "js" | "jsx" => &["function ", "class ", "const ", "interface "],
        "py" => &["def ", "class "],
        "go" => &["func ", "type "],
        _ => return Vec::new(),
    };
    let Ok(content) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut out = Vec::new();
    for line in content.lines().take(4000) {
        let trimmed = line.trim_start();
        for p in prefixes {
            if let Some(rest) = trimmed.strip_prefix(p) {
                let name: String =
                    rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() && !out.contains(&name) && out.len() < 40 {
                    out.push(name);
                }
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zrepo-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/main.rs"),
            "fn main() {}
struct Config { name: String }
",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        std::fs::write(dir.join("node_modules/pkg/x.js"), "function ignored() {}").unwrap();
        dir
    }

    #[test]
    fn index_finds_symbols_and_skips_dependency_dirs() {
        let root = temp_root("idx");
        let mut index = RepoIndex::open(&root);
        assert_eq!(index.file_count(), 1, "node_modules must not be indexed");
        let main = index.files.get("src/main.rs").unwrap();
        assert!(main.symbols.contains(&"main".to_string()));
        assert!(main.symbols.contains(&"Config".to_string()));
    }

    #[test]
    fn rescan_is_incremental_and_detects_changes() {
        let root = temp_root("inc");
        let mut index = RepoIndex::open(&root);
        let (parsed_first, _) = index.rescan();
        assert_eq!(parsed_first, 0, "nothing changed since open");

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("src/main.rs"), "fn other() {}
").unwrap();
        let (parsed_second, _) = index.rescan();
        assert_eq!(parsed_second, 1, "the changed file must be reparsed");
        assert!(index.files.get("src/main.rs").unwrap().symbols.contains(&"other".to_string()));
    }

    #[test]
    fn map_text_is_bounded_and_grouped_by_directory() {
        let root = temp_root("map");
        let index = RepoIndex::open(&root);
        let map = index.map_text(50);
        assert!(map.contains("src/"), "{map}");
        assert!(!map.contains("node_modules"), "{map}");
    }
}