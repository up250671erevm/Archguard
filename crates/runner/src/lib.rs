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
    /// Per-program exit codes overriding `exit_code`.
    pub codes: std::collections::HashMap<String, i32>,
    /// Programs that fail to spawn (e.g. not installed).
    pub missing: Vec<String>,
}

impl CommandRunner for MockRunner {
    fn run(&mut self, cmd: &CommandSpec) -> Result<i32> {
        self.calls.push(cmd.clone());
        if self.missing.contains(&cmd.program) {
            bail!("{}: not found", cmd.program);
        }
        Ok(*self.codes.get(&cmd.program).unwrap_or(&self.exit_code))
    }
}

/// True if the current process runs with effective uid 0.
pub fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// Parse a hook command line (shell-like quoting, but no shell is used).
pub fn parse_hook(line: &str) -> Result<CommandSpec> {
    let mut words = shell_words::split(line)
        .map_err(|e| anyhow::anyhow!("invalid hook `{line}`: {e}"))?
        .into_iter();
    let Some(program) = words.next() else {
        bail!("empty hook command");
    };
    Ok(CommandSpec {
        program,
        args: words.collect(),
    })
}

/// Outcome of [`guarded_upgrade`].
#[derive(Debug, PartialEq, Eq)]
pub enum UpgradeOutcome {
    /// Blocked by news; nothing was executed.
    Blocked,
    /// A pre-upgrade hook failed (exit code, or `None` if it couldn't start).
    HookFailed {
        hook: CommandSpec,
        code: Option<i32>,
    },
    /// Upgrade ran with this exit code.
    Ran(i32),
}

/// Refuse to upgrade if `report` is blocking; then run every pre-upgrade
/// hook (any failure blocks, fail-closed); finally run `cmd`.
pub fn guarded_upgrade(
    report: &Report,
    hooks: &[CommandSpec],
    cmd: &CommandSpec,
    runner: &mut dyn CommandRunner,
) -> Result<UpgradeOutcome> {
    if report.is_blocking() {
        return Ok(UpgradeOutcome::Blocked);
    }
    for h in hooks {
        let code = runner.run(h).ok();
        if code != Some(0) {
            return Ok(UpgradeOutcome::HookFailed {
                hook: h.clone(),
                code,
            });
        }
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
            guarded_upgrade(&report, &[], &cmd, &mut r).unwrap(),
            UpgradeOutcome::Blocked
        );
        assert!(r.calls.is_empty());
    }

    #[test]
    fn hooks_parse() {
        let h = parse_hook("aurscan --pre 'a b'").unwrap();
        assert_eq!(h.program, "aurscan");
        assert_eq!(h.args, s(&["--pre", "a b"]));
        assert!(parse_hook("  ").is_err());
        assert!(parse_hook("unterminated 'quote").is_err());
        // No shell: metacharacters are plain arguments.
        assert_eq!(parse_hook("x ; rm -rf /").unwrap().args[0], ";");
    }

    #[test]
    fn hooks_run_in_order_then_upgrade() {
        let hooks = vec![
            parse_hook("pkgwatch check").unwrap(),
            parse_hook("aurscan").unwrap(),
        ];
        let cmd = upgrade_command(Helper::Paru, Elevation::Sudo, &[], false).unwrap();
        let mut r = MockRunner::default();
        assert_eq!(
            guarded_upgrade(&Report::default(), &hooks, &cmd, &mut r).unwrap(),
            UpgradeOutcome::Ran(0)
        );
        let progs: Vec<_> = r.calls.iter().map(|c| c.program.as_str()).collect();
        assert_eq!(progs, ["pkgwatch", "aurscan", "paru"]);
    }

    #[test]
    fn failing_or_missing_hook_blocks_without_sync() {
        let hooks = vec![
            parse_hook("pkgwatch check").unwrap(),
            parse_hook("aurscan").unwrap(),
        ];
        let cmd = upgrade_command(Helper::Pacman, Elevation::Sudo, &[], false).unwrap();
        let mut r = MockRunner::default();
        r.codes.insert("aurscan".into(), 3);
        let o = guarded_upgrade(&Report::default(), &hooks, &cmd, &mut r).unwrap();
        assert_eq!(
            o,
            UpgradeOutcome::HookFailed {
                hook: hooks[1].clone(),
                code: Some(3)
            }
        );
        assert!(!r.calls.iter().any(|c| c.is_sync()));

        let mut r = MockRunner {
            missing: vec!["pkgwatch".into()],
            ..Default::default()
        };
        let o = guarded_upgrade(&Report::default(), &hooks, &cmd, &mut r).unwrap();
        assert!(matches!(o, UpgradeOutcome::HookFailed { code: None, .. }));
        assert_eq!(r.calls.len(), 1);
    }

    #[test]
    fn hooks_not_run_when_news_blocks() {
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
        let hooks = vec![parse_hook("pkgwatch").unwrap()];
        let cmd = upgrade_command(Helper::Pacman, Elevation::Sudo, &[], false).unwrap();
        let mut r = MockRunner::default();
        assert_eq!(
            guarded_upgrade(&report, &hooks, &cmd, &mut r).unwrap(),
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
            guarded_upgrade(&Report::default(), &[], &cmd, &mut r).unwrap(),
            UpgradeOutcome::Ran(3)
        );
        assert_eq!(r.calls, vec![cmd]);
    }
}
