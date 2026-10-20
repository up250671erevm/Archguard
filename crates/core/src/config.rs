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
//! weak_words = ["foo"]    # extra common words needing `code`/"the foo package" to match
//! phrases = ["foo bar"]   # extra phrases that never name a package ("Arch Linux" is built in)
//! max_age_days = 0       # only consider news newer than N days (0 = all)
//!
//! [pacnew]
//! merge_tool = "auto"    # auto | meld | kdiff3 | none
//! scan_dirs = ["/etc"]
//! cache_dir = "/var/cache/pacman/pkg"
//!
//! [hooks]
//! # Commands run (as you, never elevated, no shell) after the news check and
//! # before the upgrade. Any non-zero exit blocks the upgrade.
//! pre_upgrade = ["pkgwatch check", "aurscan --pre-upgrade"]
//! timeout_secs = 300     # per hook; on timeout the hook is killed and the upgrade blocked
//!
//! [tray]
//! terminal = "auto"      # auto | konsole | gnome-terminal | kitty | alacritty | foot | xterm | ...
//! interval_minutes = 360 # periodic re-check (0 = only on demand)
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
    /// Extra common words that only match with a strong signal (`code`, "the X package").
    pub weak_words: Vec<String>,
    /// Extra phrases that never name a package (e.g. "arch linux" is built in).
    pub phrases: Vec<String>,
}

impl Default for Matching {
    fn default() -> Self {
        Self {
            match_summary: false,
            min_name_len: 2,
            ignore: Vec::new(),
            always_block: false,
            max_age_days: 0,
            weak_words: Vec::new(),
            phrases: Vec::new(),
        }
    }
}

/// External merge tool for conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MergeTool {
    /// First of meld / kdiff3 found in `$PATH`.
    #[default]
    Auto,
    Meld,
    Kdiff3,
    /// Never launch a GUI tool; leave conflict markers in a temp file.
    None,
}

/// `.pacnew` handling options.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pacnew {
    pub merge_tool: MergeTool,
    pub scan_dirs: Vec<String>,
    pub cache_dir: String,
}

impl Default for Pacnew {
    fn default() -> Self {
        Self {
            merge_tool: MergeTool::Auto,
            scan_dirs: vec!["/etc".into()],
            cache_dir: "/var/cache/pacman/pkg".into(),
        }
    }
}

/// External hook commands.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hooks {
    pub pre_upgrade: Vec<String>,
    /// Per-hook timeout in seconds; a hook that exceeds it is killed and
    /// blocks the upgrade. Must be > 0.
    pub timeout_secs: u64,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            pre_upgrade: Vec::new(),
            timeout_secs: 300,
        }
    }
}

/// Tray applet options.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tray {
    pub terminal: String,
    pub interval_minutes: u32,
}

impl Default for Tray {
    fn default() -> Self {
        Self {
            terminal: "auto".into(),
            interval_minutes: 360,
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
    pub pacnew: Pacnew,
    pub hooks: Hooks,
    pub tray: Tray,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            helper: Helper::default(),
            elevation: Elevation::default(),
            feed_url: crate::NEWS_FEED_URL.to_string(),
            matching: Matching::default(),
            pacnew: Pacnew::default(),
            hooks: Hooks::default(),
            tray: Tray::default(),
        }
    }
}

impl Config {
    /// Parse a config from TOML text.
    pub fn from_toml(s: &str) -> Result<Self> {
        let c: Self = toml::from_str(s).context("invalid archguard config")?;
        if c.hooks.timeout_secs == 0 {
            anyhow::bail!("invalid archguard config: hooks.timeout_secs must be > 0");
        }
        Ok(c)
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
            [pacnew]
            merge_tool = "kdiff3"
            scan_dirs = ["/etc", "/usr/share/x"]
            [hooks]
            pre_upgrade = ["pkgwatch check", "aurscan --pre-upgrade"]
            [tray]
            terminal = "kitty"
            "#,
        )
        .unwrap();
        assert_eq!(c.helper, Helper::Yay);
        assert_eq!(c.elevation, Elevation::Run0);
        assert_eq!(c.feed_url, "http://x/feed");
        assert!(c.matching.match_summary);
        assert_eq!(c.matching.ignore, vec!["linux"]);
        assert_eq!(c.matching.min_name_len, 2);
        assert_eq!(c.pacnew.merge_tool, MergeTool::Kdiff3);
        assert_eq!(c.pacnew.scan_dirs.len(), 2);
        assert_eq!(c.pacnew.cache_dir, "/var/cache/pacman/pkg");
        assert_eq!(c.hooks.pre_upgrade.len(), 2);
        assert_eq!(c.hooks.timeout_secs, 300);
        assert!(Config::from_toml("[hooks]\ntimeout_secs = 0").is_err());
        assert_eq!(
            Config::from_toml("[hooks]\ntimeout_secs = 5")
                .unwrap()
                .hooks
                .timeout_secs,
            5
        );
        assert_eq!(c.tray.terminal, "kitty");
        assert_eq!(c.tray.interval_minutes, 360);
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
