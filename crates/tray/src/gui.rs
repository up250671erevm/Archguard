//! ksni (StatusNotifierItem) front-end and notify-rust notifications.
//!
//! The tray object only renders [`Model`] and forwards menu clicks as
//! [`Action`]s over a channel; all work happens on the worker loop in
//! [`run`], which is ordinary blocking code.

use crate::{
    ack_ids, open_url_command, pick_terminal, retry_delay, run_check, terminal_command,
    transition_notification, Action, Entry, Model, RETRY_BUDGET,
};
use anyhow::{Context, Result};
use archguard_core::config::Config;
use archguard_core::news::HttpNews;
use archguard_core::notify::Notification;
use archguard_core::paths::{config_file, state_file, ProcessEnv};
use archguard_core::state::State;
use archguard_runner::CommandSpec;
use ksni::blocking::TrayMethods;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct ArchguardTray {
    model: Model,
    tx: Sender<Action>,
}

fn to_ksni(entries: Vec<Entry>) -> Vec<ksni::MenuItem<ArchguardTray>> {
    use ksni::menu::{StandardItem, SubMenu};
    entries
        .into_iter()
        .map(|e| match e {
            Entry::Separator => ksni::MenuItem::Separator,
            Entry::Label(l) => StandardItem {
                label: l,
                enabled: false,
                ..Default::default()
            }
            .into(),
            Entry::Action(label, action) => StandardItem {
                label,
                activate: Box::new(move |t: &mut ArchguardTray| {
                    let _ = t.tx.send(action.clone());
                }),
                ..Default::default()
            }
            .into(),
            Entry::Submenu(label, sub) => SubMenu {
                label,
                submenu: to_ksni(sub),
                ..Default::default()
            }
            .into(),
        })
        .collect()
}

impl ksni::Tray for ArchguardTray {
    fn id(&self) -> String {
        "archguard".into()
    }
    fn title(&self) -> String {
        self.model.title()
    }
    fn icon_name(&self) -> String {
        self.model.icon_name().into()
    }
    fn status(&self) -> ksni::Status {
        if self.model.needs_attention() {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.model.title(),
            description: self
                .model
                .findings
                .iter()
                .map(|f| format!("• {}", f.title))
                .collect::<Vec<_>>()
                .join("\n"),
            icon_name: self.model.icon_name().into(),
            icon_pixmap: Vec::new(),
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(Action::CheckNow);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        to_ksni(self.model.menu())
    }
}

fn notify(n: &Notification) {
    let _ = notify_rust::Notification::new()
        .appname("archguard")
        .icon("system-software-update")
        .summary(&n.summary)
        .body(&n.body)
        .urgency(if n.critical {
            notify_rust::Urgency::Critical
        } else {
            notify_rust::Urgency::Normal
        })
        .show();
}

fn spawn_detached(cmd: &CommandSpec) {
    if let Err(e) = std::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .spawn()
    {
        notify(&Notification {
            summary: "archguard-tray".into(),
            body: format!("could not start `{}`: {e}", cmd.program),
            critical: false,
        });
    }
}

fn in_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Command-line options of `archguard-tray`.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Retry reaching the session bus with backoff (see [`crate::RETRY_BUDGET`]).
    pub retry: bool,
}

/// Start the SNI service. `assume_sni_available` keeps the tray alive when
/// no StatusNotifierWatcher exists yet (e.g. at login before the panel):
/// ksni registers as soon as one appears. Session-bus connection errors are
/// retried with exponential backoff.
fn spawn_with_retry(
    tx: &Sender<Action>,
    retry: bool,
) -> Result<ksni::blocking::Handle<ArchguardTray>> {
    let start = std::time::Instant::now();
    let mut attempt = 0;
    loop {
        let tray = ArchguardTray {
            model: Model::default(),
            tx: tx.clone(),
        };
        match tray.assume_sni_available(true).spawn() {
            Ok(h) => return Ok(h),
            Err(e) => {
                let delay = retry_delay(attempt);
                if !retry || start.elapsed() + delay > RETRY_BUDGET {
                    return Err(e).context(
                        "starting tray (is a D-Bus session / StatusNotifierWatcher available?)",
                    );
                }
                eprintln!("archguard-tray: {e}; retrying in {}s", delay.as_secs());
                std::thread::sleep(delay);
                attempt += 1;
            }
        }
    }
}

/// Write or remove `~/.config/autostart/archguard-tray.desktop`.
pub fn set_autostart(enable: bool) -> Result<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .context("HOME unset")?;
    let path = base.join("autostart/archguard-tray.desktop");
    if enable {
        archguard_core::fsutil::write_durable(&path, crate::AUTOSTART_DESKTOP.as_bytes())?;
    } else if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(path)
}

/// Run the tray until "Quit".
pub fn run(opts: Options) -> Result<()> {
    let env = ProcessEnv;
    let cfg = match config_file(&env) {
        Some(p) => Config::load(&p)?,
        None => Config::default(),
    };
    let state_path = state_file(&env).context("cannot determine state dir (HOME unset)")?;
    let news = HttpNews {
        url: cfg.feed_url.clone(),
    };
    let packages = archguard_core::packages::system();

    let (tx, rx) = channel();
    let handle = spawn_with_retry(&tx, opts.retry)?;

    let interval = match cfg.tray.interval_minutes {
        0 => None,
        m => Some(Duration::from_secs(u64::from(m) * 60)),
    };
    let mut model = Model::default();
    let _ = tx.send(Action::CheckNow);
    loop {
        let action = match interval {
            Some(d) => match rx.recv_timeout(d) {
                Ok(a) => a,
                Err(RecvTimeoutError::Timeout) => Action::CheckNow,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(a) => a,
                Err(_) => break,
            },
        };
        match action {
            Action::CheckNow => {
                let state = State::load(&state_path).unwrap_or_default();
                let next = match run_check(&cfg, &news, packages.as_ref(), &state, now()) {
                    Ok(m) => m,
                    Err(e) => model.with_error(&format!("{e:#}")),
                };
                if let Some(n) = transition_notification(&model, &next) {
                    notify(&n);
                }
                model = next.clone();
                handle.update(move |t| t.model = next);
            }
            Action::Ack(ids) => {
                if let Err(e) = ack_ids(&state_path, &ids) {
                    notify(&Notification {
                        summary: "archguard: ack failed".into(),
                        body: format!("{e:#}"),
                        critical: false,
                    });
                }
                let _ = tx.send(Action::CheckNow);
            }
            Action::OpenUrl(u) => match open_url_command(&u) {
                Some(c) => spawn_detached(&c),
                None => notify(&Notification {
                    summary: "archguard-tray".into(),
                    body: format!("Refusing to open non-http(s) link: {u}"),
                    critical: false,
                }),
            },
            Action::RunInTerminal(argv) => {
                let env_t = std::env::var("TERMINAL").ok();
                match pick_terminal(&cfg.tray.terminal, env_t.as_deref(), &in_path) {
                    Some(t) => spawn_detached(&terminal_command(&t, &argv)),
                    None => notify(&Notification {
                        summary: "archguard-tray".into(),
                        body: "No terminal found; set [tray] terminal in archguard.toml".into(),
                        critical: false,
                    }),
                }
            }
            Action::Quit => break,
        }
    }
    handle.shutdown().wait();
    Ok(())
}
