#!/bin/sh
# Builds every package of Forager into target/dist, with SHA256SUMS:
#
#   forager_<v>_amd64.deb             Debian, Ubuntu
#   forager-<v>-1.x86_64.rpm          Fedora, openSUSE
#   forager-<v>-1-x86_64.pkg.tar.zst  Arch
#   forager-<v>-x86_64.flatpak        any distribution with Flatpak
#   Forager-<v>-x86_64.AppImage       any distribution
#   forager-<v>-x86_64-linux.tar.gz   any distribution, unpacked by hand
#
# The names are what Forager's own asset matcher expects, so Forager can
# track and update itself in whichever format it was installed.
#
# SKIP_FLATPAK=1 leaves the Flatpak out (it takes longest).
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
version=$("$root/packaging/version.sh")
out=$root/target/dist
rm -rf "$out"
mkdir -p "$out"

cd "$root/gui"
npm ci --no-audit --no-fund
npx tauri build --bundles deb,rpm,appimage
bundle=$root/target/release/bundle
cp "$bundle/deb/Forager_${version}_amd64.deb" "$out/forager_${version}_amd64.deb"
cp "$bundle/rpm/Forager-${version}-1.x86_64.rpm" "$out/forager-${version}-1.x86_64.rpm"
cp "$bundle/appimage/Forager_${version}_amd64.AppImage" "$out/Forager-${version}-x86_64.AppImage"

"$root/packaging/tarball.sh" "$out"
"$root/packaging/arch/build.sh" "$out/forager-$version-x86_64-linux.tar.gz" "$out"
[ -n "${SKIP_FLATPAK:-}" ] || "$root/packaging/flatpak/build.sh" "$out"

cd "$out"
sha256sum -- * > SHA256SUMS
ls -l
