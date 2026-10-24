# Forager

Install and update desktop Linux apps straight from their GitHub releases,
using the system package manager. Think [Obtainium](https://github.com/ImranR98/Obtainium),
but for the desktop.

## Status

Early prototype. CLI only.

- Sources: GitHub releases
- Formats: `.rpm` (dnf, zypper), `.deb` (apt), AppImage (`~/Applications`)
- Verification: SHA-256 from GitHub's asset digest or a published
  `SHA256SUMS` / `checksums.txt` / `<asset>.sha256` file

## Usage

```sh
cargo build --release
forager add https://github.com/jgraph/drawio-desktop
forager check            # show installed vs latest and the asset that would be used
forager update           # install or upgrade every tracked app (asks first)
forager update drawio-desktop --yes
forager remove drawio-desktop
```

`add` options: `--id`, `--filter <regex>` to restrict asset names,
`--format deb|rpm|appimage` to override format selection, `--prerelease`.

Tracked apps live in `~/.config/forager/apps.toml` (override with `--config`
or `FORAGER_CONFIG`). Set `GITHUB_TOKEN` to raise the GitHub API limit from
60 to 5000 requests per hour.

## How an asset is chosen

1. Formats the system can install, in order: native package format first
   (rpm on dnf/zypper systems, deb on apt systems), then AppImage.
2. Assets naming this CPU architecture beat assets naming none; assets
   naming another architecture are skipped.
3. Debug and source packages are skipped.

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

- Tarball backend (`~/.local/opt` + symlink into `~/.local/bin`)
- Flatpak bundles, Arch packages (`pacman -U`)
- GitLab, Codeberg/Forgejo and generic HTML sources
- PackageKit over D-Bus instead of `pkexec` + native tools
- Read the installed version from the package manager, detect conflicts
  with distro repositories
- Signature verification (GPG, minisign, GitHub attestations)
- systemd user timer for background checks, desktop notifications
- GTK4 / libadwaita GUI
