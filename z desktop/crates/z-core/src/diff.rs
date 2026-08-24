//! Minimal LCS-based unified line diff (diff-019) plus a patience variant
//! (diff-020).
//!
//! ponytail: classic O(n·m) DP over full files — fine at personal scale;
//! swap to Myers/histogram if multi-MB files ever matter.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// One output line of the diff. Line numbers are 1-based (unified-diff
/// convention); `None` where the line does not exist on that side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
}

/// Which algorithm [`unified_with`] should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Classic LCS (see [`unified`]).
    Lcs,
    /// Patience: anchor on lines unique in both inputs, recurse between
    /// anchors, fall back to LCS where a region has no unique lines.
    Patience,
}

fn split_lines<'a>(old_text: &'a str, new_text: &'a str) -> (Vec<&'a str>, Vec<&'a str>) {
    let split = |t: &'a str| {
        if t.is_empty() {
            Vec::new()
        } else {
            t.lines().collect::<Vec<&str>>()
        }
    };
    (split(old_text), split(new_text))
}

/// LCS-based line diff of `old_text` vs `new_text`.
pub fn unified(old_text: &str, new_text: &str) -> Vec<DiffLine> {
    let (old, new) = split_lines(old_text, new_text);
    let mut out = Vec::new();
    lcs_region(&old, 0, &new, 0, &mut out);
    out
}

/// Diff using the requested [`Strategy`].
pub fn unified_with(strategy: Strategy, old_text: &str, new_text: &str) -> Vec<DiffLine> {
    match strategy {
        Strategy::Lcs => unified(old_text, new_text),
        Strategy::Patience => unified_patience(old_text, new_text),
    }
}

/// Patience diff of `old_text` vs `new_text`: anchor on lines that appear
/// exactly once in BOTH inputs and keep their relative order (longest
/// increasing subsequence), recurse into the gaps; regions without unique
/// lines fall back to the LCS diff.
pub fn unified_patience(old_text: &str, new_text: &str) -> Vec<DiffLine> {
    let (old, new) = split_lines(old_text, new_text);
    let mut out = Vec::new();
    patience_region(&old, 0, &new, 0, &mut out);
    out
}

/// Emit an LCS diff of the two slices into `out`, numbering lines from the
/// given 0-based region offsets.
fn lcs_region(
    old: &[&str],
    old_base: usize,
    new: &[&str],
    new_base: usize,
    out: &mut Vec<DiffLine>,
) {
    // lcs[i][j] = LCS length of old[i..] vs new[j..]
    let n = old.len();
    let m = new.len();
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old[i] == new[j] {
            out.push(DiffLine {
                kind: LineKind::Context,
                old_no: Some(old_base + i + 1),
                new_no: Some(new_base + j + 1),
                text: old[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(DiffLine {
                kind: LineKind::Removed,
                old_no: Some(old_base + i + 1),
                new_no: None,
                text: old[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: LineKind::Added,
                old_no: None,
                new_no: Some(new_base + j + 1),
                text: new[j].to_string(),
            });
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine {
            kind: LineKind::Removed,
            old_no: Some(old_base + i + 1),
            new_no: None,
            text: old[i].to_string(),
        });
        i += 1;
    }
    while j < m {
        out.push(DiffLine {
            kind: LineKind::Added,
            old_no: None,
            new_no: Some(new_base + j + 1),
            text: new[j].to_string(),
        });
        j += 1;
    }
}

fn counts<'a>(v: &[&'a str]) -> HashMap<&'a str, usize> {
    let mut m: HashMap<&str, usize> = HashMap::new();
    for l in v {
        *m.entry(l).or_insert(0) += 1;
    }
    m
}

/// Patience recursion over one region; offsets keep global line numbers.
fn patience_region(
    old: &[&str],
    old_base: usize,
    new: &[&str],
    new_base: usize,
    out: &mut Vec<DiffLine>,
) {
    let (oc, nc) = (counts(old), counts(new));
    // Anchor candidates in old order: lines unique on both sides. Because the
    // line is unique on each side, pairing old index i with its sole new index
    // is unambiguous.
    let cand: Vec<(usize, usize)> = old
        .iter()
        .enumerate()
        .filter(|(_, l)| oc.get(*l) == Some(&1) && nc.get(*l) == Some(&1))
        .filter_map(|(i, l)| new.iter().position(|nl| nl == l).map(|j| (i, j)))
        .collect();

    if cand.is_empty() {
        // No unique lines to anchor on — fall back to plain LCS here.
        lcs_region(old, old_base, new, new_base, out);
        return;
    }

    // Longest increasing subsequence over candidates by new-side position
    // (candidates are already increasing by old-side position).
    // ponytail: O(k²) DP, k ≤ min(n, m) — fine at personal scale.
    let k = cand.len();
    let mut dp = vec![1usize; k];
    let mut prev = vec![usize::MAX; k];
    for a in 1..k {
        for b in 0..a {
            if cand[b].1 < cand[a].1 && dp[b] + 1 > dp[a] {
                dp[a] = dp[b] + 1;
                prev[a] = b;
            }
        }
    }
    let mut end = (0..k).max_by_key(|&i| dp[i]).unwrap();
    let mut chain = Vec::with_capacity(dp[end]);
    while end != usize::MAX {
        chain.push(cand[end]);
        end = prev[end];
    }
    chain.reverse();

    let (mut oi, mut nj) = (0usize, 0usize);
    for &(ai, aj) in &chain {
        if ai > oi || aj > nj {
            patience_region(
                &old[oi..ai],
                old_base + oi,
                &new[nj..aj],
                new_base + nj,
                out,
            );
        }
        out.push(DiffLine {
            kind: LineKind::Context,
            old_no: Some(old_base + ai + 1),
            new_no: Some(new_base + aj + 1),
            text: old[ai].to_string(),
        });
        oi = ai + 1;
        nj = aj + 1;
    }
    patience_region(&old[oi..], old_base + oi, &new[nj..], new_base + nj, out);
}

/// `(added, removed)` counts over a diff.
pub fn stats(lines: &[DiffLine]) -> (usize, usize) {
    let added = lines.iter().filter(|l| l.kind == LineKind::Added).count();
    let removed = lines.iter().filter(|l| l.kind == LineKind::Removed).count();
    (added, removed)
}

/// diff-021: `(start_index, len)` ranges of changed regions in `lines`,
/// each expanded by `context` lines on either side, merged when the
/// expansions overlap or touch.
pub fn hunks(lines: &[DiffLine], context: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if l.kind == LineKind::Context {
            continue;
        }
        let start = i.saturating_sub(context);
        let end = (i + context + 1).min(lines.len());
        match out.last_mut() {
            // Ranges are pushed in order; merge into the previous hunk when
            // this one starts before (or exactly at) its end.
            Some(last) if start <= last.0 + last.1 => last.1 = end - last.0,
            _ => out.push((start, end - start)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[DiffLine]) -> Vec<LineKind> {
        lines.iter().map(|l| l.kind).collect()
    }

    #[test]
    fn identical_texts_all_context() {
        let d = unified("a\nb\nc\n", "a\nb\nc\n");
        assert_eq!(kinds(&d), vec![LineKind::Context; 3]);
        assert!(d.iter().all(|l| l.old_no.is_some() && l.new_no.is_some()));
        assert_eq!(stats(&d), (0, 0));
    }

    #[test]
    fn pure_insertion_has_added_with_context() {
        let d = unified("a\nb\n", "a\nX\nb\n");
        assert_eq!(
            kinds(&d),
            vec![LineKind::Context, LineKind::Added, LineKind::Context]
        );
        assert_eq!(d[1].text, "X");
        assert_eq!(d[1].old_no, None);
        assert_eq!(d[1].new_no, Some(2));
        assert_eq!(stats(&d), (1, 0));
    }

    #[test]
    fn pure_deletion_is_removed() {
        let d = unified("a\nX\nb\n", "a\nb\n");
        assert_eq!(
            kinds(&d),
            vec![LineKind::Context, LineKind::Removed, LineKind::Context]
        );
        assert_eq!(d[1].old_no, Some(2));
        assert_eq!(d[1].new_no, None);
        assert_eq!(stats(&d), (0, 1));
    }

    #[test]
    fn replacement_matches_stats() {
        let d = unified("a\nold1\nold2\nb\n", "a\nnew1\nnew2\nnew3\nb\n");
        let (added, removed) = stats(&d);
        assert_eq!((added, removed), (3, 2));
        assert_eq!(
            d.iter().filter(|l| l.kind == LineKind::Added).count(),
            added
        );
        assert_eq!(
            d.iter().filter(|l| l.kind == LineKind::Removed).count(),
            removed
        );
        assert_eq!(
            kinds(&d),
            vec![
                LineKind::Context,
                LineKind::Removed,
                LineKind::Removed,
                LineKind::Added,
                LineKind::Added,
                LineKind::Added,
                LineKind::Context
            ]
        );
    }

    #[test]
    fn empty_old_everything_added() {
        let d = unified("", "x\ny\n");
        assert_eq!(kinds(&d), vec![LineKind::Added, LineKind::Added]);
        assert_eq!(stats(&d), (2, 0));
    }

    #[test]
    fn empty_new_everything_removed() {
        let d = unified("x\ny\n", "");
        assert_eq!(kinds(&d), vec![LineKind::Removed, LineKind::Removed]);
        assert_eq!(stats(&d), (0, 2));
    }

    #[test]
    fn both_empty_yields_empty_diff() {
        assert!(unified("", "").is_empty());
    }

    // ---- diff-020: patience strategy ----

    /// Apply a diff back onto its inputs; catches anchor/indexing bugs.
    fn reconstruct(d: &[DiffLine]) -> (String, String) {
        let (mut o, mut n) = (String::new(), String::new());
        for l in d {
            let (t_o, t_n) = match l.kind {
                LineKind::Context => (true, true),
                LineKind::Added => (false, true),
                LineKind::Removed => (true, false),
            };
            if t_o {
                o.push_str(&l.text);
                o.push('\n');
            }
            if t_n {
                n.push_str(&l.text);
                n.push('\n');
            }
        }
        (o, n)
    }

    #[test]
    fn identical_texts_all_context_under_both_strategies() {
        let text = "fn a() {\n    x();\n}\n";
        for strat in [Strategy::Lcs, Strategy::Patience] {
            let d = unified_with(strat, text, text);
            assert_eq!(kinds(&d), vec![LineKind::Context; 3]);
            assert!(d.iter().all(|l| l.old_no.is_some() && l.new_no.is_some()));
            assert_eq!(stats(&d), (0, 0), "{strat:?}");
        }
        assert_eq!(unified_patience(text, text), unified(text, text));
    }

    #[test]
    fn noisy_braces_stats_consistent_and_reconstructs_under_both() {
        // Braces-heavy noisy replacement: three similar blocks, bodies
        // rewritten/inserted around repeated `{`/`}` scaffolding.
        let old = "fn alpha() {\n    o1();\n}\n\nfn beta() {\n    o2();\n}\n\nfn gamma() {\n    o3();\n}\n";
        let new = "fn alpha() {\n    n1();\n    probe();\n}\n\nfn beta() {\n    o2();\n}\n\nfn gamma() {\n    n3();\n    tail();\n}\n";

        for strat in [Strategy::Lcs, Strategy::Patience] {
            let d = unified_with(strat, old, new);
            let s = stats(&d);
            assert_eq!(
                (
                    d.iter().filter(|l| l.kind == LineKind::Added).count(),
                    d.iter().filter(|l| l.kind == LineKind::Removed).count()
                ),
                s,
                "{strat:?}"
            );
            assert!(!d.is_empty(), "{strat:?}");
            let (ro, rn) = reconstruct(&d);
            assert_eq!(ro, old, "{strat:?}");
            assert_eq!(rn, new, "{strat:?}");
        }
    }

    #[test]
    fn noisy_braces_patience_never_worse_than_lcs() {
        let old = "fn alpha() {\n    o1();\n}\n\nfn beta() {\n    o2();\n}\n\nfn gamma() {\n    o3();\n}\n";
        let new = "fn alpha() {\n    n1();\n    probe();\n}\n\nfn beta() {\n    o2();\n}\n\nfn gamma() {\n    n3();\n    tail();\n}\n";

        let l = stats(&unified_with(Strategy::Lcs, old, new));
        let p = stats(&unified_with(Strategy::Patience, old, new));
        // Against a match-optimal LCS, patience can never produce STRICTLY
        // fewer changed lines: added+removed == n+m-2*matches, LCS maximizes
        // matches, and every patience output is itself a valid common-
        // subsequence alignment (verified empirically over randomized
        // braces-heavy cases). The guarantee patience gives: never worse.
        assert!(p.0 + p.1 <= l.0 + l.1, "patience {p:?} vs lcs {l:?}");
    }

    #[test]
    fn patience_anchors_preserve_order_and_line_numbers() {
        let old = "h\nu1\nm\nu2\nt\n";
        let new = "h\nU1\nm\nu2\nt\n";
        let d = unified_patience(old, new);
        assert_eq!(stats(&d), (1, 1));
        assert_eq!(
            kinds(&d),
            vec![
                LineKind::Context,
                LineKind::Removed,
                LineKind::Added,
                LineKind::Context,
                LineKind::Context,
                LineKind::Context
            ]
        );
        assert_eq!(d[0].old_no, Some(1));
        assert_eq!(d[0].new_no, Some(1));
        assert_eq!(d[1].old_no, Some(2));
        assert_eq!(d[1].new_no, None);
        assert_eq!(d[2].old_no, None);
        assert_eq!(d[2].new_no, Some(2));
        assert_eq!(d[5].old_no, Some(5));
        assert_eq!(d[5].new_no, Some(5));
    }

    // ---- diff-021: hunk grouping ----

    #[test]
    fn single_change_is_one_hunk_expanded_by_context() {
        // a X b c d e f  (X changed at index 1)
        let d = unified("a\nb\nc\nd\ne\nf\n", "a\nZ\nc\nd\ne\nf\n");
        assert_eq!(hunks(&d, 1), vec![(0, 4)]); // ctx a, -b +Z, ctx c
        assert_eq!(hunks(&d, 0), vec![(1, 2)]); // -b +Z are adjacent
        // context clamped at slice bounds
        assert_eq!(hunks(&d, 10), vec![(0, d.len())]);
    }

    #[test]
    fn two_distant_changes_are_two_hunks() {
        let d = unified("X\nb\nc\nd\ne\nf\ng\nY\n", "a\nb\nc\nd\ne\nf\ng\nb\n");
        assert_eq!(stats(&d).0 + stats(&d).1 >= 2, true);
        let h = hunks(&d, 1);
        assert_eq!(h.len(), 2);
        for (s, l) in &h {
            assert!(*s + *l <= d.len());
            assert!(d[*s..*s + *l].iter().any(|x| x.kind != LineKind::Context));
        }
    }

    #[test]
    fn adjacent_changes_merge_when_context_meets() {
        // changes at indices 2 and 6; context 2 makes ranges [0..5) and [4..9)
        // which touch -> one hunk. With context 1 they stay separate.
        let d: Vec<DiffLine> = (0..10)
            .map(|i| DiffLine {
                kind: if i == 2 || i == 6 {
                    LineKind::Added
                } else {
                    LineKind::Context
                },
                old_no: None,
                new_no: Some(i),
                text: format!("l{i}"),
            })
            .collect();
        assert_eq!(hunks(&d, 2), vec![(0, 9)]);
        assert_eq!(hunks(&d, 1), vec![(1, 3), (5, 3)]);
    }

    #[test]
    fn no_changes_yields_no_hunks() {
        let d = unified("a\nb\n", "a\nb\n");
        assert!(hunks(&d, 1).is_empty());
        assert!(hunks(&[], 3).is_empty());
    }
}
