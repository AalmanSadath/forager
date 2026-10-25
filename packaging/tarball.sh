#!/bin/sh
# Builds forager-<version>-<arch>-linux.tar.gz from the release binaries in
# target/release: both programs, the desktop entry, icons, AppStream data,
# shell completions and man pages, and
# an install.sh that copies them into ~/.local (or another prefix).
#
# Usage: packaging/tarball.sh [output-dir]     (default target/dist)
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
version=$("$root/packaging/version.sh")
arch=$(uname -m)
out=${1:-$root/target/dist}
name=forager-$version-$arch-linux
id=io.github.aalmansadath.Forager

for bin in forager forager-gui; do
  [ -x "$root/target/release/$bin" ] || { echo "target/release/$bin is missing; build it first" >&2; exit 1; }
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
dir=$work/$name
mkdir -p "$dir/bin" "$dir/share/applications" "$dir/share/metainfo"
install -m755 "$root/target/release/forager" "$root/target/release/forager-gui" "$dir/bin/"
install -m644 "$root/packaging/$id.desktop" "$dir/share/applications/"
install -m644 "$root/packaging/$id.metainfo.xml" "$dir/share/metainfo/"
for size in 32 64 128; do
  install -Dm644 "$root/gui/src-tauri/icons/${size}x${size}.png" "$dir/share/icons/hicolor/${size}x${size}/apps/$id.png"
done
install -Dm644 "$root/gui/src-tauri/icons/128x128@2x.png" "$dir/share/icons/hicolor/256x256/apps/$id.png"
install -Dm644 "$root/gui/src-tauri/icons/icon.svg" "$dir/share/icons/hicolor/scalable/apps/$id.svg"
"$root/packaging/gen-docs.sh" "$root/target/release/forager" "$work/docs"
cp -R "$work/docs/." "$dir/share/"
install -m644 "$root/README.md" "$root/LICENSE" "$dir/"

cat > "$dir/install.sh" <<'INSTALL'
#!/bin/sh
# Installs Forager from this directory into a prefix, ~/.local by default:
#   ./install.sh [prefix]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
prefix=${1:-$HOME/.local}
mkdir -p "$prefix"
cp -R "$here/bin" "$here/share" "$prefix/"
echo "Installed forager and forager-gui into $prefix/bin."
case ":$PATH:" in *":$prefix/bin:"*) ;; *) echo "Note: $prefix/bin is not on PATH." ;; esac
INSTALL
chmod 755 "$dir/install.sh"

mkdir -p "$out"
tar -C "$work" --owner=0 --group=0 -czf "$out/$name.tar.gz" "$name"
echo "$out/$name.tar.gz"
