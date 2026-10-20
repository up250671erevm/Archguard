//! # archguard-merge
//!
//! Line-based 3-way merge for `.pacnew` files using `similar`:
//! - base   = file from the previous package version (package cache),
//! - ours   = the live, locally modified file,
//! - theirs = the `.pacnew` shipped by the new package.
//!
//! Non-overlapping changes from both sides are combined; overlapping,
//! differing changes produce git-style conflict markers.

use similar::{capture_diff_slices, Algorithm, DiffOp, TextDiff};

/// Default pacman package cache directory.
pub const PKG_CACHE_DIR: &str = "/var/cache/pacman/pkg";

/// Returns the path of the live config file for a given `.pacnew` path.
pub fn target_of_pacnew(path: &str) -> Option<&str> {
    path.strip_suffix(".pacnew").filter(|s| !s.is_empty())
}

/// Result of a 3-way merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    pub text: String,
    pub conflicts: usize,
}

impl MergeResult {
    pub fn is_clean(&self) -> bool {
        self.conflicts == 0
    }
}

/// A change relative to base: replace base lines `[start, end)` with `lines`.
#[derive(Debug, Clone)]
struct Hunk<'a> {
    start: usize,
    end: usize,
    lines: Vec<&'a str>,
}

fn split_lines(s: &str) -> Vec<&str> {
    s.split_inclusive('\n').collect()
}

fn hunks<'a>(base: &[&'a str], other: &[&'a str]) -> Vec<Hunk<'a>> {
    capture_diff_slices(Algorithm::Myers, base, other)
        .into_iter()
        .filter_map(|op| match op {
            DiffOp::Equal { .. } => None,
            DiffOp::Delete {
                old_index, old_len, ..
            } => Some(Hunk {
                start: old_index,
                end: old_index + old_len,
                lines: vec![],
            }),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => Some(Hunk {
                start: old_index,
                end: old_index,
                lines: other[new_index..new_index + new_len].to_vec(),
            }),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => Some(Hunk {
                start: old_index,
                end: old_index + old_len,
                lines: other[new_index..new_index + new_len].to_vec(),
            }),
        })
        .collect()
}

/// Apply `side`'s hunks within base range `[s, e)`.
fn render<'a>(base: &[&'a str], side: &[Hunk<'a>], s: usize, e: usize) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut pos = s;
    for h in side.iter().filter(|h| h.start >= s && h.end <= e) {
        out.extend_from_slice(&base[pos..h.start]);
        out.extend_from_slice(&h.lines);
        pos = h.end;
    }
    out.extend_from_slice(&base[pos..e]);
    out
}

fn ensure_nl(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

/// 3-way merge of text files.
pub fn merge3(base: &str, ours: &str, theirs: &str) -> MergeResult {
    let b = split_lines(base);
    let o = split_lines(ours);
    let t = split_lines(theirs);
    let ho = hunks(&b, &o);
    let ht = hunks(&b, &t);

    // Tag and sort all hunks by position, then group overlapping ones.
    let mut all: Vec<(usize, usize, bool)> = ho
        .iter()
        .map(|h| (h.start, h.end, true))
        .chain(ht.iter().map(|h| (h.start, h.end, false)))
        .collect();
    all.sort();

    let mut out = String::new();
    let mut conflicts = 0;
    let mut pos = 0;
    let mut i = 0;
    while i < all.len() {
        let (s, mut e, _) = all[i];
        let mut has_o = false;
        let mut has_t = false;
        let mut j = i;
        // Overlap (inclusive: adjacent/same-point changes are grouped).
        while j < all.len() && all[j].0 <= e {
            e = e.max(all[j].1);
            if all[j].2 {
                has_o = true;
            } else {
                has_t = true;
            }
            j += 1;
        }
        for l in &b[pos..s] {
            out.push_str(l);
        }
        let ro = render(&b, &ho, s, e);
        let rt = render(&b, &ht, s, e);
        if has_o && has_t && ro != rt {
            conflicts += 1;
            ensure_nl(&mut out);
            out.push_str("<<<<<<< current\n");
            ro.iter().for_each(|l| out.push_str(l));
            ensure_nl(&mut out);
            out.push_str("||||||| base\n");
            b[s..e].iter().for_each(|l| out.push_str(l));
            ensure_nl(&mut out);
            out.push_str("=======\n");
            rt.iter().for_each(|l| out.push_str(l));
            ensure_nl(&mut out);
            out.push_str(">>>>>>> pacnew\n");
        } else {
            let chosen = if has_o { ro } else { rt };
            chosen.iter().for_each(|l| out.push_str(l));
        }
        pos = e;
        i = j;
    }
    for l in &b[pos..] {
        out.push_str(l);
    }
    MergeResult {
        text: out,
        conflicts,
    }
}

/// Unified diff between two texts.
pub fn unified_diff(old: &str, new: &str, old_name: &str, new_name: &str) -> String {
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(old_name, new_name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = include_str!("../tests/fixtures/base.conf");
    const OURS: &str = include_str!("../tests/fixtures/ours.conf");
    const THEIRS: &str = include_str!("../tests/fixtures/theirs.conf");
    const MERGED: &str = include_str!("../tests/fixtures/merged.conf");

    #[test]
    fn strips_suffix() {
        assert_eq!(
            target_of_pacnew("/etc/pacman.conf.pacnew"),
            Some("/etc/pacman.conf")
        );
        assert_eq!(target_of_pacnew("/etc/pacman.conf"), None);
        assert_eq!(target_of_pacnew(".pacnew"), None);
    }

    #[test]
    fn fixture_clean_merge() {
        let r = merge3(BASE, OURS, THEIRS);
        assert!(r.is_clean(), "{}", r.text);
        assert_eq!(r.text, MERGED);
    }

    #[test]
    fn identical_sides() {
        assert_eq!(merge3("a\n", "a\n", "a\n").text, "a\n");
        assert_eq!(merge3("a\n", "b\n", "b\n").text, "b\n");
        assert_eq!(merge3("", "", "x\n").text, "x\n");
    }

    #[test]
    fn one_side_only() {
        let base = "a\nb\nc\n";
        assert_eq!(merge3(base, "a\nB\nc\n", base).text, "a\nB\nc\n");
        assert_eq!(merge3(base, base, "a\nb\nc\nd\n").text, "a\nb\nc\nd\n");
        assert_eq!(merge3(base, base, "b\nc\n").text, "b\nc\n");
    }

    #[test]
    fn conflict_markers() {
        let r = merge3("a\nb\nc\n", "a\nX\nc\n", "a\nY\nc\n");
        assert_eq!(r.conflicts, 1);
        assert_eq!(
            r.text,
            "a\n<<<<<<< current\nX\n||||||| base\nb\n=======\nY\n>>>>>>> pacnew\nc\n"
        );
    }

    #[test]
    fn missing_trailing_newline() {
        let r = merge3("a\nb", "a\nX", "a\nY");
        assert_eq!(r.conflicts, 1);
        assert!(r
            .text
            .contains("X\n||||||| base\nb\n=======\nY\n>>>>>>> pacnew\n"));
    }

    #[test]
    fn diff_output() {
        let d = unified_diff("a\nb\n", "a\nc\n", "old", "new");
        assert!(d.contains("--- old"));
        assert!(d.contains("-b"));
        assert!(d.contains("+c"));
        assert!(unified_diff("a\n", "a\n", "x", "y").is_empty());
    }
}
