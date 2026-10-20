//! # archguard-runner
//!
//! Package-manager runner (future role):
//! - Drive `pacman`, `paru` or `yay` for upgrades.
//! - Elevate only the commands that need it via `sudo` or `run0`;
//!   archguard itself never runs entirely as root.
//!
//! Phase 0: stub only.

/// Supported package-manager frontends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frontend {
    Pacman,
    Paru,
    Yay,
}

impl Frontend {
    /// Executable name of the frontend.
    pub fn binary(self) -> &'static str {
        match self {
            Frontend::Pacman => "pacman",
            Frontend::Paru => "paru",
            Frontend::Yay => "yay",
        }
    }

    /// Whether the frontend must be invoked through an elevation tool.
    /// AUR helpers elevate internally and must not run as root.
    pub fn needs_elevation(self) -> bool {
        matches!(self, Frontend::Pacman)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binaries() {
        assert_eq!(Frontend::Pacman.binary(), "pacman");
        assert_eq!(Frontend::Paru.binary(), "paru");
        assert_eq!(Frontend::Yay.binary(), "yay");
    }

    #[test]
    fn aur_helpers_not_elevated() {
        assert!(Frontend::Pacman.needs_elevation());
        assert!(!Frontend::Paru.needs_elevation());
        assert!(!Frontend::Yay.needs_elevation());
    }
}
