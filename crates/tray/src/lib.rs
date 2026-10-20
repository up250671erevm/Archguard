//! # archguard-tray
//!
//! Tray applet logic, kept free of D-Bus so it can be unit-tested. The
//! `gui` feature adds the ksni (StatusNotifierItem) front-end and
//! notify-rust notifications in [`gui`] and the `archguard-tray` binary.

use archguard_core::check::Report;
use archguard_core::notify::Notification;
use archguard_runner::CommandSpec;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[cfg(feature = "gui")]
pub mod gui;

/// Arch news page, opened when a news item has no link.
pub const NEWS_PAGE: &str = "https://archlinux.org/news/";

/// One news finding as shown in the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingView {
    pub id: String,
    pub title: String,
    pub link: Option<String>,
}

/// Overall applet status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Not checked yet.
    Unknown,
    Ok,
    /// Only .pacnew/.pacsave pending.
    Pacnew(usize),
    /// Relevant unacknowledged news blocks upgrades.
    Blocking(usize),
    Error(String),
}

/// Everything the tray displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub status: Status,
    pub findings: Vec<FindingView>,
    pub pacnew: Vec<PathBuf>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            status: Status::Unknown,
            findings: Vec::new(),
            pacnew: Vec::new(),
        }
    }
}

impl Model {
    /// Build from a check result and the pacnew scan.
    pub fn from_check(report: &Report, pacnew: Vec<PathBuf>) -> Self {
        let findings: Vec<_> = report
            .findings
            .iter()
            .map(|f| FindingView {
                id: f.item.id.clone(),
                title: f.item.display_title().to_string(),
                link: f.item.link.clone(),
            })
            .collect();
        let status = if !findings.is_empty() {
            Status::Blocking(findings.len())
        } else if !pacnew.is_empty() {
            Status::Pacnew(pacnew.len())
        } else {
            Status::Ok
        };
        Self {
            status,
            findings,
            pacnew,
        }
    }

    /// A failed check keeps the last known findings (fail-closed display).
    pub fn with_error(&self, err: &str) -> Self {
        Self {
            status: Status::Error(err.to_string()),
            ..self.clone()
        }
    }

    /// Freedesktop icon name.
    pub fn icon_name(&self) -> &'static str {
        match self.status {
            Status::Ok => "security-high",
            Status::Pacnew(_) => "dialog-information",
            Status::Blocking(_) => "security-low",
            Status::Error(_) => "dialog-error",
            Status::Unknown => "security-medium",
        }
    }

    /// One-line status text.
    pub fn title(&self) -> String {
        match &self.status {
            Status::Unknown => "archguard: checking…".into(),
            Status::Ok => "archguard: OK — safe to upgrade".into(),
            Status::Pacnew(n) => format!("archguard: {n} .pacnew/.pacsave pending"),
            Status::Blocking(n) => format!("archguard: upgrade blocked by {n} news item(s)"),
            Status::Error(e) => format!("archguard: check failed: {e}"),
        }
    }

    /// Whether the tray should request attention (SNI `NeedsAttention`).
    pub fn needs_attention(&self) -> bool {
        matches!(self.status, Status::Blocking(_) | Status::Error(_))
    }

    /// Menu description, independent of ksni.
    pub fn menu(&self) -> Vec<Entry> {
        let mut m = vec![
            Entry::Label(self.title()),
            Entry::Separator,
            Entry::Action("Check now".into(), Action::CheckNow),
            Entry::Action("Open Arch news".into(), Action::OpenUrl(NEWS_PAGE.into())),
        ];
        if !self.findings.is_empty() {
            m.push(Entry::Separator);
            for f in &self.findings {
                m.push(Entry::Submenu(
                    f.title.clone(),
                    vec![
                        Entry::Action(
                            "Read".into(),
                            Action::OpenUrl(f.link.clone().unwrap_or_else(|| NEWS_PAGE.into())),
                        ),
                        Entry::Action("Acknowledge".into(), Action::Ack(vec![f.id.clone()])),
                    ],
                ));
            }
            m.push(Entry::Action(
                "Acknowledge all shown".into(),
                Action::Ack(self.findings.iter().map(|f| f.id.clone()).collect()),
            ));
        }
        if !self.pacnew.is_empty() {
            m.push(Entry::Separator);
            m.push(Entry::Action(
                format!("Review {} .pacnew/.pacsave in terminal", self.pacnew.len()),
                Action::RunInTerminal(vec!["archguard".into(), "pacnew".into(), "list".into()]),
            ));
        }
        m.push(Entry::Separator);
        m.push(Entry::Action(
            "Run upgrade in terminal".into(),
            Action::RunInTerminal(vec!["archguard".into(), "upgrade".into()]),
        ));
        m.push(Entry::Action("Quit".into(), Action::Quit));
        m
    }
}

/// Menu entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Label(String),
    Separator,
    Action(String, Action),
    Submenu(String, Vec<Entry>),
}

/// User actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    CheckNow,
    OpenUrl(String),
    Ack(Vec<String>),
    RunInTerminal(Vec<String>),
    Quit,
}

/// Notification to show when going from `prev` to `next`: only for *new*
/// blocking news ids or a growing pacnew count, so periodic checks don't spam.
pub fn transition_notification(prev: &Model, next: &Model) -> Option<Notification> {
    if let Status::Error(e) = &next.status {
        // Notify once when checks start failing, not on every retry.
        return (!matches!(prev.status, Status::Error(_)))
            .then(|| archguard_core::notify::failure(e));
    }
    let old: BTreeSet<_> = prev.findings.iter().map(|f| &f.id).collect();
    let new: Vec<_> = next
        .findings
        .iter()
        .filter(|f| !old.contains(&f.id))
        .collect();
    if !new.is_empty() {
        let body = new
            .iter()
            .take(5)
            .map(|f| format!("• {}", f.title))
            .collect::<Vec<_>>()
            .join("\n");
        return Some(Notification {
            summary: format!("archguard: {} new news item(s) block upgrades", new.len()),
            body,
            critical: true,
        });
    }
    if next.pacnew.len() > prev.pacnew.len() {
        return Some(Notification {
            summary: "archguard: configuration files need attention".into(),
            body: format!("{} .pacnew/.pacsave file(s) pending", next.pacnew.len()),
            critical: false,
        });
    }
    None
}

/// Known terminals and how they take a command.
const TERMINALS: &[(&str, &[&str])] = &[
    ("konsole", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("kgx", &["--"]),
    ("xfce4-terminal", &["-x"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("foot", &[]),
    ("wezterm", &["start", "--"]),
    ("xterm", &["-e"]),
];

/// Pick a terminal: explicit config, else `$TERMINAL`, else the first known
/// one that `available` reports as installed.
pub fn pick_terminal(
    configured: &str,
    env_terminal: Option<&str>,
    available: &dyn Fn(&str) -> bool,
) -> Option<String> {
    if configured != "auto" && !configured.is_empty() {
        return Some(configured.to_string());
    }
    if let Some(t) = env_terminal.filter(|t| !t.is_empty()) {
        return Some(t.to_string());
    }
    TERMINALS
        .iter()
        .map(|(n, _)| *n)
        .find(|n| available(n))
        .map(str::to_string)
}

/// Command that runs `argv` in `terminal` and waits for Enter afterwards so
/// the user can read the output. `argv` is passed as positional parameters
/// to `sh`, never interpolated into the script.
pub fn terminal_command(terminal: &str, argv: &[String]) -> CommandSpec {
    let prefix: &[&str] = TERMINALS
        .iter()
        .find(|(n, _)| *n == terminal)
        .map(|(_, p)| *p)
        .unwrap_or(&["-e"]);
    let mut args: Vec<String> = prefix.iter().map(|s| s.to_string()).collect();
    args.extend([
        "sh".into(),
        "-c".into(),
        r#""$@"; printf '\n[archguard] done (exit %s). Press Enter to close.' "$?"; read -r _"#
            .into(),
        "archguard-tray".into(),
    ]);
    args.extend(argv.iter().cloned());
    CommandSpec {
        program: terminal.to_string(),
        args,
    }
}

/// True for absolute `http://` / `https://` URLs with a host and no
/// whitespace or control characters (feed content is untrusted).
pub fn is_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    let Some(rest) = rest else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty()
        && !host.starts_with('-')
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Command to open a URL in the user's browser; `None` unless [`is_web_url`].
pub fn open_url_command(url: &str) -> Option<CommandSpec> {
    is_web_url(url).then(|| CommandSpec {
        program: "xdg-open".into(),
        args: vec![url.into()],
    })
}

/// Delay before the `attempt`-th (0-based) retry to reach the session bus:
/// 1, 2, 4, … capped at 30 s.
pub fn retry_delay(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_secs(1u64 << attempt.min(5)).min(std::time::Duration::from_secs(30))
}

/// Total time the tray keeps retrying before giving up.
pub const RETRY_BUDGET: std::time::Duration = std::time::Duration::from_secs(300);

/// Contents of the autostart entry written by `archguard-tray --autostart enable`.
pub const AUTOSTART_DESKTOP: &str = include_str!("../../../packaging/archguard-tray.desktop");

/// Run a full check (news + installed packages + acks + pacnew scan).
/// Never syncs package databases.
pub fn run_check(
    cfg: &archguard_core::config::Config,
    news: &dyn archguard_core::news::NewsSource,
    packages: &dyn archguard_core::packages::InstalledPackages,
    state: &archguard_core::state::State,
    now: i64,
) -> anyhow::Result<Model> {
    let items = news.fetch()?;
    let installed = packages.installed()?;
    let report = archguard_core::check::evaluate(&items, &installed, state, &cfg.matching, now);
    let dirs: Vec<PathBuf> = cfg.pacnew.scan_dirs.iter().map(PathBuf::from).collect();
    let pacnew = archguard_core::pacnew::scan(&dirs)
        .into_iter()
        .map(|l| l.path)
        .collect();
    Ok(Model::from_check(&report, pacnew))
}

/// Acknowledge `ids` in the state file at `path` (load, add, durable save).
pub fn ack_ids(path: &std::path::Path, ids: &[String]) -> anyhow::Result<usize> {
    let (_, n) =
        archguard_core::state::State::update(path, |st| ids.iter().filter(|i| st.ack(i)).count())?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::check::Finding;
    use archguard_core::news::NewsItem;

    fn report(ids: &[&str]) -> Report {
        Report {
            findings: ids
                .iter()
                .map(|i| Finding {
                    item: NewsItem {
                        id: i.to_string(),
                        title: format!("title {i}"),
                        summary: String::new(),
                        link: (*i != "nolink").then(|| format!("https://archlinux.org/news/{i}/")),
                        published: None,
                    },
                    packages: Default::default(),
                })
                .collect(),
        }
    }

    #[test]
    fn status_priorities() {
        assert_eq!(Model::from_check(&report(&[]), vec![]).status, Status::Ok);
        let m = Model::from_check(&report(&[]), vec!["/etc/a.pacnew".into()]);
        assert_eq!(m.status, Status::Pacnew(1));
        assert!(!m.needs_attention());
        let m = Model::from_check(&report(&["a"]), vec!["/etc/a.pacnew".into()]);
        assert_eq!(m.status, Status::Blocking(1));
        assert!(m.needs_attention());
        assert_eq!(m.icon_name(), "security-low");
        assert!(m.title().contains("blocked by 1"));
        assert_eq!(Model::default().icon_name(), "security-medium");
    }

    #[test]
    fn error_keeps_findings() {
        let m = Model::from_check(&report(&["a"]), vec![]).with_error("offline");
        assert!(matches!(m.status, Status::Error(_)));
        assert_eq!(m.findings.len(), 1);
        assert!(m.needs_attention());
    }

    #[test]
    fn menu_has_actions() {
        let m = Model::from_check(&report(&["a", "nolink"]), vec!["/etc/x.pacnew".into()]);
        let menu = m.menu();
        let actions: Vec<_> = menu
            .iter()
            .filter_map(|e| match e {
                Entry::Action(_, a) => Some(a.clone()),
                _ => None,
            })
            .collect();
        assert!(actions.contains(&Action::CheckNow));
        assert!(actions.contains(&Action::Quit));
        assert!(actions.contains(&Action::Ack(vec!["a".into(), "nolink".into()])));
        assert!(actions.contains(&Action::RunInTerminal(vec![
            "archguard".into(),
            "upgrade".into()
        ])));
        let subs: Vec<_> = menu
            .iter()
            .filter_map(|e| match e {
                Entry::Submenu(_, s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(subs.len(), 2);
        assert_eq!(
            subs[1][0],
            Entry::Action("Read".into(), Action::OpenUrl(NEWS_PAGE.into()))
        );
        // OK state: no ack entries.
        let ok = Model::from_check(&report(&[]), vec![]).menu();
        assert!(!ok
            .iter()
            .any(|e| matches!(e, Entry::Action(_, Action::Ack(_)))));
    }

    #[test]
    fn notifications_only_on_new_things() {
        let a = Model::from_check(&report(&["a"]), vec![]);
        let ab = Model::from_check(&report(&["a", "b"]), vec![]);
        assert!(
            transition_notification(&Model::default(), &a)
                .unwrap()
                .critical
        );
        assert!(transition_notification(&a, &a).is_none());
        let n = transition_notification(&a, &ab).unwrap();
        assert!(n.summary.contains("1 new") && n.body.contains("title b"));
        let p = Model::from_check(&report(&[]), vec!["/etc/x.pacnew".into()]);
        let ok = Model::from_check(&report(&[]), vec![]);
        assert!(!transition_notification(&ok, &p).unwrap().critical);
        assert!(transition_notification(&p, &ok).is_none());
    }

    #[test]
    fn check_and_ack_roundtrip() {
        use archguard_core::config::Config;
        use archguard_core::news::{parse_feed, StaticNews};
        use archguard_core::packages::FakePackages;
        use archguard_core::state::State;
        let feed = include_bytes!("../../core/tests/fixtures/news.xml");
        let news = StaticNews(parse_feed(feed).unwrap());
        let pkgs = FakePackages::new(["foolib"]);
        let dir = std::env::temp_dir().join(format!("ag-tray-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::write(dir.join("etc/a.conf.pacnew"), "").unwrap();
        let mut cfg = Config::default();
        cfg.pacnew.scan_dirs = vec![dir.join("etc").display().to_string()];
        let state_path = dir.join("state");

        let m = run_check(&cfg, &news, &pkgs, &State::default(), 0).unwrap();
        assert_eq!(m.status, Status::Blocking(1));
        assert_eq!(m.pacnew.len(), 1);

        let ids: Vec<_> = m.findings.iter().map(|f| f.id.clone()).collect();
        assert_eq!(ack_ids(&state_path, &ids).unwrap(), 1);
        assert_eq!(ack_ids(&state_path, &ids).unwrap(), 0);
        let st = State::load(&state_path).unwrap();
        let m = run_check(&cfg, &news, &pkgs, &st, 0).unwrap();
        assert_eq!(m.status, Status::Pacnew(1));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn terminal_selection() {
        let none = |_: &str| false;
        let has = |t: &str| t == "alacritty" || t == "xterm";
        assert_eq!(
            pick_terminal("kitty", Some("foot"), &none).as_deref(),
            Some("kitty")
        );
        assert_eq!(
            pick_terminal("auto", Some("foot"), &has).as_deref(),
            Some("foot")
        );
        assert_eq!(
            pick_terminal("auto", None, &has).as_deref(),
            Some("alacritty")
        );
        assert_eq!(pick_terminal("auto", Some(""), &none), None);
    }

    #[test]
    fn terminal_commands_never_interpolate() {
        let argv = vec!["archguard".to_string(), "upgrade".to_string()];
        let c = terminal_command("konsole", &argv);
        assert_eq!(c.program, "konsole");
        assert_eq!(&c.args[..3], ["-e", "sh", "-c"]);
        assert!(c.args[3].starts_with("\"$@\""));
        assert_eq!(&c.args[c.args.len() - 2..], argv.as_slice());
        let g = terminal_command("gnome-terminal", &argv);
        assert_eq!(g.args[0], "--");
        let k = terminal_command("kitty", &argv);
        assert_eq!(k.args[0], "sh");
        let unknown = terminal_command("myterm", &argv);
        assert_eq!(unknown.args[0], "-e");
        assert_eq!(open_url_command("https://x").unwrap().args, ["https://x"]);
    }

    #[test]
    fn url_validation() {
        for ok in [
            "https://archlinux.org/news/x/",
            "HTTP://example.com",
            "https://a?b#c",
        ] {
            assert!(is_web_url(ok), "{ok}");
        }
        for bad in [
            "file:///etc/shadow",
            "javascript:alert(1)",
            "--help",
            "https://",
            "https:///x",
            "https://-evil",
            "https://a b",
            "https://a\n",
            "ftp://x",
            "",
        ] {
            assert!(!is_web_url(bad), "{bad:?}");
            assert!(open_url_command(bad).is_none());
        }
    }

    #[test]
    fn backoff() {
        let d: Vec<_> = (0..8).map(|i| retry_delay(i).as_secs()).collect();
        assert_eq!(d, [1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn error_notifies_once() {
        let ok = Model::from_check(&report(&[]), vec![]);
        let err = ok.with_error("timeout");
        let n = transition_notification(&ok, &err).unwrap();
        assert!(n.summary.contains("check failed") && n.body.contains("timeout"));
        assert!(transition_notification(&err, &err.with_error("again")).is_none());
        assert!(transition_notification(&err, &ok).is_none());
    }
}
