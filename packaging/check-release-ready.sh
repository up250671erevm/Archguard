#!/usr/bin/env bash
# Guard before publishing packaging/archguard to the AUR.
# Fails if the checksum is still the all-zero placeholder (or SKIP), if
# PKGBUILD/.SRCINFO/Cargo.toml disagree, or (unless --offline) if the sha256
# doesn't match the real GitHub tag tarball.
#
#   packaging/check-release-ready.sh 0.1.0 [--offline]
set -euo pipefail

ver="${1:?usage: $0 <version> [--offline]}"
offline="${2:-}"
if [[ ! "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: version must look like 1.2.3 (no leading v), got: $ver" >&2
  exit 2
fi
root="$(cd "$(dirname "$0")/.." && pwd)"
pb="$root/packaging/archguard/PKGBUILD"
si="$root/packaging/archguard/.SRCINFO"
fail() { echo "NOT READY: $*" >&2; exit 1; }

pkgver="$(sed -n 's/^pkgver=//p' "$pb")"
sum="$(sed -n "s/^sha256sums=('\(.*\)')$/\1/p" "$pb")"
cargo_ver="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"

[[ "$pkgver" == "$ver" ]] || fail "PKGBUILD pkgver=$pkgver, expected $ver"
[[ "$cargo_ver" == "$ver" ]] || fail "Cargo.toml version=$cargo_ver, expected $ver"
[[ "$sum" =~ ^[0-9a-f]{64}$ ]] || fail "sha256sums is not a single sha256: '$sum'"
[[ "$sum" =~ ^0{64}$ ]] && fail "sha256sums is still the all-zero placeholder; run packaging/update-release-checksums.sh $ver"
grep -qx $'\tpkgver = '"$ver" "$si" || fail ".SRCINFO pkgver differs"
grep -qx $'\tsha256sums = '"$sum" "$si" || fail ".SRCINFO sha256sums differs from PKGBUILD"
grep -q "^## \[$ver\]" "$root/CHANGELOG.md" || fail "CHANGELOG.md has no [$ver] section"
grep -Eq "^## \[$ver\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" "$root/CHANGELOG.md" || fail "CHANGELOG.md [$ver] section has no release date (YYYY-MM-DD)"

if [[ "$offline" != "--offline" ]]; then
  url="https://github.com/up250671erevm/Archguard/archive/refs/tags/v${ver}.tar.gz"
  tmp="$(mktemp)"; trap 'rm -f "$tmp"' EXIT
  curl -fsSL --retry 3 -o "$tmp" "$url" || fail "tag tarball not found: $url (is tag v$ver pushed?)"
  real="$(sha256sum "$tmp" | cut -d' ' -f1)"
  [[ "$real" == "$sum" ]] || fail "sha256 of $url is $real, PKGBUILD has $sum"
fi
echo "release $ver: ready to publish"
