//! # archguard (CLI library)
//!
//! Subcommand logic, written against traits so it can be tested without
//! network, a real pacman DB, or executing anything:
//! - `check`: report unacknowledged news relevant to installed packages.
//! - `ack <id|--all>`: acknowledge news.
//! - `upgrade`: run `check`; if blocking, refuse; else run the configured helper.

use anyhow::{bail, Result};
use archguard_core::check::{evaluate, resolve_ids, Report};
use archguard_core::config::Config;
use archguard_core::news::NewsSource;
use archguard_core::packages::InstalledPackages;
use archguard_core::state::State;
use archguard_runner::{guarded_upgrade, upgrade_command, CommandRunner, UpgradeOutcome};
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;

pub mod pacnew;

/// Exit code: no blocking news.
pub const EXIT_OK: i32 = 0;
/// Exit code: relevant unacknowledged news blocks upgrades.
pub const EXIT_BLOCKED: i32 = 1;

/// Guard Arch Linux upgrades against unread, relevant Arch news.
#[derive(Debug, Parser)]
#[command(name = "archguard", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check Arch news against installed packages (exit 1 if blocking).
    Check,
    /// Acknowledge news items by id/slug, or all currently relevant ones.
    Ack {
        /// News ids or slugs to acknowledge.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        ids: Vec<String>,
        /// Acknowledge every news item in the feed.
        #[arg(long)]
        all: bool,
    },
    /// Check first, then run the full system upgrade if nothing blocks.
    Upgrade {
        /// Extra arguments passed to the package manager.
        #[arg(last = true)]
        extra: Vec<String>,
    },
    /// Find, diff and merge .pacnew / .pacsave files.
    #[command(subcommand)]
    Pacnew(PacnewCmd),
}

#[derive(Debug, Subcommand)]
pub enum PacnewCmd {
    /// List .pacnew / .pacsave files under the configured scan dirs.
    #[command(alias = "scan")]
    List,
    /// Show a unified diff between the live file and its .pacnew.
    Diff {
        /// Live file or its .pacnew path.
        file: PathBuf,
    },
    /// 3-way merge a .pacnew into the live file.
    Merge {
        /// Live file or its .pacnew path.
        file: PathBuf,
        /// Do not ask for confirmation before writing.
        #[arg(long, short)]
        yes: bool,
    },
}

/// Everything the commands need, injectable for tests.
pub struct Ctx<'a> {
    pub config: Config,
    pub news: &'a dyn NewsSource,
    pub packages: &'a dyn InstalledPackages,
    pub state: State,
    pub now: i64,
    pub is_root: bool,
}

impl Ctx<'_> {
    pub fn report(&self) -> Result<Report> {
        let items = self.news.fetch()?;
        let installed = self.packages.installed()?;
        Ok(evaluate(
            &items,
            &installed,
            &self.state,
            &self.config.matching,
            self.now,
        ))
    }
}

fn print_report(r: &Report, out: &mut dyn Write) -> Result<()> {
    if !r.is_blocking() {
        writeln!(out, "archguard: no unacknowledged relevant news.")?;
        return Ok(());
    }
    writeln!(
        out,
        "archguard: {} unacknowledged relevant news item(s):",
        r.findings.len()
    )?;
    for f in &r.findings {
        let pkgs: Vec<_> = f.packages.iter().map(String::as_str).collect();
        writeln!(out, "  * [{}] {}", f.item.short_id(), f.item.title)?;
        if !pkgs.is_empty() {
            writeln!(out, "      affects installed: {}", pkgs.join(", "))?;
        }
        if let Some(l) = &f.item.link {
            writeln!(out, "      {l}")?;
        }
    }
    writeln!(
        out,
        "Read them, then run `archguard ack <id>` (or `--all`)."
    )?;
    Ok(())
}

/// `archguard check`. Returns the exit code.
pub fn cmd_check(ctx: &Ctx, out: &mut dyn Write) -> Result<i32> {
    let r = ctx.report()?;
    print_report(&r, out)?;
    Ok(if r.is_blocking() {
        EXIT_BLOCKED
    } else {
        EXIT_OK
    })
}

/// `archguard ack`. Mutates `ctx.state`; caller persists it.
pub fn cmd_ack(ctx: &mut Ctx, ids: &[String], all: bool, out: &mut dyn Write) -> Result<i32> {
    let items = ctx.news.fetch()?;
    let mut targets: Vec<String> = Vec::new();
    if all {
        targets.extend(items.iter().map(|i| i.id.clone()));
    } else {
        for arg in ids {
            if arg.trim().is_empty() {
                bail!("empty news id");
            }
            let found = resolve_ids(&items, arg);
            if found.is_empty() {
                bail!("no news item matches `{arg}`");
            }
            targets.extend(found.into_iter().map(str::to_string));
        }
    }
    let n = targets.iter().filter(|t| ctx.state.ack(t)).count();
    writeln!(out, "archguard: acknowledged {n} news item(s).")?;
    Ok(EXIT_OK)
}

/// `archguard upgrade`. Never runs a sync command while blocked.
pub fn cmd_upgrade(
    ctx: &Ctx,
    extra: &[String],
    runner: &mut dyn CommandRunner,
    out: &mut dyn Write,
) -> Result<i32> {
    let cmd = upgrade_command(ctx.config.helper, ctx.config.elevation, extra, ctx.is_root)?;
    let r = ctx.report()?;
    match guarded_upgrade(&r, &cmd, runner)? {
        UpgradeOutcome::Blocked => {
            print_report(&r, out)?;
            writeln!(out, "archguard: upgrade BLOCKED; `{cmd}` was not run.")?;
            Ok(EXIT_BLOCKED)
        }
        UpgradeOutcome::Ran(code) => Ok(code),
    }
}
