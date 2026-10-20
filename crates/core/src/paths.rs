//! XDG path resolution, with an injectable environment for tests.

use std::path::PathBuf;

/// Minimal view of the environment needed to resolve XDG paths.
pub trait Env {
    fn var(&self, key: &str) -> Option<String>;
}

/// The real process environment.
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }
}

fn xdg_dir(env: &dyn Env, var: &str, fallback: &str) -> Option<PathBuf> {
    if let Some(v) = env.var(var) {
        let p = PathBuf::from(v);
        if p.is_absolute() {
            return Some(p);
        }
    }
    env.var("HOME").map(|h| PathBuf::from(h).join(fallback))
}

/// `$XDG_CONFIG_HOME/archguard.toml` (default `~/.config/archguard.toml`).
pub fn config_file(env: &dyn Env) -> Option<PathBuf> {
    xdg_dir(env, "XDG_CONFIG_HOME", ".config").map(|d| d.join("archguard.toml"))
}

/// `$XDG_STATE_HOME/archguard/state` (default `~/.local/state/archguard/state`).
pub fn state_file(env: &dyn Env) -> Option<PathBuf> {
    xdg_dir(env, "XDG_STATE_HOME", ".local/state")
        .map(|d| d.join(crate::STATE_DIR_NAME).join("state"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    pub struct MapEnv(pub HashMap<&'static str, &'static str>);
    impl Env for MapEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.0.get(key).map(|s| s.to_string())
        }
    }

    #[test]
    fn defaults_from_home() {
        let env = MapEnv([("HOME", "/home/u")].into());
        assert_eq!(
            config_file(&env).unwrap(),
            PathBuf::from("/home/u/.config/archguard.toml")
        );
        assert_eq!(
            state_file(&env).unwrap(),
            PathBuf::from("/home/u/.local/state/archguard/state")
        );
    }

    #[test]
    fn xdg_overrides_and_relative_ignored() {
        let env = MapEnv(
            [
                ("HOME", "/home/u"),
                ("XDG_CONFIG_HOME", "/cfg"),
                ("XDG_STATE_HOME", "relative"),
            ]
            .into(),
        );
        assert_eq!(
            config_file(&env).unwrap(),
            PathBuf::from("/cfg/archguard.toml")
        );
        assert_eq!(
            state_file(&env).unwrap(),
            PathBuf::from("/home/u/.local/state/archguard/state")
        );
    }

    #[test]
    fn no_home() {
        let env = MapEnv(HashMap::new());
        assert!(config_file(&env).is_none());
    }
}
