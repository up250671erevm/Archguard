# Releasing archguard

Checklist for EMIVICMAN. Example uses `0.1.0`; replace as needed.

## 1. Prepare (in a PR)

- [ ] Workspace version in `Cargo.toml` is `0.1.0`; `cargo update -w` so `Cargo.lock` matches.
- [ ] `CHANGELOG.md`: move items from *Unreleased* to `## [0.1.0] - YYYY-MM-DD`.
- [ ] `packaging/archguard/PKGBUILD`: `pkgver=0.1.0`, `pkgrel=1`.
- [ ] CI green (fmt, clippy, tests, MSRV, Arch, makepkg + namcap, release + clean chroot).
- [ ] PR approved and merged to `main`.

## 2. Tag (on up-to-date `main`)

```sh
git switch main && git pull --ff-only
git tag -s v0.1.0 -m "archguard 0.1.0"     # -a instead of -s if you don't sign
git push origin v0.1.0
```

Then create the GitHub release from the tag (paste the CHANGELOG section):

```sh
gh release create v0.1.0 --title "archguard 0.1.0" --notes-file <(sed -n '/^## \[0.1.0\]/,/^## \[/p' CHANGELOG.md | sed '$d')
```

## 3. Fill in the tarball checksum

The release PKGBUILD ships a zero placeholder checksum that **fails on purpose**
until this step is done. GitHub generates the tag tarball; its hash is only
known after the tag exists:

```sh
packaging/update-release-checksums.sh 0.1.0
```

The script checks that the `Cargo.toml` version matches, downloads
`https://github.com/up250671erevm/Archguard/archive/refs/tags/v0.1.0.tar.gz`,
checks that it contains `Archguard-0.1.0/Cargo.toml` with that version, writes
`sha256sums` into `packaging/archguard/PKGBUILD` and regenerates `.SRCINFO`
(with `makepkg --printsrcinfo` on Arch; otherwise it patches the file and tells
you to verify it).

Equivalent by hand on Arch: `cd packaging/archguard && updpkgsums && makepkg --printsrcinfo > .SRCINFO`.

Commit to `main` through a PR: `packaging: v0.1.0 checksums`.

## 4. Verify on Arch

```sh
cd packaging/archguard
namcap PKGBUILD
extra-x86_64-build            # clean chroot build (devtools)
namcap archguard-0.1.0-1-x86_64.pkg.tar.zst
```

## 5. Publish to the AUR

One-time: add your SSH key to your AUR account. Then for each package:

```sh
git clone ssh://aur@aur.archlinux.org/archguard.git aur-archguard
cp packaging/archguard/{PKGBUILD,.SRCINFO} aur-archguard/
cd aur-archguard
git add PKGBUILD .SRCINFO
git commit -m "Update to 0.1.0"
git push
```

`archguard-git` the same way (`ssh://aur@aur.archlinux.org/archguard-git.git`,
files from `packaging/archguard-git/`). Its `.SRCINFO` only needs regenerating
when the PKGBUILD changes (`makepkg --printsrcinfo > .SRCINFO`).

Notes:
- AUR repos must contain only flat files (no subdirectories); `.SRCINFO` must match the PKGBUILD.
- No `.install` file is used: users enable the timer themselves
  (`systemctl --user enable --now archguard-check.timer`) and the tray autostart
  (`archguard-tray --autostart enable`).

## 6. After release

- [ ] Add a new `## [Unreleased]` section to `CHANGELOG.md`.
- [ ] Bump `pkgrel` (not `pkgver`) for packaging-only fixes.
