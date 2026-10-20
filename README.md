# archguard

> **ES:** CLI en Rust para Arch Linux / EndeavourOS que lee las noticias de Arch (RSS), las cruza con tus paquetes instalados y **bloquea la actualización antes de `-Sy`** si hay una noticia relevante sin leer. Es un *wrapper* (no un hook de pacman). Más adelante ayudará a fusionar archivos `.pacnew`.
>
> **EN:** Rust CLI for Arch Linux / EndeavourOS that reads the Arch news RSS feed, matches items against installed packages and **blocks the upgrade before `-Sy`** when relevant unread news exists. It is a wrapper, not a pacman hook. Later it will help merge `.pacnew` files.

**Status:** Phase 1 — `check`, `ack`, `upgrade` implemented.

## Workspace

| Crate | Role |
|---|---|
| `crates/core` | News (reqwest + feed-rs), alpm package matching, state in `~/.local/state/archguard`, `.pacnew` scan |
| `crates/runner` | Runs pacman/paru/yay; elevates only what is needed via sudo/run0 — never everything as root |
| `crates/merge` | 3-way `.pacnew` merge with `similar`, base from old pkg in `/var/cache/pacman/pkg`, meld/kdiff3 on conflict |
| `crates/cli` | `archguard` binary (clap) |

> The `alpm` crate (pinned `~5.0`, libalpm 16 / pacman 7.1) is behind the cargo feature `alpm` so non-Arch builds work; without it archguard falls back to `pacman -Qq`. Both only read the **local** DB — archguard never syncs it during `check`.

## Usage / Uso

```sh
archguard check            # exit 1 if relevant unacknowledged news exist
archguard ack <id|slug>... # or: archguard ack --all
archguard upgrade [-- extra pacman/paru/yay args]
```

`upgrade` refuses to run `-Syu` while `check` is blocking. Only `pacman` is elevated (sudo/run0); paru/yay are never run as root.

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
```

State (acked ids): `~/.local/state/archguard/state`.

## Install / Instalación

```sh
# From source
cargo install --path crates/cli --locked --features alpm

# Arch (VCS package)
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
