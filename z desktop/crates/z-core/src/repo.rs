//! Repository intelligence — an incremental index of the project.
//!
//! The index gives the agent a stable, cheap map of the repository so it does
//! not re-read the tree on every task. Files are fingerprinted by
//! (mtime, size); unchanged files keep their cached symbols. The map text is
//! part of the stable prompt prefix, so it changes only when the index does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::search_index::TrigramIndex;

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
    /// Lazy whole-repo lexical index, rebuilt only by `build_search_index`.
    search_index: TrigramIndex,
    /// Doc id -> rel path, aligned with insertion order into `search_index`.
    search_paths: Vec<String>,
    /// Cached repo map keyed by (input fingerprint, budget). idx-021.
    map_cache: Option<(u64, usize, String)>,
}

/// Directories that never carry source worth indexing.
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "build", "__pycache__", ".next", "venv",
    ".venv", ".idea", ".vscode",
];

impl RepoIndex {
    pub fn open(root: &Path) -> Self {
        let mut index =
            Self { root: Some(root.to_path_buf()), files: HashMap::new(), ..Default::default() };
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
                // idx-004 (ADR-0007): tree-sitter is primary for .rs; the
                // regex scan stays as fallback for other languages and for
                // any file the parser rejects.
                entry.symbols = if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    std::fs::read_to_string(path)
                        .map(|src| {
                            match super::symbols::extract_rust_symbols(&src) {
                                Ok(syms) => syms.into_iter().map(|s| s.name).collect(),
                                Err(_) => extract_symbols(path),
                            }
                        })
                        .unwrap_or_default()
                } else {
                    extract_symbols(path)
                };
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

    /// Rebuild the lexical index from disk over every indexed file. Whole
    /// rebuild, O(all bytes): no incremental updates yet.
    // ponytail: full rebuild on demand; switch to incremental trigram updates
    // when rebuild latency shows up on large repos (idx-021).
    pub fn build_search_index(&mut self) -> Result<(), String> {
        let Some(root) = self.root.clone() else {
            return Err("no repository root configured".into());
        };
        let mut paths: Vec<String> = self.files.keys().cloned().collect();
        paths.sort(); // deterministic doc ids regardless of HashMap order
        let mut index = TrigramIndex::default();
        let mut ordered: Vec<String> = Vec::with_capacity(paths.len());
        for rel in &paths {
            // Non-UTF8/unreadable files aren't searchable text; skip rather
            // than fail the whole build.
            if let Ok(content) = std::fs::read_to_string(root.join(rel)) {
                index.add(&content);
                ordered.push(rel.clone());
            }
        }
        self.search_index = index;
        self.search_paths = ordered;
        Ok(())
    }

    /// Lexical search over the last-built index; returns `(rel_path, score)`
    /// best-first. Empty until `build_search_index` has run.
    pub fn search(&self, query: &str) -> Vec<(String, f32)> {
        self.search_index
            .ranked_search(query)
            .into_iter()
            .filter_map(|h| self.search_paths.get(h.doc_id as usize).map(|p| (p.clone(), h.score)))
            .collect()
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
        lines.join("\n")
    }

    /// idx-021: flat ranked repo map — one `rel_path  Nsymb` line per indexed
    /// file, best-first by symbol count, truncated to `max_chars` with an
    /// explicit `... +N more` marker when files were dropped.
    pub fn build_map(&self, max_chars: usize) -> String {
        let mut files: Vec<&FileEntry> = self.files.values().collect();
        files.sort_by(|a, b| {
            b.symbols.len().cmp(&a.symbols.len()).then(a.rel_path.cmp(&b.rel_path))
        });
        let lines: Vec<String> =
            files.iter().map(|f| format!("{}  {}", f.rel_path, f.symbols.len())).collect();
        // Cost of emitting the first n lines, counting each trailing newline.
        let cost = |n: usize| -> usize { lines[..n].iter().map(|l| l.len() + 1).sum() };
        let full = lines.len();
        if cost(full) <= max_chars {
            return lines.join("\n");
        }
        let mut keep = full;
        while keep > 0 {
            let marker = format!("... +{} more", full - keep);
            if cost(keep) + marker.len() <= max_chars {
                let mut out = lines[..keep].join("\n");
                out.push('\n');
                out.push_str(&marker);
                return out;
            }
            keep -= 1;
        }
        String::new()
    }

    /// Fingerprint of everything `build_map` reads: the set of indexed paths
    /// and each file's stamp. FNV-1a over sorted entries.
    fn map_fingerprint(&self) -> u64 {
        let mut parts: Vec<(&str, (i64, u32))> =
            self.files.values().map(|f| (f.rel_path.as_str(), f.stamp)).collect();
        parts.sort_unstable();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for (path, stamp) in &parts {
            for b in path
                .bytes()
                .chain(std::iter::once(0))
                .chain(stamp.0.to_le_bytes())
                .chain(stamp.1.to_le_bytes())
            {
                h ^= b as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    /// Cached `build_map`; recomputed only when the index fingerprint or the
    /// budget changed since the last call.
    pub fn repo_map(&mut self, max_chars: usize) -> String {
        let fp = self.map_fingerprint();
        if let Some((cached_fp, cached_max, map)) = &self.map_cache {
            if *cached_fp == fp && *cached_max == max_chars {
                return map.clone();
            }
        }
        let map = self.build_map(max_chars);
        self.map_cache = Some((fp, max_chars, map.clone()));
        map
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

    #[test]
    fn search_returns_rel_paths_ranked_best_first() {
        let root = temp_root("srch");
        std::fs::write(root.join("src/config.rs"), "pub fn load_configuration() {}\n").unwrap();
        std::fs::write(root.join("src/readme.md"), "notes about configuration files\n").unwrap();
        // Non-UTF8 junk must be skipped, not fail the build.
        std::fs::write(root.join("blob.bin"), [0xffu8, 0xfe, 0x00]).unwrap();
        let mut index = RepoIndex::open(&root);
        index.build_search_index().expect("build succeeds despite blob.bin");
        let hits = index.search("configuration");
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|(p, _)| p != "blob.bin"));
        assert!(hits.iter().all(|(p, _)| index.files.contains_key(p)), "{hits:?}");
        // Full-query docs tie at 1.0; the tie breaks by ascending doc id,
        // which follows sorted rel paths ("config.rs" < "readme.md").
        assert_eq!(hits[0].0, "src/config.rs", "{hits:?}");
        assert_eq!(hits[0].1, 1.0);
        assert!(hits.windows(2).all(|w| w[0].1 >= w[1].1));
    }

    #[test]
    fn search_before_build_or_on_empty_repo_is_empty() {
        let mut index = RepoIndex::default();
        assert!(index.search("anything").is_empty());
        assert!(index.build_search_index().is_err(), "no root configured");
        let root = temp_root("empty");
        std::fs::remove_file(root.join("src/main.rs")).unwrap();
        let mut index = RepoIndex::open(&root);
        index.build_search_index().unwrap();
        assert!(index.search("main").is_empty());
    }

    #[test]
    fn build_map_format_is_path_then_symbol_count() {
        let root = temp_root("bmap");
        std::fs::write(root.join("src/lib.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
        let mut index = RepoIndex::open(&root);
        // lib.rs (2 syms) ranks above main.rs (2 syms? main+Config) — just
        // assert both lines exist in `rel  N` shape.
        let map = index.repo_map(10_000);
        for line in map.lines() {
            let (path, count) = line.rsplit_once("  ").expect("two-space separator");
            assert!(index.files.contains_key(path), "{path}");
            assert_eq!(count.parse::<usize>().unwrap(), index.files[path].symbols.len());
        }
        assert!(map.contains("src/main.rs"), "{map}");
        assert!(map.contains("src/lib.rs"), "{map}");
    }

    #[test]
    fn build_map_respects_char_budget_and_marks_truncation() {
        let root = temp_root("budget");
        for i in 0..40 {
            std::fs::write(
                root.join(format!("src/mod{i}.rs")),
                format!("fn sym_{i}() {{}}\n"),
            )
            .unwrap();
        }
        let index = RepoIndex::open(&root);
        let map = index.build_map(200);
        assert!(map.len() <= 200, "len={}, budget=200", map.len());
        assert!(map.contains("... +"), "truncation marker missing: {map}");

        // Generous budget: everything fits, no marker.
        let full = index.build_map(100_000);
        assert!(!full.contains("+"), "no marker when everything fits: {full}");
        assert_eq!(full.lines().count(), index.file_count() as usize);
    }

    #[test]
    fn repo_map_caches_until_inputs_change() {
        let root = temp_root("cache");
        let mut index = RepoIndex::open(&root);
        let first = index.repo_map(10_000);
        assert_eq!(index.repo_map(10_000), first, "same inputs => cached copy");

        // Mutate a file stamp: fingerprint must change and the cache must be
        // invalidated so the new symbols show up.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("src/extra.rs"), "fn brand_new_symbol() {}\n").unwrap();
        index.rescan();
        let second = index.repo_map(10_000);
        assert_ne!(first, second);
        assert!(second.contains("src/extra.rs"), "{second}");
    }
}