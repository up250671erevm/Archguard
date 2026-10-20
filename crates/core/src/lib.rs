//! # archguard-core
//!
//! Core logic of archguard:
//! - [`news`]: fetch and parse the Arch Linux news RSS feed (`reqwest` + `feed-rs`).
//! - [`packages`]: list installed packages from the *local* pacman DB (never syncs).
//! - [`matching`]: match news items against installed packages.
//! - [`state`]: persist acknowledged news under `$XDG_STATE_HOME/archguard/state`.
//! - [`config`]: TOML config at `$XDG_CONFIG_HOME/archguard.toml`.
//! - [`check`]: combine all of the above into a verdict.
//!
//! Future: `.pacnew` scan (F2).

pub mod check;
pub mod config;
pub mod fsutil;
pub mod matching;
pub mod news;
pub mod packages;
pub mod pacnew;
pub mod paths;
pub mod state;

/// Arch Linux news RSS feed URL.
pub const NEWS_FEED_URL: &str = "https://archlinux.org/feeds/news/";

/// Name of the state subdirectory under `$XDG_STATE_HOME` (default `~/.local/state`).
pub const STATE_DIR_NAME: &str = "archguard";

/// Returns the crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_url_is_https() {
        assert!(NEWS_FEED_URL.starts_with("https://archlinux.org/"));
    }

    #[test]
    fn version_matches() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
