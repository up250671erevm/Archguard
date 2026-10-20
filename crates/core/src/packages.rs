//! Installed-package listing from the **local** pacman database.
//!
//! None of these implementations ever synchronise the package databases.

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;

/// Something that can list installed package names.
pub trait InstalledPackages {
    fn installed(&self) -> Result<BTreeSet<String>>;
}

/// Fixed set of package names (tests).
#[derive(Debug, Default, Clone)]
pub struct FakePackages(pub BTreeSet<String>);

impl FakePackages {
    pub fn new<I: IntoIterator<Item = S>, S: Into<String>>(names: I) -> Self {
        Self(names.into_iter().map(Into::into).collect())
    }
}

impl InstalledPackages for FakePackages {
    fn installed(&self) -> Result<BTreeSet<String>> {
        Ok(self.0.clone())
    }
}

/// Parses `pacman -Qq` style output (one name per line).
pub fn parse_qq(out: &str) -> BTreeSet<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Fallback: runs `pacman -Qq` (local DB query, no sync, no root).
pub struct PacmanQuery;

impl InstalledPackages for PacmanQuery {
    fn installed(&self) -> Result<BTreeSet<String>> {
        let out = std::process::Command::new("pacman")
            .arg("-Qq")
            .output()
            .context("running `pacman -Qq`")?;
        if !out.status.success() {
            bail!("`pacman -Qq` failed with {}", out.status);
        }
        Ok(parse_qq(&String::from_utf8_lossy(&out.stdout)))
    }
}

/// Reads the local DB directly with libalpm (feature `alpm`).
#[cfg(feature = "alpm")]
pub struct AlpmPackages {
    pub root: String,
    pub dbpath: String,
}

#[cfg(feature = "alpm")]
impl Default for AlpmPackages {
    fn default() -> Self {
        Self {
            root: "/".into(),
            dbpath: "/var/lib/pacman".into(),
        }
    }
}

#[cfg(feature = "alpm")]
impl InstalledPackages for AlpmPackages {
    fn installed(&self) -> Result<BTreeSet<String>> {
        let handle = alpm::Alpm::new(self.root.as_str(), self.dbpath.as_str())
            .context("initialising libalpm")?;
        Ok(handle
            .localdb()
            .pkgs()
            .iter()
            .map(|p| p.name().to_string())
            .collect())
    }
}

/// libalpm major versions (sonames) this build is known to work with.
#[cfg(feature = "alpm")]
pub const SUPPORTED_LIBALPM_MAJOR: &[u32] = &[16];

/// Runtime libalpm version string.
#[cfg(feature = "alpm")]
pub fn libalpm_version() -> &'static str {
    alpm::version()
}

/// The best available implementation for this build.
pub fn system() -> Box<dyn InstalledPackages> {
    #[cfg(feature = "alpm")]
    {
        Box::new(AlpmPackages::default())
    }
    #[cfg(not(feature = "alpm"))]
    {
        Box::new(PacmanQuery)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_lists() {
        let f = FakePackages::new(["a", "b"]);
        assert_eq!(f.installed().unwrap().len(), 2);
    }

    #[test]
    fn qq_parsing() {
        let s = parse_qq("glibc\n linux \n\nfoo\n");
        assert_eq!(s.into_iter().collect::<Vec<_>>(), ["foo", "glibc", "linux"]);
    }

    #[cfg(feature = "alpm")]
    #[test]
    fn libalpm_version_is_supported() {
        let v = libalpm_version();
        let major: u32 = v.split('.').next().unwrap().parse().unwrap();
        assert!(
            SUPPORTED_LIBALPM_MAJOR.contains(&major),
            "libalpm {v} not in supported set {SUPPORTED_LIBALPM_MAJOR:?}"
        );
    }

    #[cfg(feature = "alpm")]
    #[test]
    fn alpm_reads_local_db() {
        let pkgs = AlpmPackages::default().installed().unwrap();
        assert!(pkgs.contains("pacman"));
    }
}
