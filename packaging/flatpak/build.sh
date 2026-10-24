#!/usr/bin/env bash
# Builds the Flatpak from the committed tree and exports
# forager-<version>-<arch>.flatpak, a single-file bundle that installs with
#   flatpak install --user forager-<version>-<arch>.flatpak
# and fetches the GNOME runtime from Flathub if it is missing.
#
# Usage: packaging/flatpak/build.sh [output-dir]     (default target/dist)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
version=$("$root/packaging/version.sh")
out=$(realpath -m "${1:-$root/target/dist}")
id=io.github.aalmansadath.Forager
state=$root/target/flatpak

"$here/check-sources.py"
mkdir -p "$out" "$state"
# TERM=dumb keeps flatpak-builder from sending terminal queries that leak
# into the output.
TERM=dumb flatpak-builder --user --force-clean --disable-rofiles-fuse \
  --install-deps-from=flathub \
  --state-dir="$state/state" --repo="$state/repo" \
  "$state/build" "$here/$id.yml"
flatpak build-bundle \
  --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
  "$state/repo" "$out/forager-$version-$(uname -m).flatpak" "$id"
echo "$out/forager-$version-$(uname -m).flatpak"
