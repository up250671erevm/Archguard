//! # archguard-merge
//!
//! `.pacnew` merge helper (future role):
//! - 3-way merge (base = file from the old package in `/var/cache/pacman/pkg`,
//!   ours = current file, theirs = `.pacnew`) using the `similar` crate.
//! - On conflict, open `meld` or `kdiff3`.
//!
//! Phase 0: stub only.

/// Default pacman package cache directory.
pub const PKG_CACHE_DIR: &str = "/var/cache/pacman/pkg";

/// Returns the path of the live config file for a given `.pacnew` path.
pub fn target_of_pacnew(path: &str) -> Option<&str> {
    path.strip_suffix(".pacnew").filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_suffix() {
        assert_eq!(
            target_of_pacnew("/etc/pacman.conf.pacnew"),
            Some("/etc/pacman.conf")
        );
        assert_eq!(target_of_pacnew("/etc/pacman.conf"), None);
        assert_eq!(target_of_pacnew(".pacnew"), None);
    }
}
