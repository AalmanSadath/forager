#!/bin/sh
# Builds forager-<version>-x86_64.pkg.tar.zst from the release tarball.
# On Arch (or in CI's archlinux container) it runs makepkg directly; anywhere
# else it runs makepkg in an archlinux container with podman.
#
# Usage: packaging/arch/build.sh <tarball> [output-dir]
set -eu

here=$(cd "$(dirname "$0")" && pwd)
tarball=$(realpath "$1")
out=$(realpath -m "${2:-$here/../../target/dist}")
version=$("$here/../version.sh")
sum=$(sha256sum "$tarball" | cut -d' ' -f1)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sed -e "s/@VERSION@/$version/" -e "s/@SHA256@/$sum/" "$here/PKGBUILD" > "$work/PKGBUILD"
# makepkg uses a local file of the source's name instead of downloading it.
cp "$tarball" "$work/"
mkdir -p "$out"

# -d: the runtime dependencies need not be installed to package binaries.
# makepkg refuses to run as root, so a root shell builds as `builder` and
# copies the result out itself; under rootless podman that root is the
# invoking user, so the package is theirs on the host.
makepkg_cmd='makepkg -d --noconfirm -f'
if command -v makepkg >/dev/null; then
  if [ "$(id -u)" = 0 ]; then
    id builder >/dev/null 2>&1 || useradd -m builder
    chown -R builder "$work"
    su builder -c "cd '$work' && $makepkg_cmd"
  else
    (cd "$work" && $makepkg_cmd)
  fi
  cp "$work"/*.pkg.tar.zst "$out/"
else
  podman run --rm -v "$work:/work:Z" -v "$out:/out:Z" docker.io/library/archlinux:latest sh -c "
    pacman -Sy --noconfirm --needed base-devel >/dev/null &&
    useradd -m builder && cp -r /work /build && chown -R builder /build &&
    su builder -c 'cd /build && $makepkg_cmd' &&
    cp /build/*.pkg.tar.zst /out/"
fi
# makepkg names it with the package release (-1-); the release files leave
# that out, as the rpm's name does.
mv "$out/forager-$version-1-x86_64.pkg.tar.zst" "$out/forager-$version-x86_64.pkg.tar.zst"
ls "$out/forager-$version-x86_64.pkg.tar.zst"
