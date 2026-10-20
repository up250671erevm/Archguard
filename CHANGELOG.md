# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - unreleased (tag pending)

First public release.

### Added
- `archguard check [--notify]`: reads the Arch news RSS feed, matches items
  against installed packages (libalpm, local DB only — never syncs) and exits 1
  if relevant unacknowledged news exist. Feed downloads are capped at 4 MiB;
  untitled items block (fail-closed).
- Items without a source GUID get a deterministic id `archguard:<sha256>` of
  link + title + publication date (or title + date + summary without a link).
  Two items sharing a link therefore never share an ack; editing the title or
  date of such an item makes it block again (fail-closed).
- `archguard ack <id|slug>... | --all` with durable (fsync) and locked state in
  `~/.local/state/archguard/state` (the feed is fetched before the lock is taken).
- `archguard upgrade [-- args]`: refuses `-Syu` while news block; runs pacman
  (via sudo/run0) or paru/yay (never as root); `[hooks] pre_upgrade` commands
  with a per-hook timeout (`timeout_secs`, default 300) — failure or timeout
  blocks. Hooks run non-interactively (stdin closed) in their own process
  group; on timeout the whole group is killed (SIGTERM, then SIGKILL).
- `archguard pacnew list|diff|merge`: 3-way merge of `.pacnew` files using the
  previous package version from the pacman cache as base (follows links inside
  the archive), meld/kdiff3 on conflict, writes through symlinks, keeps
  backups, elevates only the final cp/install/mv/rm.
- `archguard-tray`: StatusNotifierItem applet (ksni) with notifications
  (notify-rust): status, check now, read/ack news, review .pacnew, run upgrade in
  a terminal. Waits for a StatusNotifierWatcher and retries the session bus with
  backoff; opens only http(s) links; `--autostart enable|disable`.
- systemd user units `archguard-check.service` / `.timer` (5 min after boot,
  daily, persistent).
- Config file `~/.config/archguard.toml` (helper, elevation, feed, matching,
  pacnew, hooks, tray).
- Packaging: `archguard-git` and release `archguard` PKGBUILDs with `.SRCINFO`;
  CI builds both, runs namcap, verifies the user units with
  `systemd-analyze --user verify`, and builds the release package in a clean chroot.

[Unreleased]: https://github.com/up250671erevm/Archguard/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/up250671erevm/Archguard/releases/tag/v0.1.0
