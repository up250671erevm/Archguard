#!/usr/bin/env bash
# Clean-chroot build of the release PKGBUILD with devtools on a GitHub
# ubuntu runner.
#
# devtools' extra-x86_64-build uses systemd-nspawn, which needs a real
# systemd (PID 1) + machined to allocate a scope. Docker containers have
# neither, so we bootstrap an Arch rootfs on the VM, chroot into it with the
# host's /run bind-mounted (giving nspawn the host system bus), and run
# devtools from there.
#
# Usage: sudo ci/clean-chroot-build.sh <dir with PKGBUILD + source tarball>
set -euo pipefail

src="$(realpath "${1:?pkg dir}")"
root=/var/tmp/archroot
mirror='https://geo.mirror.pkgbuild.com'

apt-get update -qq
apt-get install -y -qq systemd-container zstd >/dev/null
systemctl start systemd-machined || true

if [[ ! -d $root ]]; then
  mkdir -p /var/tmp/boot
  curl -fsSL --retry 3 "$mirror/iso/latest/archlinux-bootstrap-x86_64.tar.zst" -o /var/tmp/boot/bootstrap.tar.zst
  tar --zstd -xf /var/tmp/boot/bootstrap.tar.zst -C /var/tmp/boot --numeric-owner
  mv /var/tmp/boot/root.x86_64 "$root"
fi
echo "Server = $mirror/\$repo/os/\$arch" > "$root/etc/pacman.d/mirrorlist"
cp -L /etc/resolv.conf "$root/etc/resolv.conf"
test -s /etc/machine-id && cp /etc/machine-id "$root/etc/machine-id"

mounts=()
m() { mount "$@"; mounts+=("${@: -1}"); }
cleanup() { for ((i=${#mounts[@]}-1; i>=0; i--)); do umount -R "${mounts[i]}" 2>/dev/null || umount -l "${mounts[i]}" || true; done; }
trap cleanup EXIT
m --bind "$root" "$root"            # pacman wants / to be a mountpoint
m -t proc proc "$root/proc"
m --rbind /sys "$root/sys"; mount --make-rslave "$root/sys"
m --rbind /dev "$root/dev"; mount --make-rslave "$root/dev"
m --rbind /run "$root/run"; mount --make-rslave "$root/run"   # host system bus for nspawn
m -t tmpfs tmpfs "$root/tmp"

mkdir -p "$root/build"
cp -a "$src/." "$root/build/"

chroot "$root" /bin/bash -euo pipefail -c '
  pacman-key --init >/dev/null
  pacman-key --populate archlinux >/dev/null
  pacman -Syu --noconfirm --needed base-devel devtools namcap sudo git >/dev/null
  id builder >/dev/null 2>&1 || useradd -m builder
  echo "builder ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/builder
  chown -R builder: /build
  cd /build
  rm -f ./*.pkg.tar.zst
  sudo -u builder extra-x86_64-build -- -- --noconfirm
  ls -l ./*.pkg.tar.zst
  namcap ./*.pkg.tar.zst
  pacman -Qip ./*.pkg.tar.zst | grep -E "^(Name|Version|Conflicts With|Depends On|Build Date)"
'
mkdir -p "$src/out"
cp "$root"/build/*.pkg.tar.zst "$src/out/"
