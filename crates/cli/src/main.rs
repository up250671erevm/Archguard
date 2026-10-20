//! `archguard` binary: wires real implementations into the CLI library.

use anyhow::{Context, Result};
use archguard::pacnew::{
    cmd_diff, cmd_list, cmd_merge, in_path, resolve_tool, MergeOutcome, PacnewCtx,
};
use archguard::{
    cmd_ack_items, cmd_check, cmd_check_notify, cmd_upgrade, Cli, Command, Ctx, PacnewCmd,
};
use archguard_core::config::Config;
use archguard_core::news::HttpNews;
use archguard_core::notify::Notifier;
use archguard_core::pacnew::{list_cache, PacmanOwner, TarExtractor};
use archguard_core::paths::{config_file, state_file, ProcessEnv};
use archguard_core::state::State;
use archguard_runner::{running_as_root, SystemRunner};
use clap::Parser;
use std::ffi::CString;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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
    if let Command::Pacnew(sub) = command {
        return run_pacnew(&config, sub);
    }
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
    let ctx = Ctx {
        config,
        news: &news,
        packages: packages.as_ref(),
        state,
        now,
        is_root: running_as_root(),
    };
    let mut out = std::io::stdout();
    match command {
        Command::Check { notify: false } => cmd_check(&ctx, &mut out),
        Command::Check { notify: true } => {
            let pacnew = archguard_core::pacnew::scan(
                &ctx.config
                    .pacnew
                    .scan_dirs
                    .iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>(),
            )
            .len();
            let notifier = notifier();
            cmd_check_notify(&ctx, Some((notifier.as_ref(), pacnew)), &mut out)
        }
        Command::Ack { ids, all } => {
            // Fetch first (network, unlocked), then a short locked
            // read-modify-write so a concurrent tray ack isn't lost.
            let items = ctx.news.fetch()?;
            let (_, code) = State::update(&state_path, |st| {
                cmd_ack_items(st, &items, &ids, all, &mut out)
            })?;
            code
        }
        Command::Upgrade { extra } => cmd_upgrade(&ctx, &extra, &mut SystemRunner, &mut out),
        Command::Pacnew(_) => unreachable!("handled above"),
    }
}

/// Desktop notifier: notify-rust (feature `notify`), else `notify-send`.
fn notifier() -> Box<dyn Notifier> {
    #[cfg(feature = "notify")]
    {
        Box::new(archguard::notify_rust_impl::NotifyRust)
    }
    #[cfg(not(feature = "notify"))]
    {
        Box::new(archguard_core::notify::NotifySend)
    }
}

fn access_w(p: &Path) -> bool {
    let Ok(c) = CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: valid NUL-terminated path; access(2) has no other preconditions.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

fn writable(target: &Path) -> bool {
    access_w(target) && target.parent().is_some_and(access_w)
}

fn private_tmpdir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    // SAFETY: getuid cannot fail.
    let dir = base.join(format!("archguard-{}", unsafe { libc::getuid() }));
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

fn ask(q: &str) -> bool {
    print!("{q} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).is_ok()
        && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn run_pacnew(config: &Config, sub: PacnewCmd) -> Result<i32> {
    let mut out = std::io::stdout();
    let p = &config.pacnew;
    let mut runner = SystemRunner;
    let mut confirm = ask;
    let mut ctx = PacnewCtx {
        scan_dirs: p.scan_dirs.iter().map(PathBuf::from).collect(),
        owner: &PacmanOwner,
        extractor: &TarExtractor,
        cache: list_cache(Path::new(&p.cache_dir)),
        runner: &mut runner,
        confirm: &mut confirm,
        tool: resolve_tool(p.merge_tool, &in_path),
        elevation: config.elevation,
        is_root: running_as_root(),
        tmpdir: private_tmpdir()?,
        writable: &writable,
        now: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    };
    match sub {
        PacnewCmd::List => cmd_list(&ctx, &mut out),
        PacnewCmd::Diff { file } => cmd_diff(&file, &mut out),
        PacnewCmd::Merge { file, yes } => Ok(match cmd_merge(&mut ctx, &file, yes, &mut out)? {
            MergeOutcome::Installed { .. } | MergeOutcome::NothingToDo => 0,
            _ => 1,
        }),
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
