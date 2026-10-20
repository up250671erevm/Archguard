//! Combine news, installed packages, state and config into a verdict.

use crate::config::Matching;
use crate::matching::matched_packages;
use crate::news::NewsItem;
use crate::state::State;
use std::collections::BTreeSet;

/// A relevant, unacknowledged news item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub item: NewsItem,
    /// Installed packages that matched (empty if `always_block`).
    pub packages: BTreeSet<String>,
}

/// Result of a check.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    /// True if an upgrade must be blocked.
    pub fn is_blocking(&self) -> bool {
        !self.findings.is_empty()
    }
}

/// Evaluate news against installed packages and acknowledgements.
/// `now` is a Unix timestamp used for `max_age_days`.
pub fn evaluate(
    items: &[NewsItem],
    installed: &BTreeSet<String>,
    state: &State,
    opts: &Matching,
    now: i64,
) -> Report {
    let cutoff = (opts.max_age_days > 0).then(|| now - i64::from(opts.max_age_days) * 86_400);
    let findings = items
        .iter()
        .filter(|i| !state.is_acked(&i.id))
        .filter(|i| match (cutoff, i.published) {
            (Some(c), Some(p)) => p >= c,
            _ => true,
        })
        .filter_map(|i| {
            let packages = matched_packages(i, installed, opts);
            (opts.always_block || !packages.is_empty()).then(|| Finding {
                item: i.clone(),
                packages,
            })
        })
        .collect();
    Report { findings }
}

/// Resolve an ack argument (full id or short slug) to full ids.
pub fn resolve_ids<'a>(items: &'a [NewsItem], arg: &str) -> Vec<&'a str> {
    items
        .iter()
        .filter(|i| i.id == arg || i.short_id() == arg)
        .map(|i| i.id.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news::parse_feed;

    fn fixture() -> Vec<NewsItem> {
        parse_feed(include_bytes!("../tests/fixtures/news.xml")).unwrap()
    }

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn blocks_on_installed_package() {
        let r = evaluate(
            &fixture(),
            &set(&["foolib", "glibc"]),
            &State::default(),
            &Matching::default(),
            0,
        );
        assert!(r.is_blocking());
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].packages, set(&["foolib"]));
    }

    #[test]
    fn nothing_installed_nothing_blocks() {
        let r = evaluate(
            &fixture(),
            &set(&["glibc"]),
            &State::default(),
            &Matching::default(),
            0,
        );
        assert!(!r.is_blocking());
    }

    #[test]
    fn ack_unblocks() {
        let items = fixture();
        let mut st = State::default();
        for id in resolve_ids(&items, "foolib-20-requires-manual-intervention") {
            st.ack(id);
        }
        let r = evaluate(&items, &set(&["foolib"]), &st, &Matching::default(), 0);
        assert!(!r.is_blocking());
    }

    #[test]
    fn always_block_and_age() {
        let items = fixture();
        let now = items[0].published.unwrap() + 86_400;
        let opts = Matching {
            always_block: true,
            max_age_days: 30,
            ..Default::default()
        };
        let r = evaluate(&items, &BTreeSet::new(), &State::default(), &opts, now);
        // The 2025 item is too old.
        assert_eq!(r.findings.len(), 2);
    }

    #[test]
    fn resolve_by_full_or_short() {
        let items = fixture();
        assert_eq!(resolve_ids(&items, &items[1].id).len(), 1);
        assert_eq!(
            resolve_ids(&items, "barserver-configuration-format-changed").len(),
            1
        );
        assert!(resolve_ids(&items, "nope").is_empty());
    }
}
