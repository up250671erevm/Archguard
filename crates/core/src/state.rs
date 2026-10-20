//! Persistent acknowledgement state (`$XDG_STATE_HOME/archguard/state`).
//!
//! Format: one acknowledged news id per line; `#` comments allowed.

use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::Path;

/// Set of acknowledged news ids.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct State {
    pub acked: BTreeSet<String>,
}

impl State {
    pub fn parse(s: &str) -> Self {
        Self {
            acked: s
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect(),
        }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::from("# archguard acknowledged news ids\n");
        for id in &self.acked {
            out.push_str(id);
            out.push('\n');
        }
        out
    }

    pub fn is_acked(&self, id: &str) -> bool {
        self.acked.contains(id)
    }

    /// Returns true if newly inserted. Empty/whitespace ids are ignored.
    pub fn ack(&mut self, id: &str) -> bool {
        let id = id.trim();
        !id.is_empty() && self.acked.insert(id.to_string())
    }

    /// Load; a missing file is an empty state.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(Self::parse(&s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Durably and atomically save: temp file, `fsync(file)`, rename over
    /// the target, `fsync(dir)`. Creates parent dirs.
    pub fn save(&self, path: &Path) -> Result<()> {
        crate::fsutil::write_durable(path, self.serialize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut s = State::default();
        assert!(s.ack("b"));
        assert!(s.ack("a"));
        assert!(!s.ack("a"));
        assert!(!s.ack(""));
        assert!(!s.ack("   "));
        assert_eq!(s.acked.len(), 2);
        let back = State::parse(&s.serialize());
        assert_eq!(back, s);
        assert!(back.is_acked("a"));
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("archguard-test-{}", std::process::id()));
        let path = dir.join("nested/state");
        let mut s = State::default();
        s.ack("tag:x");
        s.save(&path).unwrap();
        assert_eq!(State::load(&path).unwrap(), s);
        assert!(!path.with_extension("tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_is_empty() {
        assert_eq!(
            State::load(Path::new("/nonexistent/archguard/state")).unwrap(),
            State::default()
        );
    }
}
