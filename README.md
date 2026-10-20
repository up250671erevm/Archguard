# archguard

> **ES:** CLI en Rust para Arch Linux / EndeavourOS que lee las noticias de Arch (RSS), las cruza con tus paquetes instalados y **bloquea la actualización antes de `-Sy`** si hay una noticia relevante sin leer. Es un *wrapper* (no un hook de pacman). Más adelante ayudará a fusionar archivos `.pacnew`.
>
> **EN:** Rust CLI for Arch Linux / EndeavourOS that reads the Arch news RSS feed, matches items against installed packages and **blocks the upgrade before `-Sy`** when relevant unread news exists. It is a wrapper, not a pacman hook. Later it will help merge `.pacnew` files.

**Status:** Phase 0 (skeleton). Nothing functional yet beyond `archguard --version`.

## Workspace

| Crate | Role |
|---|---|
| `crates/core` | News (reqwest + feed-rs), alpm package matching, state in `~/.local/state/archguard`, `.pacnew` scan |
| `crates/runner` | Runs pacman/paru/yay; elevates only what is needed via sudo/run0 — never everything as root |
| `crates/merge` | 3-way `.pacnew` merge with `similar`, base from old pkg in `/var/cache/pacman/pkg`, meld/kdiff3 on conflict |
| `crates/cli` | `archguard` binary (clap) |

> Note: the `alpm` crate dependency is intentionally **not** added yet — it requires libalpm (Arch only) and would break builds on non-Arch CI runners. It will be introduced in F1, built in the `archlinux:latest` CI job.

## Install / Instalación

```sh
# From source
cargo install --path crates/cli --locked

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
