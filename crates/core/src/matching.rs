//! Matching news items against installed package names.
//!
//! Plain tokens match installed package names, except:
//! - known phrases such as "Arch Linux" are removed first (built-in list plus
//!   `[matching] phrases`), so they never yield `linux`/`arch`;
//! - common English words that are also package names (`which`, `less`,
//!   `time`, ...; built-in list plus `[matching] weak_words`) only match with a
//!   strong signal: inside `backticks` / `<code>`, or explicit package context
//!   ("the X package", "package X", "X package(s)").
//!
//! `ignore` still suppresses names; `always_block` still blocks everything.

use crate::config::Matching;
use crate::news::NewsItem;
use std::collections::BTreeSet;

/// Phrases that never name a package on their own (lowercase).
pub const BUILTIN_PHRASES: &[&str] = &[
    "arch linux",
    "archlinux.org",
    "arch wiki",
    "linux foundation",
];

/// Common English words that are also Arch package names (lowercase).
pub const BUILTIN_WEAK_WORDS: &[&str] = &[
    "which", "less", "time", "file", "make", "patch", "words", "watch", "tree", "units", "most",
    "dialog", "expect", "at", "man", "info", "check", "pass", "links", "parallel", "dash", "go",
    "bc", "ed", "base",
    // Not official package names, but common in titles ("[core]", "Arch …",
    // "help wanted") and possibly installed from the AUR / local repos.
    "arch", "core", "extra", "help",
];

/// Characters allowed in Arch package names.
fn is_pkg_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '+' | '-')
}

fn clean(raw: &str) -> Option<String> {
    let t = raw.trim_matches(|c| matches!(c, '.' | '-' | '_'));
    (!t.is_empty()).then(|| t.to_ascii_lowercase())
}

/// Split text into candidate package-name tokens (lowercased).
pub fn tokens(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for raw in text.split(|c: char| !is_pkg_char(c)) {
        let Some(lower) = clean(raw) else { continue };
        // "foo-2.0" / "foo-4.4.0-1" style: also try every prefix before a
        // "-<digit>", so the whole -<version>-<pkgrel> suffix is stripped.
        out.extend(versioned_heads(&lower));
        out.insert(lower);
    }
    out
}

/// Prefixes of `tok` that precede a `-<digit>` ("screen-4.4.0-1" -> "screen", "screen-4.4.0").
fn versioned_heads(tok: &str) -> Vec<String> {
    tok.match_indices('-')
        .filter(|(i, _)| *i > 0 && tok[i + 1..].starts_with(|c: char| c.is_ascii_digit()))
        .map(|(i, _)| tok[..i].to_string())
        .collect()
}

/// Remove phrases (case-insensitive, word-bounded) by replacing them with spaces.
fn strip_phrases(text: &str, phrases: &[String]) -> String {
    let mut lower = text.to_ascii_lowercase();
    for p in phrases {
        let p = p.trim().to_ascii_lowercase();
        if p.is_empty() {
            continue;
        }
        let mut from = 0;
        while let Some(off) = lower[from..].find(&p) {
            let start = from + off;
            let end = start + p.len();
            let before_ok = lower[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !is_pkg_char(c));
            let after_ok = lower[end..].chars().next().is_none_or(|c| !is_pkg_char(c));
            if before_ok && after_ok {
                lower.replace_range(start..end, &" ".repeat(p.len()));
            }
            from = end;
        }
    }
    lower
}

/// Tokens with a strong package signal: `code`, "the X package",
/// "package X", "X package(s)" / "X pkg", or word + version
/// ("X-1.2", "X 1.2", "X >= 1.2", "X=1.2").
pub fn strong_tokens(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    // Backtick spans (HTML <code> is converted to backticks by the feed parser).
    for (i, span) in text.split('`').enumerate() {
        if i % 2 == 1 {
            out.extend(tokens(span));
        }
    }
    let words: Vec<String> = text
        .split(|c: char| !is_pkg_char(c))
        .filter_map(clean)
        .collect();
    // Word + version.
    let raw: Vec<&str> = text.split_whitespace().collect();
    for (i, r) in raw.iter().enumerate() {
        let r = r.trim_matches(|c: char| !is_pkg_char(c) && !matches!(c, '<' | '>' | '='));
        let (name, op) = match r.find(['<', '>', '=']) {
            Some(j) => (&r[..j], Some(&r[j..])),
            None => (r, None),
        };
        let Some(name) = clean(name) else { continue };
        out.extend(versioned_heads(&name));
        let digit = |s: &str| {
            s.trim_start_matches(['<', '>', '='])
                .starts_with(|c: char| c.is_ascii_digit())
        };
        let versioned = match op {
            Some(op) if op.trim_start_matches(['<', '>', '=']).is_empty() => {
                raw.get(i + 1).is_some_and(|n| digit(n))
            }
            Some(op) => digit(op),
            None => raw.get(i + 1).is_some_and(|n| {
                let n = n.trim_start_matches(['(', '[']);
                n.starts_with(|c: char| c.is_ascii_digit())
                    || (n.starts_with(['<', '>', '='])
                        && (digit(n) || raw.get(i + 2).is_some_and(|m| digit(m))))
            }),
        };
        if versioned {
            out.insert(name);
        }
    }
    let is_pkg_word = |w: &str| matches!(w, "package" | "packages" | "pkg");
    for (i, w) in words.iter().enumerate() {
        if is_pkg_word(w) {
            if i > 0 && !is_pkg_word(&words[i - 1]) {
                out.insert(words[i - 1].clone());
            }
            if w == "package" && i + 1 < words.len() && !is_pkg_word(&words[i + 1]) {
                out.insert(words[i + 1].clone());
            }
        }
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
    let phrases: Vec<String> = BUILTIN_PHRASES
        .iter()
        .map(|s| s.to_string())
        .chain(opts.phrases.iter().cloned())
        .collect();
    // Phrases are removed first so "Arch Linux 2026" is not "linux + version".
    let text = strip_phrases(&text, &phrases);
    let strong = strong_tokens(&text);
    let plain = tokens(&text);
    let weak = |t: &str| {
        BUILTIN_WEAK_WORDS.contains(&t) || opts.weak_words.iter().any(|w| w.eq_ignore_ascii_case(t))
    };
    plain
        .into_iter()
        .filter(|t| !weak(t))
        .chain(strong)
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

    fn real_installed() -> BTreeSet<String> {
        set(&[
            "which",
            "less",
            "linux",
            "linux-firmware",
            "glibc",
            "grub",
            "openssh",
            "iptables",
            "iptables-nft",
            "nftables",
            "pacman",
            "arch-install-scripts",
        ])
    }

    fn m(title: &str) -> BTreeSet<String> {
        matched_packages(&item(title, ""), &real_installed(), &Matching::default())
    }

    #[test]
    fn reported_false_positives() {
        assert!(
            m("Breaking changes for all users of varnish, which is renamed to vinyl-cache")
                .is_empty()
        );
        assert!(m("Arch Linux 2026 Leader Election Results").is_empty());
        assert!(m("Arch Linux news: less is more").is_empty());
    }

    #[test]
    fn real_arch_news_titles() {
        assert_eq!(
            m("iptables now defaults to the nft backend"),
            set(&["iptables"])
        );
        assert_eq!(
            m("linux-firmware >= 20250508.788aadc8-2 requires manual intervention"),
            set(&["linux-firmware"])
        );
        assert_eq!(
            m("Glibc 2.41 corrupting Discord installation"),
            set(&["glibc"])
        );
        assert_eq!(
            m("Grub bootloader upgrade and configuration incompatibilities"),
            set(&["grub"])
        );
        assert_eq!(
            m("OpenSSH service restart after upgrading to 9.8p1"),
            set(&["openssh"])
        );
        assert_eq!(
            m("Manual intervention for pacman 7.0.0 and local repositories"),
            set(&["pacman"])
        );
        assert_eq!(m("linux 6.20 drops support for old CPUs"), set(&["linux"]));
        assert_eq!(m("Arch Linux: linux-lts users read this"), BTreeSet::new());
    }

    #[test]
    fn weak_words_need_strong_signal() {
        assert_eq!(m("The which package moved to core"), set(&["which"]));
        assert_eq!(m("package which is deprecated"), set(&["which"]));
        assert_eq!(m("Replace `which` with command -v"), set(&["which"]));
        assert_eq!(m("which packages need a rebuild"), set(&["which"]));
        let opts = Matching {
            match_summary: true,
            ..Default::default()
        };
        let i = item("Tooling news", "Use `less` 680 or newer");
        assert_eq!(
            matched_packages(&i, &real_installed(), &opts),
            set(&["less"])
        );
        let i = item("Tooling news", "read less, do more");
        assert!(matched_packages(&i, &real_installed(), &opts).is_empty());
    }

    #[test]
    fn configurable_weak_words_phrases_ignore() {
        let inst = real_installed();
        let opts = Matching {
            weak_words: vec!["GRUB".into()],
            ..Default::default()
        };
        assert!(matched_packages(&item("grub update", ""), &inst, &opts).is_empty());
        assert_eq!(
            matched_packages(&item("the grub package", ""), &inst, &opts),
            set(&["grub"])
        );
        let opts = Matching {
            phrases: vec!["OpenSSH Summit".into()],
            ..Default::default()
        };
        assert!(matched_packages(&item("OpenSSH Summit 2026", ""), &inst, &opts).is_empty());
        let opts = Matching {
            ignore: vec!["which".into()],
            ..Default::default()
        };
        assert!(matched_packages(&item("the `which` package", ""), &inst, &opts).is_empty());
    }

    #[test]
    fn word_plus_version_is_strong() {
        let inst = set(&["screen", "ncurses", "go", "less", "which"]);
        let t = |title: &str| matched_packages(&item(title, ""), &inst, &Matching::default());
        // Real Arch news titles.
        assert_eq!(
            t("screen-4.2.0 cannot reattach older instances"),
            set(&["screen"])
        );
        assert_eq!(
            t("screen-4.2.1 cannot reattach older instances either"),
            set(&["screen"])
        );
        assert_eq!(t("screen and ncurses"), set(&["screen", "ncurses"]));
        let opts = Matching {
            match_summary: true,
            ..Default::default()
        };
        let i = item(
            "Session news",
            "screen-4.4.0-1 unable to attach old sessions",
        );
        assert_eq!(matched_packages(&i, &inst, &opts), set(&["screen"]));
        // go stays weak, but go + version is strong.
        assert!(t("Go faster with the new mirrors").is_empty());
        assert_eq!(t("Go 1.24 requires rebuilding Go packages"), set(&["go"]));
        assert_eq!(t("go-1.23.4-1 breaks cgo"), set(&["go"]));
        assert_eq!(t("less >= 680 changes defaults"), set(&["less"]));
        assert_eq!(t("less>=680 changes defaults"), set(&["less"]));
        assert_eq!(t("which=2.21 removed"), set(&["which"]));
        assert!(t("Less is more").is_empty());
        assert!(t("which 's' is it").is_empty());
    }

    #[test]
    fn strips_full_version_and_pkgrel() {
        let t = tokens("screen-4.4.0-1 and lib32-mesa-24.1.0-2");
        assert!(t.contains("screen"));
        assert!(t.contains("lib32-mesa"));
        assert!(!t.contains("lib32"));
    }
}
