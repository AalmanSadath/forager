# Forager

Install and update desktop Linux apps straight from their GitHub releases,
using the system package manager. Think [Obtainium](https://github.com/ImranR98/Obtainium),
but for the desktop.

## Status

Early prototype. CLI only.

- Sources: GitHub releases
- Formats: `.rpm` (dnf, zypper), `.deb` (apt), AppImage (`~/Applications`),
  tarballs (`~/.local/opt/<id>`, executables linked into `~/.local/bin`),
  Flatpak bundles (`flatpak install --user`)
- Verification: SHA-256 from GitHub's asset digest or a published
  `SHA256SUMS` / `checksums.txt` / `<asset>.sha256` file

## Usage

```sh
cargo build --release
forager add https://github.com/jgraph/drawio-desktop
forager check            # show installed vs latest and the asset that would be used
forager update           # install or upgrade every tracked app (asks first)
forager update drawio-desktop --yes
forager edit drawio-desktop --format appimage   # or --format auto, --filter, --bin, --prerelease
forager remove drawio-desktop
```

`edit` changes an app's settings; a new format is installed (and the old
install removed) on the next `update`.

`add` options: `--id`, `--filter <regex>` to restrict asset names,
`--format deb|rpm|appimage|tarball|flatpak` to override format selection,
`--prerelease`, `--bin <path>` (repeatable) to name the executables to link
from a tarball, relative to its top directory.

Tracked apps live in `~/.config/forager/apps.toml` (override with `--config`
or `FORAGER_CONFIG`). Set `GITHUB_TOKEN` to raise the GitHub API limit from
60 to 5000 requests per hour. Release lists are cached in
`~/.cache/forager/github` and fetched conditionally; with a token, checking
an app whose releases have not changed does not use up the limit.

Only one Forager process works on a config at a time; a second one waits.
Installs removed outside Forager (`dnf remove`, a deleted AppImage) show as
`missing` in `check` and are reinstalled by `update`. Switching an app to
another format removes the old install once the new one is in place.

## How an asset is chosen

1. Formats the system can install, in order: native package format first
   (rpm on dnf/zypper systems, deb on apt systems), then Flatpak if
   `flatpak` is installed, then AppImage, then tarball.
2. Assets naming this CPU architecture beat assets naming none; assets
   naming another architecture or operating system are skipped.
3. Debug and source packages are skipped, as are tarballs naming neither
   Linux nor this architecture (usually source archives).

A tarball's single top-level directory is unwrapped. Without `--bin`,
Forager links the one executable in that directory or its `bin/`; if there
are several, the one named after the app id. Files in `~/.local/bin` that
Forager did not create are never replaced.

## Security

Installing a `.deb` or `.rpm` runs the package's scripts as root, so it
means trusting the upstream project and its GitHub account completely.
Forager shows the asset and whether it can be verified before installing,
and with `--yes` it refuses unverified assets unless `--allow-unverified`
is given. A checksum published next to the asset only guards against
corrupted downloads, not a compromised release.

## Layout

- `crates/forager-core`: sources, asset matching, version comparison,
  verification, installation, config store
- `crates/forager-cli`: the `forager` command

## Roadmap

- Arch packages (`pacman -U`)
- GitLab, Codeberg/Forgejo and generic HTML sources
- PackageKit over D-Bus instead of `pkexec` + native tools
- Detect conflicts with distro repositories
- Signature verification (GPG, minisign, GitHub attestations)
- systemd user timer for background checks, desktop notifications
- GTK4 / libadwaita GUI
