//! Matching news items against installed package names.

use crate::config::Matching;
use crate::news::NewsItem;
use std::collections::BTreeSet;

/// Characters allowed in Arch package names.
fn is_pkg_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '+' | '-')
}

/// Split text into candidate package-name tokens (lowercased).
pub fn tokens(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for raw in text.split(|c: char| !is_pkg_char(c)) {
        let t = raw.trim_matches(|c| matches!(c, '.' | '-' | '_'));
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        // "foo>=2.0" / "foo-2.0" style: also try without a trailing version.
        if let Some(idx) = lower.rfind('-') {
            let (head, tail) = lower.split_at(idx);
            if tail[1..].starts_with(|c: char| c.is_ascii_digit()) && !head.is_empty() {
                out.insert(head.to_string());
            }
        }
        out.insert(lower);
    }
    out
}

/// Installed packages mentioned by `item`, according to `opts`.
pub fn matched_packages(
    item: &NewsItem,
    installed: &BTreeSet<String>,
    opts: &Matching,
) -> BTreeSet<String> {
    let mut text = item.title.clone();
    if opts.match_summary {
        text.push(' ');
        text.push_str(&item.summary);
    }
    tokens(&text)
        .into_iter()
        .filter(|t| t.len() >= opts.min_name_len)
        .filter(|t| installed.contains(t))
        .filter(|t| !opts.ignore.iter().any(|i| i.eq_ignore_ascii_case(t)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, summary: &str) -> NewsItem {
        NewsItem {
            id: "x".into(),
            title: title.into(),
            summary: summary.into(),
            link: None,
            published: None,
        }
    }

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tokenizes() {
        let t = tokens("The grub-2.12 and Python3, lib32-mesa >= 24!");
        assert!(t.contains("grub"));
        assert!(t.contains("grub-2.12"));
        assert!(t.contains("python3"));
        assert!(t.contains("lib32-mesa"));
        assert!(!t.contains("lib32"));
    }

    #[test]
    fn matches_title_only_by_default() {
        let inst = set(&["foolib", "qux"]);
        let i = item("foolib >= 2.0 needs intervention", "also qux");
        let m = matched_packages(&i, &inst, &Matching::default());
        assert_eq!(m, set(&["foolib"]));
    }

    #[test]
    fn summary_matching_opt_in() {
        let inst = set(&["qux"]);
        let i = item("something", "also qux");
        let opts = Matching {
            match_summary: true,
            ..Default::default()
        };
        assert_eq!(matched_packages(&i, &inst, &opts), set(&["qux"]));
    }

    #[test]
    fn ignore_and_min_len() {
        let inst = set(&["linux", "go"]);
        let i = item("Linux and Go update", "");
        let opts = Matching {
            ignore: vec!["LINUX".into()],
            min_name_len: 3,
            ..Default::default()
        };
        assert!(matched_packages(&i, &inst, &opts).is_empty());
    }

    #[test]
    fn case_insensitive() {
        let inst = set(&["nvidia"]);
        let i = item("NVIDIA 590 drops Pascal", "");
        assert_eq!(
            matched_packages(&i, &inst, &Matching::default()),
            set(&["nvidia"])
        );
    }
}
