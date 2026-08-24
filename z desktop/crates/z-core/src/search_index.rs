//! Trigram inverted index for lexical candidate generation (idx-018).
//!
//! Minimal structure per Z-DESKTOP-TASKS idx-018: documents are stored
//! verbatim, their lowercased 3-grams are indexed into postings maps, and
//! queries are answered by postings-union followed by substring verify.
//! Incremental updates (idx-019) and query-path tuning (idx-020) build on
//! top of this.

use std::collections::{HashMap, HashSet};

/// A scored lexical match: doc id plus trigram-coverage score in [0, 1].
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub doc_id: u32,
    pub score: f32,
}

#[derive(Debug, Default, Clone)]
pub struct TrigramIndex {
    /// Lowercased trigram -> doc ids containing it. Texts shorter than 3
    /// chars have no trigrams and are indexed under the special key "" so
    /// they remain reachable via postings.
    map: HashMap<String, Vec<u32>>,
    /// Stored document texts by doc id; None marks a removed slot, kept
    /// reserved so later adds never reuse the id.
    docs: Vec<Option<String>>,
}

/// Lowercased character trigrams of `text`; empty vec when len < 3.
fn trigrams(text: &str) -> Vec<String> {
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    if lower.len() < 3 {
        return Vec::new();
    }
    lower.windows(3).map(|w| w.iter().collect()).collect()
}

/// Posting-map keys a lowercased doc text is indexed under: its trigrams,
/// or the single empty-string key when it has none (< 3 chars).
fn posting_keys(lower: &str) -> Vec<String> {
    let mut keys = trigrams(lower);
    if keys.is_empty() {
        keys.push(String::new());
    }
    keys
}

impl TrigramIndex {
    /// Store a document and index its lowercase trigrams. Returns its doc id.
    pub fn add(&mut self, text: &str) -> u32 {
        let id = self.docs.len() as u32;
        let lower = text.to_lowercase();
        self.index_text(id, &lower);
        self.docs.push(Some(lower));
        id
    }

    /// Re-index an existing doc in place: drop its old trigram postings,
    /// then index `text` under the same id. Errors when `id` is out of range
    /// or already removed; removed slots stay reserved either way.
    pub fn incremental_add(&mut self, id: u32, text: &str) -> Result<(), String> {
        let lower = text.to_lowercase();
        let old = self.live_doc_text(id)?;
        self.unindex(id, &old);
        self.index_text(id, &lower);
        self.docs[id as usize] = Some(lower);
        Ok(())
    }

    /// Drop every posting for `id`, making it unreachable; its slot stays
    /// reserved. Errors when `id` is out of range or already removed.
    pub fn remove(&mut self, id: u32) -> Result<(), String> {
        let old = self.live_doc_text(id)?;
        self.unindex(id, &old);
        self.docs[id as usize] = None;
        Ok(())
    }

    /// (live docs, removed-but-reserved slots) over the docs vec.
    pub fn doc_stats(&self) -> (usize, usize) {
        let live = self.docs.iter().filter(|d| d.is_some()).count();
        (live, self.docs.len() - live)
    }

    fn live_doc_text(&self, id: u32) -> Result<String, String> {
        self.docs
            .get(id as usize)
            .cloned()
            .flatten()
            .ok_or_else(|| format!("unknown or removed doc id {id}"))
    }

    fn unindex(&mut self, id: u32, lower: &str) {
        for key in posting_keys(lower) {
            if let Some(postings) = self.map.get_mut(&key) {
                postings.retain(|&d| d != id);
                if postings.is_empty() {
                    self.map.remove(&key);
                }
            }
        }
    }

    fn index_text(&mut self, id: u32, lower: &str) {
        for key in posting_keys(lower) {
            self.map.entry(key).or_default().push(id);
        }
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
            .and_then(|doc| doc.as_deref())
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

    /// Candidates ranked by query-trigram coverage (# of the query's distinct
    /// trigrams present in the doc / total), descending; ties break by
    /// ascending doc id. Substring verification is deliberately NOT applied —
    /// a verified full match contains every query trigram, so filtering first
    /// would collapse all scores to 1.0 and erase the ranking signal.
    /// Queries with no trigrams (< 3 chars) fall back to verify()'s linear
    /// scan, scoring every match 1.0 so ties resolve by doc id.
    pub fn ranked_search(&self, query: &str) -> Vec<SearchHit> {
        let qtris: HashSet<String> = trigrams(query).into_iter().collect();
        if qtris.is_empty() {
            return (0..self.docs.len() as u32)
                .filter(|&id| self.verify(id, query))
                .map(|doc_id| SearchHit { doc_id, score: 1.0 })
                .collect();
        }
        let mut hits: Vec<SearchHit> = self
            .candidates(query)
            .into_iter()
            .map(|id| {
                let dset: HashSet<String> =
                    trigrams(self.docs[id as usize].as_deref().expect("live doc"))
                        .into_iter()
                        .collect();
                let matched = qtris.iter().filter(|q| dset.contains(*q)).count();
                SearchHit {
                    doc_id: id,
                    score: matched as f32 / qtris.len() as f32,
                }
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.doc_id.cmp(&b.doc_id))
        });
        hits
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
        assert!(idx.ranked_search("anything").is_empty());
    }

    #[test]
    fn ranked_search_orders_by_trigram_coverage_then_doc_id() {
        let mut idx = TrigramIndex::default();
        idx.add("plain constant confusion"); // shares a few "configuration" trigrams
        idx.add("the configuration guide"); // contains the full query: score 1.0
        idx.add("config maps"); // shares the "conf*" trigrams only
        let hits = idx.ranked_search("configuration");
        assert_eq!(hits[0], SearchHit { doc_id: 1, score: 1.0 });
        assert!(hits[0].score > hits.last().unwrap().score);
        // Scores are non-increasing; equal coverage keeps ascending doc ids.
        for w in hits.windows(2) {
            assert!(w[0].score >= w[1].score);
            if (w[0].score - w[1].score).abs() < f32::EPSILON {
                assert!(w[0].doc_id < w[1].doc_id);
            }
        }
    }

    #[test]
    fn ranked_search_short_query_falls_back_with_full_scores() {
        let mut idx = TrigramIndex::default();
        idx.add("bbb x");
        idx.add("bbb y");
        idx.add("no match here");
        for q in ["bb", "bbb"] {
            let hits = idx.ranked_search(q);
            assert_eq!(
                hits.iter().map(|h| h.doc_id).collect::<Vec<_>>(),
                vec![0, 1],
                "query {q:?}"
            );
            assert!(hits.iter().all(|h| h.score == 1.0));
        }
    }

    #[test]
    fn incremental_add_replaces_postings_under_same_id() {
        let mut idx = TrigramIndex::default();
        let id = idx.add("alpha beta");
        assert_eq!(idx.search("alpha"), vec![id]);
        idx.incremental_add(id, "gamma delta").unwrap();
        assert_eq!(idx.search("gamma"), vec![id]);
        assert_eq!(idx.search("delta"), vec![id]);
        assert_eq!(idx.search("alpha"), Vec::<u32>::new());
        // Re-indexing below 3 chars must move the doc to/from the
        // empty-key postings cleanly.
        idx.incremental_add(id, "ok").unwrap();
        assert_eq!(idx.search("ok"), vec![id]);
        assert_eq!(idx.search("gamma"), Vec::<u32>::new());
    }

    #[test]
    fn remove_drops_hits_but_keeps_slot_reserved() {
        let mut idx = TrigramIndex::default();
        let gone = idx.add("removable content");
        let kept = idx.add("permanent material");
        idx.remove(gone).unwrap();
        assert_eq!(idx.search("removable"), Vec::<u32>::new());
        assert_eq!(idx.search("permanent"), vec![kept]);
        assert!(!idx.verify(gone, "content"));
        // Reserved: the next add skips the freed id instead of reusing it.
        assert_eq!(idx.add("fresh text"), kept + 1);
    }

    #[test]
    fn doc_stats_counts_live_and_removed() {
        let mut idx = TrigramIndex::default();
        assert_eq!(idx.doc_stats(), (0, 0));
        idx.add("one");
        let two = idx.add("two");
        idx.add("three");
        idx.remove(two).unwrap();
        assert_eq!(idx.doc_stats(), (2, 1));
    }

    #[test]
    fn unknown_or_removed_ids_error() {
        let mut idx = TrigramIndex::default();
        assert!(idx.incremental_add(0, "nope").is_err());
        assert!(idx.remove(0).is_err());
        let id = idx.add("temporary");
        idx.remove(id).unwrap();
        assert!(idx.incremental_add(id, "again").is_err());
        assert!(idx.remove(id).is_err());
    }
}
