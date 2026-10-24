# Forager

Install and update desktop Linux apps straight from their GitHub releases,
using the system package manager. Think [Obtainium](https://github.com/ImranR98/Obtainium),
but for the desktop.

## Status

Early prototype. CLI only.

- Sources: GitHub, GitLab (gitlab.com, GNOME, freedesktop, KDE, Debian Salsa,
  Framagit, or self-hosted) and Forgejo/Gitea (Codeberg or self-hosted) releases
- Formats: `.rpm` (dnf, zypper), `.deb` (apt), Arch packages
  (`.pkg.tar.*`, `.pacman`; pacman), AppImage (`~/Applications`),
  tarballs (`~/.local/opt/<id>`, executables linked into `~/.local/bin`),
  Flatpak bundles (`flatpak install --user`)
- Verification: SHA-256 from GitHub's asset digest or a published
  `SHA256SUMS` / `checksums.txt` / `<asset>.sha256` file

## Usage

```sh
cargo build --release
forager add https://github.com/jgraph/drawio-desktop
forager add https://gitlab.com/gitlab-org/cli --id glab
forager add https://git.example.org/team/tool --forge forgejo   # self-hosted
forager check            # show installed vs latest and the asset that would be used
forager update           # install or upgrade every tracked app (asks first)
forager update drawio-desktop --yes
forager edit drawio-desktop --format appimage   # or --format auto, --filter, --bin, --prerelease
forager remove drawio-desktop
```

Background checks run from a systemd user timer:

```sh
forager timer enable                 # daily `check --notify`: a desktop notification lists updates
forager timer enable --auto-update   # also installs updates that need no root (AppImage, tarball, Flatpak)
forager timer status
forager timer disable
```

The timer reads `GITHUB_TOKEN` and other variables from `~/.config/forager/env`.
`update --unprivileged-only` skips anything that needs root, so the timer
never asks for a password.

`edit` changes an app's settings; a new format is installed (and the old
install removed) on the next `update`.

`add` options: `--id`, `--forge gitlab|forgejo` for hosts Forager does not
recognise, `--filter <regex>` to restrict asset names,
`--format deb|rpm|pacman|appimage|tarball|flatpak` to override format selection,
`--prerelease`, `--bin <path>` (repeatable) to name the executables to link
from a tarball, relative to its top directory.

Tracked apps live in `~/.config/forager/apps.toml` (override with `--config`
or `FORAGER_CONFIG`). Set `GITHUB_TOKEN` to raise the GitHub API limit from
60 to 5000 requests per hour. Release lists are cached in `~/.cache/forager`
and fetched conditionally; with a GitHub token, checking an app whose
releases have not changed does not use up the limit. `GITLAB_TOKEN` is used
for gitlab.com and `CODEBERG_TOKEN` for codeberg.org. For any host,
`FORAGER_TOKEN_<HOST>` (e.g. `FORAGER_TOKEN_GIT_EXAMPLE_ORG`) takes
precedence; a token is only ever sent to its own host.

Only one Forager process works on a config at a time; a second one waits.
Installs removed outside Forager (`dnf remove`, a deleted AppImage) show as
`missing` in `check` and are reinstalled by `update`. Switching an app to
another format removes the old install once the new one is in place.

When a deb, rpm or Arch package is also in the distribution's repositories,
`update` says so and asks again before the first install, since system
updates and the forge's build can replace each other. `check` marks such
installs with the distro's version.

## How an asset is chosen

1. Formats the system can install, in order: native package format first
   (rpm on dnf/zypper systems, Arch packages on pacman systems, deb on apt
   systems), then Flatpak if
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

Installing a `.deb`, `.rpm` or Arch package runs the package's scripts as
root, so it means trusting the upstream project and its forge account
completely.
Forager shows the asset and whether it can be verified before installing,
and with `--yes` it refuses unverified assets unless `--allow-unverified`
is given. A checksum published next to the asset only guards against
corrupted downloads, not a compromised release.

To guard against that, give Forager the project's signing key; assets
without a valid signature are then refused, with no override:

```sh
forager add https://github.com/jedisct1/minisign --bin x86_64/minisign \
    --minisign-key RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3
forager add https://github.com/keepassxreboot/keepassxc --gpg-key keepassxc.asc
forager add https://github.com/owner/repo --attestation   # GitHub attestations, needs `gh auth login`
forager edit keepassxc --no-signature
```

Forager looks for `<asset>.minisig`, or `<asset>.asc` / `.sig` / `.gpg`,
and falls back to a signed checksum list such as `SHA256SUMS.asc`. GPG keys
are kept in `~/.local/share/forager/keys` and checked with `gpgv`; nothing
is added to your own keyring. Get the key from the project's website or
check its fingerprint there, not from the release being verified.

## Layout

- `crates/forager-core`: sources, asset matching, version comparison,
  verification, installation, config store
- `crates/forager-cli`: the `forager` command

## Roadmap

- Generic HTML sources
- PackageKit over D-Bus instead of `pkexec` + native tools
- GTK4 / libadwaita GUI
