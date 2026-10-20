#!/usr/bin/env bash
# Fill in the real sha256 of the GitHub tag tarball for the release PKGBUILD
# and regenerate its .SRCINFO. Run AFTER pushing tag v<version>.
#
#   packaging/update-release-checksums.sh 0.1.0
set -euo pipefail

ver="${1:?usage: $0 <version>   (e.g. 0.1.0)}"
root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/packaging/archguard"
url="https://github.com/up250671erevm/Archguard/archive/refs/tags/v${ver}.tar.gz"

cargo_ver="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
if [[ "$cargo_ver" != "$ver" ]]; then
  echo "error: Cargo.toml workspace version is $cargo_ver, not $ver" >&2
  exit 1
fi

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
echo "downloading $url"
curl -fsSL --retry 3 "$url" -o "$tmp"
# Sanity: the tarball must contain the expected top-level dir and version.
tar -xOzf "$tmp" "Archguard-${ver}/Cargo.toml" | grep -q "^version = \"${ver}\"" \
  || { echo "error: tarball does not contain Archguard-${ver}/Cargo.toml with version ${ver}" >&2; exit 1; }
sum="$(sha256sum "$tmp" | cut -d' ' -f1)"
echo "sha256: $sum"

sed -i -e "s/^pkgver=.*/pkgver=${ver}/" \
       -e "s/^pkgrel=.*/pkgrel=1/" \
       -e "s/^sha256sums=.*/sha256sums=('${sum}')/" "$dir/PKGBUILD"

if command -v makepkg >/dev/null 2>&1; then
  (cd "$dir" && makepkg --printsrcinfo > .SRCINFO)
else
  echo "makepkg not found; patching .SRCINFO with sed (verify on Arch!)"
  sed -i -e "s/^\tpkgver = .*/\tpkgver = ${ver}/" \
         -e "s/^\tpkgrel = .*/\tpkgrel = 1/" \
         -e "s#^\tsource = .*#\tsource = archguard-${ver}.tar.gz::${url}#" \
         -e "s/^\tsha256sums = .*/\tsha256sums = ${sum}/" "$dir/.SRCINFO"
fi
echo "updated $dir/PKGBUILD and $dir/.SRCINFO"
git -C "$root" --no-pager diff --stat -- packaging/archguard
