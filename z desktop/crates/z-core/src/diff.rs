//! Minimal LCS-based unified line diff (diff-019).
//!
//! ponytail: classic O(n·m) DP over full files — fine at personal scale;
//! swap to Myers/histogram if multi-MB files ever matter.

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

/// LCS-based line diff of `old_text` vs `new_text`.
pub fn unified(old_text: &str, new_text: &str) -> Vec<DiffLine> {
    let old: Vec<&str> = if old_text.is_empty() {
        Vec::new()
    } else {
        old_text.lines().collect()
    };
    let new: Vec<&str> = if new_text.is_empty() {
        Vec::new()
    } else {
        new_text.lines().collect()
    };

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

    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old[i] == new[j] {
            out.push(DiffLine {
                kind: LineKind::Context,
                old_no: Some(i + 1),
                new_no: Some(j + 1),
                text: old[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(DiffLine {
                kind: LineKind::Removed,
                old_no: Some(i + 1),
                new_no: None,
                text: old[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: LineKind::Added,
                old_no: None,
                new_no: Some(j + 1),
                text: new[j].to_string(),
            });
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine {
            kind: LineKind::Removed,
            old_no: Some(i + 1),
            new_no: None,
            text: old[i].to_string(),
        });
        i += 1;
    }
    while j < m {
        out.push(DiffLine {
            kind: LineKind::Added,
            old_no: None,
            new_no: Some(j + 1),
            text: new[j].to_string(),
        });
        j += 1;
    }
    out
}

/// `(added, removed)` counts over a diff.
pub fn stats(lines: &[DiffLine]) -> (usize, usize) {
    let added = lines.iter().filter(|l| l.kind == LineKind::Added).count();
    let removed = lines.iter().filter(|l| l.kind == LineKind::Removed).count();
    (added, removed)
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
}
