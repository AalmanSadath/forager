#!/usr/bin/env bash
#
# Regenerates the offline sources the Flatpak builds from:
#   cargo-sources.json  from Cargo.lock           (flatpak-cargo-generator.py)
#   node-sources.json   from gui/package-lock.json (flatpak-node-generator)
# Run it after changing either lockfile. Both checks at the end are the ones
# CI runs, so a stale manifest is caught here.
#
# flatpak-node-generator is run on a copy of the two manifests alone: in a
# working tree with node_modules installed it silently treats installed
# packages as local and leaves their tarballs out (see BifroSSH's
# flatpak/regen-node-sources.sh for the details).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)

if ! command -v flatpak-node-generator >/dev/null; then
  echo 'flatpak-node-generator not found; install it with:' >&2
  echo '  pipx install "git+https://github.com/flatpak/flatpak-builder-tools#subdirectory=node"' >&2
  exit 1
fi

python3 "$here/flatpak-cargo-generator.py" "$root/Cargo.lock" -o "$here/cargo-sources.json"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$root/gui/package.json" "$root/gui/package-lock.json" "$work/"
(cd "$work" && flatpak-node-generator npm package-lock.json -o sources.json)
cp "$work/sources.json" "$here/node-sources.json"

"$here/check-sources.py"
