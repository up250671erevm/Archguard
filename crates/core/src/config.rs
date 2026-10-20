//! User configuration (`~/.config/archguard.toml`).
//!
//! ```toml
//! helper = "paru"        # pacman | paru | yay
//! elevation = "sudo"     # sudo | run0 (only used for pacman)
//! feed_url = "https://archlinux.org/feeds/news/"
//!
//! [matching]
//! match_summary = false  # also scan the news body, not only the title
//! min_name_len = 2       # ignore installed package names shorter than this
//! ignore = ["linux"]     # package names that never trigger a block
//! always_block = false   # treat every unacked news item as relevant
//! max_age_days = 0       # only consider news newer than N days (0 = all)
//! ```

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Package manager frontend used for upgrades.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Helper {
    #[default]
    Pacman,
    Paru,
    Yay,
}

/// Privilege elevation tool (only used for `pacman`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Elevation {
    #[default]
    Sudo,
    Run0,
}

/// Matching options.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Matching {
    pub match_summary: bool,
    pub min_name_len: usize,
    pub ignore: Vec<String>,
    pub always_block: bool,
    pub max_age_days: u32,
}

impl Default for Matching {
    fn default() -> Self {
        Self {
            match_summary: false,
            min_name_len: 2,
            ignore: Vec::new(),
            always_block: false,
            max_age_days: 0,
        }
    }
}

/// Full configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub helper: Helper,
    pub elevation: Elevation,
    pub feed_url: String,
    pub matching: Matching,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            helper: Helper::default(),
            elevation: Elevation::default(),
            feed_url: crate::NEWS_FEED_URL.to_string(),
            matching: Matching::default(),
        }
    }
}

impl Config {
    /// Parse a config from TOML text.
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).context("invalid archguard config")
    }

    /// Load from `path`; a missing file yields the defaults.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s).with_context(|| format!("in {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_default() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
        assert_eq!(Config::default().feed_url, crate::NEWS_FEED_URL);
    }

    #[test]
    fn full_config() {
        let c = Config::from_toml(
            r#"
            helper = "yay"
            elevation = "run0"
            feed_url = "http://x/feed"
            [matching]
            match_summary = true
            ignore = ["linux"]
            "#,
        )
        .unwrap();
        assert_eq!(c.helper, Helper::Yay);
        assert_eq!(c.elevation, Elevation::Run0);
        assert_eq!(c.feed_url, "http://x/feed");
        assert!(c.matching.match_summary);
        assert_eq!(c.matching.ignore, vec!["linux"]);
        assert_eq!(c.matching.min_name_len, 2);
    }

    #[test]
    fn rejects_unknown_and_bad_values() {
        assert!(Config::from_toml("helper = \"apt\"").is_err());
        assert!(Config::from_toml("bogus = 1").is_err());
    }

    #[test]
    fn missing_file_is_default() {
        let c = Config::load(Path::new("/nonexistent/archguard.toml")).unwrap();
        assert_eq!(c, Config::default());
    }
}
