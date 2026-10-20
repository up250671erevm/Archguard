//! # archguard-core
//!
//! Core logic of archguard (future role):
//! - Fetch and parse the Arch Linux news RSS feed (`reqwest` + `feed-rs`).
//! - Match news items against installed packages via libalpm (`alpm` crate).
//! - Persist acknowledged news / state under `~/.local/state/archguard`.
//! - Scan the filesystem for `.pacnew` files.
//!
//! Phase 0: stub only.

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
