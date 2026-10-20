//! F1 acceptance: a fixture feed mentioning an installed (fake) package makes
//! `upgrade` block without ever invoking a sync command.

use archguard::{cmd_ack, cmd_check, cmd_upgrade, Cli, Ctx, EXIT_BLOCKED, EXIT_OK};
use archguard_core::config::{Config, Helper};
use archguard_core::news::{parse_feed, StaticNews};
use archguard_core::packages::FakePackages;
use archguard_core::state::State;
use archguard_runner::MockRunner;
use clap::CommandFactory;

const FEED: &[u8] = include_bytes!("../../core/tests/fixtures/news.xml");

fn news() -> StaticNews {
    StaticNews(parse_feed(FEED).unwrap())
}

fn ctx<'a>(news: &'a StaticNews, pkgs: &'a FakePackages, helper: Helper) -> Ctx<'a> {
    Ctx {
        config: Config {
            helper,
            ..Config::default()
        },
        news,
        packages: pkgs,
        state: State::default(),
        now: 0,
        is_root: false,
    }
}

#[test]
fn upgrade_blocks_without_sync() {
    let n = news();
    let pkgs = FakePackages::new(["glibc", "foolib"]);
    for helper in [Helper::Pacman, Helper::Paru, Helper::Yay] {
        let c = ctx(&n, &pkgs, helper);
        let mut runner = MockRunner::default();
        let mut out = Vec::new();
        let code = cmd_upgrade(&c, &[], &mut runner, &mut out).unwrap();
        assert_eq!(code, EXIT_BLOCKED);
        assert!(
            runner.calls.is_empty(),
            "runner invoked: {:?}",
            runner.calls
        );
        assert!(!runner.calls.iter().any(|c| c.is_sync()));
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("BLOCKED"));
        assert!(text.contains("foolib-20-requires-manual-intervention"));
    }
}

#[test]
fn ack_then_upgrade_runs_sync_once() {
    let n = news();
    let pkgs = FakePackages::new(["foolib"]);
    let mut c = ctx(&n, &pkgs, Helper::Pacman);
    let mut sink = Vec::new();
    assert_eq!(cmd_check(&c, &mut sink).unwrap(), EXIT_BLOCKED);
    cmd_ack(
        &mut c,
        &["foolib-20-requires-manual-intervention".into()],
        false,
        &mut sink,
    )
    .unwrap();
    assert_eq!(cmd_check(&c, &mut sink).unwrap(), EXIT_OK);
    let mut runner = MockRunner::default();
    assert_eq!(cmd_upgrade(&c, &[], &mut runner, &mut sink).unwrap(), 0);
    assert_eq!(runner.calls.len(), 1);
    assert_eq!(runner.calls[0].to_string(), "sudo pacman -Syu");
}

#[test]
fn ack_all_and_unknown() {
    let n = news();
    let pkgs = FakePackages::new(["foolib", "barserver"]);
    let mut c = ctx(&n, &pkgs, Helper::Paru);
    let mut sink = Vec::new();
    assert!(cmd_ack(&mut c, &["nope".into()], false, &mut sink).is_err());
    cmd_ack(&mut c, &[], true, &mut sink).unwrap();
    assert_eq!(c.state.acked.len(), 3);
    assert_eq!(cmd_check(&c, &mut sink).unwrap(), EXIT_OK);
}

#[test]
fn aur_helper_as_root_refused_before_anything_runs() {
    let n = news();
    let pkgs = FakePackages::new(["glibc"]);
    let mut c = ctx(&n, &pkgs, Helper::Yay);
    c.is_root = true;
    let mut runner = MockRunner::default();
    assert!(cmd_upgrade(&c, &[], &mut runner, &mut Vec::new()).is_err());
    assert!(runner.calls.is_empty());
}

#[test]
fn version_matches_cargo_pkg_version() {
    let cmd = Cli::command();
    assert_eq!(cmd.get_version(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(archguard_core::version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn cli_debug_assert() {
    Cli::command().debug_assert();
}

#[test]
fn ack_rejects_empty_id() {
    let n = news();
    let pkgs = FakePackages::new(["foolib"]);
    let mut c = ctx(&n, &pkgs, Helper::Pacman);
    assert!(cmd_ack(&mut c, &["".into()], false, &mut Vec::new()).is_err());
    assert!(cmd_ack(&mut c, &["  ".into()], false, &mut Vec::new()).is_err());
    assert!(c.state.acked.is_empty());
}

/// Regression (testerdin, PR #7): items without GUID must keep a stable id so
/// an ack survives re-fetching; untitled ones must not block forever.
#[test]
fn ack_of_guidless_items_persists_across_refetch() {
    const NO_GUID: &[u8] = include_bytes!("../../core/tests/fixtures/news_noguid.xml");
    let dir = std::env::temp_dir().join(format!("ag-noguid-{}", std::process::id()));
    let state_path = dir.join("state");
    let pkgs = FakePackages::new(["foolib"]);

    // First run: blocked by the untitled item and the foolib item.
    let n1 = StaticNews(parse_feed(NO_GUID).unwrap());
    let mut c = ctx(&n1, &pkgs, Helper::Pacman);
    assert_eq!(cmd_check(&c, &mut Vec::new()).unwrap(), EXIT_BLOCKED);
    cmd_ack(&mut c, &[], true, &mut Vec::new()).unwrap();
    c.state.save(&state_path).unwrap();

    // Second run: fresh parse + state reload -> nothing blocks.
    let n2 = StaticNews(parse_feed(NO_GUID).unwrap());
    let mut c2 = ctx(&n2, &pkgs, Helper::Pacman);
    c2.state = State::load(&state_path).unwrap();
    assert_eq!(cmd_check(&c2, &mut Vec::new()).unwrap(), EXIT_OK);
    let mut runner = MockRunner::default();
    assert_eq!(
        cmd_upgrade(&c2, &[], &mut runner, &mut Vec::new()).unwrap(),
        0
    );
    assert_eq!(runner.calls.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

mod notify_and_hooks {
    use super::*;
    use archguard_core::notify::{Notification, Notifier};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Spy(RefCell<Vec<Notification>>, bool);
    impl Notifier for Spy {
        fn send(&self, n: &Notification) -> anyhow::Result<()> {
            self.0.borrow_mut().push(n.clone());
            if self.1 {
                anyhow::bail!("no session bus");
            }
            Ok(())
        }
    }

    #[test]
    fn check_notify_sends_when_blocking_only() {
        let n = news();
        let pkgs = FakePackages::new(["foolib"]);
        let c = ctx(&n, &pkgs, Helper::Pacman);
        let spy = Spy::default();
        assert_eq!(
            archguard::cmd_check_notify(&c, Some((&spy, 0)), &mut Vec::new()).unwrap(),
            EXIT_BLOCKED
        );
        assert_eq!(spy.0.borrow().len(), 1);
        assert!(spy.0.borrow()[0].critical);

        let clean = FakePackages::new(["glibc"]);
        let c = ctx(&n, &clean, Helper::Pacman);
        let spy = Spy::default();
        archguard::cmd_check_notify(&c, Some((&spy, 0)), &mut Vec::new()).unwrap();
        assert!(spy.0.borrow().is_empty());
    }

    #[test]
    fn notification_failure_is_only_a_warning() {
        let n = news();
        let pkgs = FakePackages::new(["foolib"]);
        let c = ctx(&n, &pkgs, Helper::Pacman);
        let spy = Spy(RefCell::new(vec![]), true);
        let mut out = Vec::new();
        assert_eq!(
            archguard::cmd_check_notify(&c, Some((&spy, 3)), &mut out).unwrap(),
            EXIT_BLOCKED
        );
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("warning: notification failed"));
    }

    #[test]
    fn failing_pre_upgrade_hook_blocks_without_sync() {
        let n = news();
        let pkgs = FakePackages::new(["glibc"]);
        let mut c = ctx(&n, &pkgs, Helper::Paru);
        c.config.hooks.pre_upgrade = vec!["pkgwatch check".into(), "aurscan --pre-upgrade".into()];
        let mut runner = MockRunner::default();
        runner.codes.insert("aurscan".into(), 1);
        let mut out = Vec::new();
        assert_eq!(
            cmd_upgrade(&c, &[], &mut runner, &mut out).unwrap(),
            EXIT_BLOCKED
        );
        assert!(!runner.calls.iter().any(|c| c.is_sync()));
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("aurscan --pre-upgrade"));

        let mut ok = MockRunner::default();
        assert_eq!(cmd_upgrade(&c, &[], &mut ok, &mut Vec::new()).unwrap(), 0);
        assert_eq!(ok.calls.len(), 3);
        assert_eq!(ok.calls[2].to_string(), "paru -Syu");
    }

    #[test]
    fn invalid_hook_config_errors_before_running_anything() {
        let n = news();
        let pkgs = FakePackages::new(["glibc"]);
        let mut c = ctx(&n, &pkgs, Helper::Pacman);
        c.config.hooks.pre_upgrade = vec!["".into()];
        let mut runner = MockRunner::default();
        assert!(cmd_upgrade(&c, &[], &mut runner, &mut Vec::new()).is_err());
        assert!(runner.calls.is_empty());
    }
}
