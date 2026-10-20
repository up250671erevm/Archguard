# archguard

> **ES:** CLI en Rust para Arch Linux / EndeavourOS que lee las noticias de Arch (RSS), las cruza con tus paquetes instalados y **bloquea la actualización antes de `-Sy`** si hay una noticia relevante sin leer. Es un *wrapper* (no un hook de pacman). Más adelante ayudará a fusionar archivos `.pacnew`.
>
> **EN:** Rust CLI for Arch Linux / EndeavourOS that reads the Arch news RSS feed, matches items against installed packages and **blocks the upgrade before `-Sy`** when relevant unread news exists. It is a wrapper, not a pacman hook. Later it will help merge `.pacnew` files.

**Status:** v0.1.1 — `check [--notify]`, `ack`, `upgrade` (+ hooks), `pacnew`, tray applet, systemd user timer. See [CHANGELOG.md](CHANGELOG.md) and [RELEASE.md](RELEASE.md).

## Workspace

| Crate | Role |
|---|---|
| `crates/core` | News (reqwest + feed-rs), alpm package matching, state in `~/.local/state/archguard`, `.pacnew` scan |
| `crates/runner` | Runs pacman/paru/yay; elevates only what is needed via sudo/run0 — never everything as root |
| `crates/merge` | 3-way `.pacnew` merge with `similar`, base from old pkg in `/var/cache/pacman/pkg`, meld/kdiff3 on conflict |
| `crates/cli` | `archguard` binary (clap) |
| `crates/tray` | `archguard-tray` (feature `gui`: ksni + notify-rust); logic is D-Bus-free and unit-tested |

> The `alpm` crate (pinned `~5.0`, libalpm 16 / pacman 7.1) is behind the cargo feature `alpm` so non-Arch builds work; without it archguard falls back to `pacman -Qq`. Both only read the **local** DB — archguard never syncs it during `check`.

## Usage / Uso

```sh
archguard check            # exit 1 if relevant unacknowledged news exist
archguard ack <id|slug>... # or: archguard ack --all
archguard upgrade [-- extra pacman/paru/yay args]
```

### .pacnew

```sh
archguard pacnew list            # .pacnew/.pacsave under scan_dirs (default /etc)
archguard pacnew diff /etc/pacman.conf
archguard pacnew merge /etc/pacman.conf [--yes]
```

`merge` does a 3-way merge (base = file from the previous package version in `/var/cache/pacman/pkg`, ours = live file, theirs = `.pacnew`). A clean merge shows the diff and, after confirmation, writes the result (backup `*.archguard-bak.<time>`) and removes the `.pacnew`. On conflicts, or when no base is cached, meld or kdiff3 is launched; without them conflict markers are left in a private temp file. Only the final cp/install/mv/rm steps are elevated (sudo/run0).

### Tray, notifications and timer

```sh
archguard check --notify                      # desktop notification if something needs attention
systemctl --user enable --now archguard-check.timer   # daily + 5 min after boot, Persistent=true
archguard-tray &                              # StatusNotifierItem (KDE, GNOME + AppIndicator, ...)
```

The tray shows OK / blocking news / pending .pacnew, and offers *Check now*, *Open Arch news*, per-item *Read*/*Acknowledge*, *Acknowledge all shown*, and *Run upgrade in terminal* (`archguard upgrade`, so the same guard applies). To autostart it: `archguard-tray --autostart enable` (writes `~/.config/autostart/archguard-tray.desktop`; `disable` removes it). If the panel isn't up yet at login the tray waits for it; if the session bus isn't reachable it retries with backoff for 5 minutes (`--no-retry` to fail fast).

Notifications use notify-rust when built with feature `archguard/notify` (the PKGBUILD does), else `notify-send`. A failed notification never changes the exit code. The service treats exit 1 ("blocking news") as success.

### Pre-upgrade hooks (pkgwatch / aurscan / anything)

```toml
[hooks]
pre_upgrade = ["pkgwatch check", "aurscan --pre-upgrade"]
timeout_secs = 300   # per hook; on timeout its whole process group is killed (SIGTERM, then SIGKILL) and the upgrade blocks
```

Run non-interactively (stdin closed, own process group), in order, as your user (never elevated), without a shell (quoting like `sh`, but `;`, `|`, `$()` are literal), only after the news check passed and before `-Syu`. Any non-zero exit — or a hook that can't be started — blocks the upgrade (fail-closed).

`upgrade` refuses to run `-Syu` while `check` is blocking. Only `pacman` is elevated (sudo/run0); paru/yay are never run as root.

**News matching.** Titles (and bodies with `match_summary = true`) are matched
against installed package names. To avoid false positives, common English
words that are also package names (`which`, `less`, `time`, `file`, `go`, ...)
only match with a strong signal: `code` markup, package context ("the X
package", "package X") or a version ("X-1.2", "X 1.2", "X >= 1.2"), and
"Arch Linux" never matches `linux`. This means a news item that mentions such a
package only as a bare word no longer blocks. To be stricter, set
`always_block = true` (every unacknowledged item blocks); to be looser, put a
name in `ignore`; add your own ambiguous words to `weak_words`.
General news that name no installed package (e.g. "Arch Linux 2026 Leader
Election Results") don't block either; set `always_block = true` to see them all.

Config `~/.config/archguard.toml` (all optional):

```toml
helper = "paru"        # pacman | paru | yay
elevation = "sudo"     # sudo | run0
feed_url = "https://archlinux.org/feeds/news/"
[matching]
match_summary = false
min_name_len = 2
ignore = []
always_block = false
max_age_days = 0
weak_words = []        # extra words that need `code`, "the X package" or a version to match
phrases = []           # extra phrases that never name a package ("Arch Linux" is built in)
[pacnew]
merge_tool = "auto"    # auto | meld | kdiff3 | none
scan_dirs = ["/etc"]
cache_dir = "/var/cache/pacman/pkg"
[hooks]
pre_upgrade = []
timeout_secs = 300
[tray]
terminal = "auto"      # or konsole, gnome-terminal, kitty, alacritty, foot, xterm, ...
interval_minutes = 360
```

State (acked ids): `~/.local/state/archguard/state`.

## Install / Instalación

```sh
# From source
cargo install --path crates/cli --locked --features alpm,notify
cargo install --path crates/tray --locked --features gui,alpm

# Arch: release package (after the v0.1.0 tag) or VCS package
cd packaging/archguard && makepkg -si
cd packaging/archguard-git && makepkg -si
```

## Roadmap

- **F0** – Workspace skeleton, CI, PKGBUILD (-git), license, docs.
- **F1** – `check` / `ack` / `upgrade`: block upgrades on relevant unread news, *without syncing the DB*.
- **F2** – `.pacnew` 3-way merge.
- **F3** – Tray icon (ksni + notify-rust), systemd user timer, optional pkgwatch/aurscan hook.
- **F4** – v0.1 release, `archguard` PKGBUILD (tarball + sha256), namcap, clean chroot build.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
