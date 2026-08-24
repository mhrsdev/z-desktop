//! Tree-sitter based Rust symbol extraction (idx-004, per ADR-0007).
//!
//! The index keeps its regex path as fallback (ADR-0007 mandate); this
//! module is the primary extractor for .rs files.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
}

/// Extract top-level-ish Rust symbols via tree-sitter. Panics from the C
/// parser on malformed input are caught per ADR-0007 and surfaced as Err.
pub fn extract_rust_symbols(source: &str) -> Result<Vec<Symbol>, String> {
    let result = std::panic::catch_unwind(|| -> Vec<Symbol> {
        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .is_err()
        {
            return Vec::new();
        }
        let Some(tree) = parser.parse(source, None) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        walk(tree.root_node(), source, &mut out);
        out
    });
    match result {
        Ok(symbols) => Ok(symbols),
        Err(_) => Err("parser panicked on malformed input".into()),
    }
}

/// Stable symbol id "{file_hash}:{name}:{kind}:{index}" (idx-010).
pub fn symbol_id(file_hash: &str, name: &str, kind: &SymbolKind, index: usize) -> String {
    format!("{file_hash}:{name}:{}:{index}", kind_snake(kind))
}

fn kind_snake(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Impl => "impl",
        SymbolKind::Module => "module",
    }
}

/// Pair each extracted symbol with its stable id, using position index
/// (idx-010). Parser failure degrades to an empty list, matching
/// extract_rust_symbols' internal fallback for unusable input.
pub fn extract_with_ids(file_hash: &str, source: &str) -> Vec<(Symbol, String)> {
    extract_rust_symbols(source)
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let id = symbol_id(file_hash, &s.name, &s.kind, i);
            (s, id)
        })
        .collect()
}

/// Minimal cross-file symbol index (idx-005): flat (file_hash, Symbol, id)
/// rows. ponytail: O(n) scan per lookup; index by name only if lookups get hot.
#[derive(Debug, Default)]
pub struct SymbolTable {
    entries: Vec<(String, Symbol, String)>,
}

impl SymbolTable {
    /// Extract symbols from one file and append them to the table.
    pub fn add_file(&mut self, file_hash: &str, source: &str) {
        self.entries.extend(
            extract_with_ids(file_hash, source)
                .into_iter()
                .map(|(s, id)| (file_hash.to_string(), s, id)),
        );
    }

    /// Ids of every symbol named `name`, across all indexed files.
    pub fn lookup_name(&self, name: &str) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(_, s, _)| s.name == name)
            .map(|(_, _, id)| id.as_str())
            .collect()
    }

    /// Total number of indexed symbols across all files.
    pub fn total(&self) -> usize {
        self.entries.len()
    }
}

fn walk(node: tree_sitter::Node, src: &str, out: &mut Vec<Symbol>) {
    use tree_sitter::Node;
    fn kind_of(node: Node) -> Option<SymbolKind> {
        match node.kind() {
            "function_item" => Some(SymbolKind::Function),
            "struct_item" => Some(SymbolKind::Struct),
            "enum_item" => Some(SymbolKind::Enum),
            "trait_item" => Some(SymbolKind::Trait),
            "impl_item" | "impl" => Some(SymbolKind::Impl),
            "mod_item" => Some(SymbolKind::Module),
            _ => None,
        }
    }
    if let Some(kind) = kind_of(node) {
        // Most items expose a `name` field; `impl_item` names by its trait
        // or type operand instead (tree-sitter-rust has no `name` there).
        let name_node = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("trait"))
            .or_else(|| node.child_by_field_name("type"));
        if let Some(name_node) = name_node {
            let name = &src[name_node.start_byte()..name_node.end_byte()];
            if !out.iter().any(|s| s.name == name && s.kind == kind) && out.len() < 200 {
                out.push(Symbol { name: name.to_string(), kind });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, src, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
mod utils;
pub struct Config { pub name: String }
pub enum Mode { Fast, Slow }
pub trait Runner { fn run(&self); }
impl Runner for Config { fn run(&self) {} }
pub fn main() {}
fn helper() {}
"#;

    #[test]
    fn extracts_all_six_symbol_kinds_with_names() {
        let syms = extract_rust_symbols(SAMPLE).expect("parses");
        assert!(syms.contains(&Symbol { name: "Config".into(), kind: SymbolKind::Struct }));
        assert!(syms.contains(&Symbol { name: "Mode".into(), kind: SymbolKind::Enum }));
        assert!(syms.contains(&Symbol { name: "Runner".into(), kind: SymbolKind::Trait }));
        assert!(syms.contains(&Symbol { name: "main".into(), kind: SymbolKind::Function }));
        assert!(syms.contains(&Symbol { name: "helper".into(), kind: SymbolKind::Function }));
        // impl Runner for Config: named by its trait operand.
        assert!(
            syms.iter().any(|s| s.kind == SymbolKind::Impl && s.name.contains("Runner")),
            "impl symbol expected, got: {syms:?}"
        );
        // mod utils; is a declaration — still a mod_item node.
        assert!(syms.contains(&Symbol { name: "utils".into(), kind: SymbolKind::Module }));
    }

    #[test]
    fn malformed_source_never_panics() {
        let garbage = "fn {{{ broken !!! rust code (((";
        // Either Ok with partial results or Err — but never a panic escape.
        let _ = extract_rust_symbols(garbage);
    }

    #[test]
    fn debug_node_kinds() {
        let src = "impl Runner for Config { fn run(&self) {} }";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        fn walk(n: tree_sitter::Node) {
            println!("KIND: {}", n.kind());
            let mut c = n.walk();
            for ch in n.children(&mut c) { walk(ch); }
        }
        walk(tree.root_node());
    }

    #[test]
    fn empty_source_yields_no_symbols() {
        assert!(extract_rust_symbols("").unwrap().is_empty());
    }

    #[test]
    fn symbol_id_format_is_file_hash_name_kind_index() {
        assert_eq!(symbol_id("abc123", "main", &SymbolKind::Function, 3), "abc123:main:function:3");
        assert_eq!(symbol_id("h", "Config", &SymbolKind::Struct, 0), "h:Config:struct:0");
        // Every kind maps to its lowercase snake name.
        for (kind, want) in [
            (SymbolKind::Function, "function"),
            (SymbolKind::Struct, "struct"),
            (SymbolKind::Enum, "enum"),
            (SymbolKind::Trait, "trait"),
            (SymbolKind::Impl, "impl"),
            (SymbolKind::Module, "module"),
        ] {
            assert_eq!(symbol_id("f", "x", &kind, 1), format!("f:x:{want}:1"));
        }
    }

    #[test]
    fn distinct_indices_produce_distinct_ids() {
        let a = symbol_id("hash", "dup", &SymbolKind::Function, 0);
        let b = symbol_id("hash", "dup", &SymbolKind::Function, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn symbol_table_finds_same_name_across_two_files() {
        let mut table = SymbolTable::default();
        table.add_file("hash_a", "fn shared() {}");
        table.add_file("hash_b", "fn shared() {}");
        let hits = table.lookup_name("shared");
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&"hash_a:shared:function:0"));
        assert!(hits.contains(&"hash_b:shared:function:0"));
    }

    #[test]
    fn symbol_table_distinct_names_stay_isolated() {
        let mut table = SymbolTable::default();
        table.add_file("h1", "fn alpha() {}\nstruct Beta;");
        table.add_file("h2", "fn gamma() {}");
        assert!(table.lookup_name("alpha").len() == 1);
        assert!(table.lookup_name("beta").is_empty(), "lookup is case-sensitive exact match");
        assert!(table.lookup_name("missing").is_empty());
        assert_eq!(table.lookup_name("gamma"), vec!["h2:gamma:function:0"]);
    }

    #[test]
    fn symbol_table_total_counts_all_symbols() {
        let mut table = SymbolTable::default();
        assert_eq!(table.total(), 0);
        table.add_file("h1", "fn a() {}\nstruct B;\nenum C {}");
        assert_eq!(table.total(), 3);
        table.add_file("h2", "trait D {}");
        assert_eq!(table.total(), 4);
        // Malformed source degrades to zero added symbols, no panic.
        table.add_file("h3", "fn {{{ broken !!!");
        assert_eq!(table.total(), 4);
    }

    #[test]
    fn extract_with_ids_pairs_every_symbol_in_order() {
        let paired = extract_with_ids("deadbeef", SAMPLE);
        let syms = extract_rust_symbols(SAMPLE).unwrap();
        assert_eq!(paired.len(), syms.len());
        for (i, ((s, id), orig)) in paired.iter().zip(&syms).enumerate() {
            assert_eq!(s, orig, "order preserved at {i}");
            assert_eq!(*id, symbol_id("deadbeef", &s.name, &s.kind, i));
            assert!(id.starts_with("deadbeef:"));
        }
        // Spot-check the first pair's exact format.
        let (first, first_id) = &paired[0];
        assert_eq!(
            first_id.as_str(),
            format!("deadbeef:{}:{}:0", first.name, kind_snake(&first.kind))
        );
    }
}
