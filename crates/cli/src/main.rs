//! `archguard` binary: wires real implementations into the CLI library.

use anyhow::{Context, Result};
use archguard::{cmd_ack, cmd_check, cmd_upgrade, Cli, Command, Ctx};
use archguard_core::config::Config;
use archguard_core::news::HttpNews;
use archguard_core::paths::{config_file, state_file, ProcessEnv};
use archguard_core::state::State;
use archguard_runner::{running_as_root, SystemRunner};
use clap::Parser;
use std::time::{SystemTime, UNIX_EPOCH};

fn run() -> Result<i32> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        println!(
            "archguard {}: try `archguard --help`",
            archguard_core::version()
        );
        return Ok(0);
    };
    let env = ProcessEnv;
    let config = match config_file(&env) {
        Some(p) => Config::load(&p)?,
        None => Config::default(),
    };
    let state_path = state_file(&env).context("cannot determine state dir (HOME unset)")?;
    let state = State::load(&state_path)?;
    let news = HttpNews {
        url: config.feed_url.clone(),
    };
    let packages = archguard_core::packages::system();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut ctx = Ctx {
        config,
        news: &news,
        packages: packages.as_ref(),
        state,
        now,
        is_root: running_as_root(),
    };
    let mut out = std::io::stdout();
    match command {
        Command::Check => cmd_check(&ctx, &mut out),
        Command::Ack { ids, all } => {
            let code = cmd_ack(&mut ctx, &ids, all, &mut out)?;
            ctx.state.save(&state_path)?;
            Ok(code)
        }
        Command::Upgrade { extra } => cmd_upgrade(&ctx, &extra, &mut SystemRunner, &mut out),
    }
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("archguard: error: {e:#}");
            std::process::exit(2);
        }
    }
}
