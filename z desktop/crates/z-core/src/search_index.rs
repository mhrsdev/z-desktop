//! Trigram inverted index for lexical candidate generation (idx-018).
//!
//! Minimal structure per Z-DESKTOP-TASKS idx-018: documents are stored
//! verbatim, their lowercased 3-grams are indexed into postings maps, and
//! queries are answered by postings-union followed by substring verify.
//! Incremental updates (idx-019) and query-path tuning (idx-020) build on
//! top of this.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Default, Clone)]
pub struct TrigramIndex {
    /// Lowercased trigram -> doc ids containing it. Texts shorter than 3
    /// chars have no trigrams and are indexed under the special key "" so
    /// they remain reachable via postings.
    map: HashMap<String, Vec<u32>>,
    /// Stored document texts by doc id.
    docs: Vec<String>,
}

/// Lowercased character trigrams of `text`; empty vec when len < 3.
fn trigrams(text: &str) -> Vec<String> {
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    if lower.len() < 3 {
        return Vec::new();
    }
    lower.windows(3).map(|w| w.iter().collect()).collect()
}

impl TrigramIndex {
    /// Store a document and index its lowercase trigrams. Returns its doc id.
    pub fn add(&mut self, text: &str) -> u32 {
        let id = self.docs.len() as u32;
        let lower = text.to_lowercase();
        if lower.chars().count() < 3 {
            self.map.entry(String::new()).or_default().push(id);
        } else {
            for tri in trigrams(text) {
                self.map.entry(tri).or_default().push(id);
            }
        }
        self.docs.push(lower);
        id
    }

    /// Union of postings for every query trigram, deduped and sorted.
    /// Empty when the query yields no trigrams (< 3 chars).
    pub fn candidates(&self, query: &str) -> Vec<u32> {
        let mut hits: HashSet<u32> = HashSet::new();
        for tri in trigrams(query) {
            if let Some(postings) = self.map.get(&tri) {
                hits.extend(postings.iter().copied());
            }
        }
        let mut ids: Vec<u32> = hits.into_iter().collect();
        ids.sort_unstable();
        ids
    }

    /// True iff the stored doc `id` contains the full query as a substring
    /// (case-insensitive on both sides).
    pub fn verify(&self, id: u32, query: &str) -> bool {
        self.docs
            .get(id as usize)
            .is_some_and(|doc| doc.contains(&query.to_lowercase()))
    }

    /// Candidates filtered down to actual substring matches. Queries shorter
    /// than 3 chars produce no trigrams, so fall back to a linear scan.
    // ponytail: linear scan is O(docs); fine until corpora make p95 matter (idx-021).
    pub fn search(&self, query: &str) -> Vec<u32> {
        if trigrams(query).is_empty() {
            return (0..self.docs.len() as u32)
                .filter(|&id| self.verify(id, query))
                .collect();
        }
        self.candidates(query)
            .into_iter()
            .filter(|&id| self.verify(id, query))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_returns_increasing_ids() {
        let mut idx = TrigramIndex::default();
        assert_eq!(idx.add("hello world"), 0);
        assert_eq!(idx.add("second doc"), 1);
        assert_eq!(idx.add("third"), 2);
    }

    #[test]
    fn candidates_superset_contains_true_match() {
        let mut idx = TrigramIndex::default();
        idx.add("the quick brown fox");
        idx.add("something completely different");
        idx.add("lazy dog sleeps");
        // "brown" matches doc 0 via trigrams; others share no trigrams.
        let cands = idx.candidates("quick brown");
        assert!(cands.contains(&0), "expected doc 0 in {cands:?}");
        assert!(cands.iter().all(|&id| id < 3));
    }

    #[test]
    fn search_filters_to_actual_substring_matches() {
        let mut idx = TrigramIndex::default();
        idx.add("sysadmin tools"); // shares the "sys"/"yst" trigrams with query
        idx.add("system of a down");
        idx.add("unrelated text");
        // Candidates over-select via shared trigrams ("sys"), but only the
        // true substring match survives verify.
        assert_eq!(idx.candidates("system"), vec![0, 1]);
        assert_eq!(idx.search("system"), vec![1]);
        // Case-insensitive both directions.
        assert_eq!(idx.search("SYSTEM"), vec![1]);
        let mixed = idx.add("MiXeD CaSe");
        assert!(idx.verify(mixed, "mixed case"));
    }

    #[test]
    fn short_query_falls_back_to_linear_scan() {
        let mut idx = TrigramIndex::default();
        idx.add("alpha beta");
        idx.add("beta gamma");
        idx.add("nothing here");
        // 2-char query has no trigrams -> linear scan still finds matches.
        assert_eq!(idx.search("be"), vec![0, 1]);
        assert_eq!(idx.candidates("be"), Vec::<u32>::new());
        // Single char too: 'a' is in docs 0 and 1, not in "nothing here".
        assert_eq!(idx.search("a"), vec![0, 1]);
    }

    #[test]
    fn short_documents_index_under_empty_key_but_still_searchable() {
        let mut idx = TrigramIndex::default();
        let id = idx.add("ab");
        assert_eq!(idx.search("ab"), vec![id]);
        assert_eq!(idx.search("abc"), Vec::<u32>::new());
    }

    #[test]
    fn empty_index_yields_empty_results() {
        let idx = TrigramIndex::default();
        assert!(idx.candidates("anything").is_empty());
        assert!(idx.search("anything").is_empty());
        assert!(idx.search("").is_empty());
        assert!(!idx.verify(0, "anything"));
    }
}
