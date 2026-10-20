//! # archguard-runner
//!
//! Package-manager runner:
//! - Builds the upgrade command for `pacman`, `paru` or `yay`.
//! - Elevates **only** `pacman` via `sudo` or `run0`; AUR helpers are never
//!   run as root (they elevate internally), and archguard refuses to start an
//!   AUR helper when it is itself running as root.
//! - [`guarded_upgrade`] runs the news check first and refuses to issue any
//!   `-Sy`/`-Syu` while relevant unacknowledged news exist.

use anyhow::{bail, Result};
use archguard_core::check::Report;
use archguard_core::config::{Elevation, Helper};

/// A command to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    /// True if this command would synchronise package databases (`-Sy…`).
    pub fn is_sync(&self) -> bool {
        self.args.iter().any(|a| is_sync_flag(a))
    }
}

impl std::fmt::Display for CommandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.program)?;
        for a in &self.args {
            write!(f, " {a}")?;
        }
        Ok(())
    }
}

/// Detects `-Sy`, `-Syu`, `-Syyu`, `-Suy`, `--sync --refresh` style flags.
pub fn is_sync_flag(a: &str) -> bool {
    if a == "--refresh" {
        return true;
    }
    a.starts_with("-S") && !a.starts_with("--") && a[2..].contains('y')
}

fn helper_bin(h: Helper) -> &'static str {
    match h {
        Helper::Pacman => "pacman",
        Helper::Paru => "paru",
        Helper::Yay => "yay",
    }
}

fn elevation_bin(e: Elevation) -> &'static str {
    match e {
        Elevation::Sudo => "sudo",
        Elevation::Run0 => "run0",
    }
}

/// Whether the frontend must be invoked through an elevation tool.
pub fn needs_elevation(h: Helper) -> bool {
    matches!(h, Helper::Pacman)
}

/// Build the full-upgrade command. `is_root` = archguard's own effective uid is 0.
pub fn upgrade_command(
    helper: Helper,
    elevation: Elevation,
    extra: &[String],
    is_root: bool,
) -> Result<CommandSpec> {
    if !needs_elevation(helper) && is_root {
        bail!(
            "refusing to run {} as root; run archguard as your normal user",
            helper_bin(helper)
        );
    }
    let mut args = vec!["-Syu".to_string()];
    args.extend(extra.iter().cloned());
    Ok(if needs_elevation(helper) && !is_root {
        let mut a = vec![helper_bin(helper).to_string()];
        a.extend(args);
        CommandSpec {
            program: elevation_bin(elevation).to_string(),
            args: a,
        }
    } else {
        CommandSpec {
            program: helper_bin(helper).to_string(),
            args,
        }
    })
}

/// Executes commands; mocked in tests.
pub trait CommandRunner {
    /// Run and return the exit code.
    fn run(&mut self, cmd: &CommandSpec) -> Result<i32>;
}

/// Runs commands for real, inheriting stdio.
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&mut self, cmd: &CommandSpec) -> Result<i32> {
        let st = std::process::Command::new(&cmd.program)
            .args(&cmd.args)
            .status()?;
        Ok(st.code().unwrap_or(1))
    }
}

/// Records commands without executing them.
#[derive(Debug, Default)]
pub struct MockRunner {
    pub calls: Vec<CommandSpec>,
    pub exit_code: i32,
}

impl CommandRunner for MockRunner {
    fn run(&mut self, cmd: &CommandSpec) -> Result<i32> {
        self.calls.push(cmd.clone());
        Ok(self.exit_code)
    }
}

/// True if the current process runs with effective uid 0.
pub fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// Outcome of [`guarded_upgrade`].
#[derive(Debug, PartialEq, Eq)]
pub enum UpgradeOutcome {
    /// Blocked by news; nothing was executed.
    Blocked,
    /// Upgrade ran with this exit code.
    Ran(i32),
}

/// Refuse to upgrade if `report` is blocking; otherwise run `cmd`.
pub fn guarded_upgrade(
    report: &Report,
    cmd: &CommandSpec,
    runner: &mut dyn CommandRunner,
) -> Result<UpgradeOutcome> {
    if report.is_blocking() {
        return Ok(UpgradeOutcome::Blocked);
    }
    Ok(UpgradeOutcome::Ran(runner.run(cmd)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::check::Finding;
    use archguard_core::news::NewsItem;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn pacman_is_elevated() {
        let c = upgrade_command(Helper::Pacman, Elevation::Sudo, &[], false).unwrap();
        assert_eq!(c.program, "sudo");
        assert_eq!(c.args, s(&["pacman", "-Syu"]));
        let c =
            upgrade_command(Helper::Pacman, Elevation::Run0, &s(&["--noconfirm"]), false).unwrap();
        assert_eq!(c.to_string(), "run0 pacman -Syu --noconfirm");
    }

    #[test]
    fn pacman_as_root_not_double_elevated() {
        let c = upgrade_command(Helper::Pacman, Elevation::Sudo, &[], true).unwrap();
        assert_eq!(c.program, "pacman");
    }

    #[test]
    fn aur_helpers_never_elevated() {
        for h in [Helper::Paru, Helper::Yay] {
            let c = upgrade_command(h, Elevation::Sudo, &[], false).unwrap();
            assert_eq!(c.program, helper_bin(h));
            assert!(!c.args.iter().any(|a| a == "sudo" || a == "run0"));
            assert!(upgrade_command(h, Elevation::Sudo, &[], true).is_err());
        }
    }

    #[test]
    fn sync_flag_detection() {
        for f in ["-Sy", "-Syu", "-Syyu", "-Suy", "--refresh"] {
            assert!(is_sync_flag(f), "{f}");
        }
        for f in ["-S", "-Su", "-Qq", "--sync", "-Ss"] {
            assert!(!is_sync_flag(f), "{f}");
        }
    }

    #[test]
    fn blocked_runs_nothing() {
        let report = Report {
            findings: vec![Finding {
                item: NewsItem {
                    id: "x".into(),
                    title: "t".into(),
                    summary: String::new(),
                    link: None,
                    published: None,
                },
                packages: Default::default(),
            }],
        };
        let cmd = upgrade_command(Helper::Pacman, Elevation::Sudo, &[], false).unwrap();
        let mut r = MockRunner::default();
        assert_eq!(
            guarded_upgrade(&report, &cmd, &mut r).unwrap(),
            UpgradeOutcome::Blocked
        );
        assert!(r.calls.is_empty());
    }

    #[test]
    fn clean_runs_once() {
        let cmd = upgrade_command(Helper::Yay, Elevation::Sudo, &[], false).unwrap();
        let mut r = MockRunner {
            exit_code: 3,
            ..Default::default()
        };
        assert_eq!(
            guarded_upgrade(&Report::default(), &cmd, &mut r).unwrap(),
            UpgradeOutcome::Ran(3)
        );
        assert_eq!(r.calls, vec![cmd]);
    }
}
