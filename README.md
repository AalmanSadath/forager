# Forager

[![CI](https://github.com/AalmanSadath/forager/actions/workflows/ci.yml/badge.svg)](https://github.com/AalmanSadath/forager/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/AalmanSadath/forager)](https://github.com/AalmanSadath/forager/releases/latest)
[![Add to Forager](https://aalmansadath.github.io/forager/badge.svg)](https://aalmansadath.github.io/forager/add/?url=https%3A%2F%2Fgithub.com%2FAalmanSadath%2Fforager)

Install and update desktop Linux apps straight from their releases, using
the system package manager. Think
[Obtainium](https://github.com/ImranR98/Obtainium), but for the desktop.

![The apps grid](docs/screenshots/apps.png)

## Features

- **Sources:** GitHub, GitLab, Forgejo/Codeberg (hosted or self-hosted),
  SourceForge, SourceHut, Jenkins, direct links, download pages.
- **Formats:** rpm, deb, Arch packages, Flatpak bundles, AppImages, tarballs.
- **Verification:** SHA-256 checksums; minisign, GPG or GitHub attestations.
- **AppImages:** delta updates via `.zsync`; adopt ones downloaded by hand.
- **Track-only** apps, **background checks**, **export/import** and
  **"Add to Forager" badges**.
- A desktop app and a CLI over the same core.

## Install

Each [release](https://github.com/AalmanSadath/forager/releases) has:

| File | For | Install |
| --- | --- | --- |
| `forager_<v>_amd64.deb` | Debian 13, Ubuntu 24.04 and newer | `sudo apt install ./forager_<v>_amd64.deb` |
| `forager-<v>.x86_64.rpm` | Fedora, openSUSE | `sudo dnf install ./forager-<v>.x86_64.rpm` |
| `forager-<v>-x86_64.pkg.tar.zst` | Arch | `sudo pacman -U forager-<v>-x86_64.pkg.tar.zst` |
| `forager-<v>-x86_64.flatpak` | any distribution with Flatpak | `flatpak install --user forager-<v>-x86_64.flatpak` |
| `Forager-<v>-x86_64.AppImage` | any distribution | `chmod +x` it and run it |
| `forager-<v>-x86_64-linux.tar.gz` | any distribution | unpack, then `./install.sh` (into `~/.local`) |

- All but the AppImage include both the app (`forager-gui`) and the CLI (`forager`).
- Verify with `SHA256SUMS` or `gh attestation verify <file> -R AalmanSadath/forager`.
- The Flatpak is not a sandbox: it drives the host's package tools
  (permissions explained in `packaging/flatpak/*.yml`).

Forager updates itself like any other app:

```sh
forager add https://github.com/AalmanSadath/forager --attestation   # needs `gh auth login`
forager edit forager --format flatpak                              # only if you installed the Flatpak
```

## The app

The app page shows every asset of the latest release and why it was or
wasn't chosen, live as you change settings. Updates show what will be
installed and how it's verified before anything downloads.

| App page | Adding an app |
| --- | --- |
| ![An app's page](docs/screenshots/app-page.png) | ![Adding an app](docs/screenshots/add-app.png) |

| Updating | Activity | Settings |
| --- | --- | --- |
| ![The update dialog](docs/screenshots/update.png) | ![The activity log](docs/screenshots/activity.png) | ![Settings](docs/screenshots/settings.png) |

`forager-gui add <url>` and `forager-gui open <id>` open those pages directly.

## Command line

```sh
# Add
forager add https://github.com/jgraph/drawio-desktop
forager add https://gitlab.com/gitlab-org/cli --id glab
forager add https://git.example.org/team/tool --forge forgejo        # self-hosted
forager add https://nodejs.org/dist/latest/ --forge html \
    --pattern 'node-v[0-9.]+-linux-x64\.tar\.xz$' --bin bin/node      # a download page
forager add https://sourceforge.net/projects/smplayer --format appimage
forager add https://ci.example.org/job/tool/                         # a Jenkins job
forager add https://zoom.us/client/latest/zoom_x86_64.rpm            # a direct link
forager adopt ~/Downloads/nvim-linux-x86_64.appimage                 # an AppImage you downloaded

# Check and update
forager check                    # installed vs latest, and the asset that would be used
forager check --json             # also for list and history
forager update                   # every app, asks first
forager update drawio-desktop --yes

# Change, roll back, skip
forager edit drawio-desktop --format appimage   # or auto; also --filter, --bin, --prerelease
forager edit bifrossh --tag-filter '^v'         # only v… releases, not android-v…
forager notes drawio-desktop                    # release notes (or --tag)
forager install drawio-desktop --tag v24.0.0    # roll back; skips the newer release
forager skip drawio-desktop                     # don't offer this release; newer ones are
forager unskip drawio-desktop
forager remove drawio-desktop

# Track only: report releases, never install
forager add https://github.com/ImranR98/Obtainium --track-only
forager seen obtainium                          # stop reporting the latest as new

# Other
forager menu-entries                            # add missing menu entries
forager history                                 # what was installed, updated, removed, failed
forager completions bash|zsh|fish
```

### Sources

| Source | A release is | Notes |
| --- | --- | --- |
| GitHub, GitLab, Forgejo/Gitea | a forge release | self-hosted GitLab or Forgejo needs `--forge gitlab\|forgejo` |
| Web page | the links matching `--pattern` | `--forge html`; version from a `(?P<version>...)` group, else the first dotted number in the file name |
| SourceForge | a folder of files | MD5 only, so unverified unless signed |
| SourceHut | a tag and its attached files | |
| Jenkins | a successful build with archived files, `build-<n>` | public jobs only |
| Direct link | the file it leads to now | version from the file name or folder, else its date; `--forge direct` if the address has no file name |

The latest release is the newest one with a file for this system.

### `add` options

| Option | Effect |
| --- | --- |
| `--id <id>` | name for the app (default: repository name) |
| `--forge <kind>` | `gitlab`, `forgejo`, `html`, `jenkins`, `direct` for unrecognised hosts |
| `--pattern <regex>` | download links on a web page (`--forge html`) |
| `--filter <regex>` | asset names must match |
| `--tag-filter <regex>` | release tags must match |
| `--format <fmt>` | `deb`, `rpm`, `pacman`, `appimage`, `tarball` or `flatpak` instead of the best one |
| `--prerelease` | consider prereleases |
| `--bin <path>` | executable to link from a tarball (repeatable) |
| `--track-only` | never install; report new releases |
| `--minisign-key`, `--gpg-key`, `--attestation` | require a signature; see [Security](#security) |

`edit` takes the same settings; a new format is installed, and the old
install removed, on the next `update`.

### Background checks

```sh
forager timer enable                 # daily check, with a notification listing updates
forager timer enable --auto-update   # also install updates that need no root (AppImage, tarball, Flatpak)
forager timer enable --auto-update --skip-metered --skip-on-battery
forager timer status
forager timer disable
```

The timer reads variables such as `GITHUB_TOKEN` from
`~/.config/forager/env` and never asks for a password. The skip flags hold
off updates on metered connections (NetworkManager) or battery (UPower);
the check still runs.

### Sharing

```sh
forager export apps.toml       # sources, settings and signing keys; no installs
forager import apps.toml       # on another machine; --install installs them too
forager link node              # forager://add?url=…  (the app page's "Copy link")
forager link node --badge      # README badge Markdown (the app page's "Copy badge")
forager add 'forager://add?url=https://github.com/o/tool&filter=musl'
```

A link opens the Add page filled in for you to check; it never installs
anything or carries keys. Parameters: `url`, and optionally `forge`,
`pattern`, `id`, `filter`, `tag_filter`, `format`, `bin`, `prerelease=1`,
`track_only=1`. Since GitHub strips `forager://` links, badges go through
a redirect page:

```markdown
[![Add to Forager](https://aalmansadath.github.io/forager/badge.svg)](https://aalmansadath.github.io/forager/add/?url=https%3A%2F%2Fgithub.com%2Fowner%2Frepo)
```

### Configuration

| What | Where |
| --- | --- |
| Tracked apps | `~/.config/forager/apps.toml`; `--config` or `FORAGER_CONFIG` to override |
| Release cache | `~/.cache/forager`, fetched conditionally |
| History | `~/.local/state/forager/history.jsonl` |
| GPG keys | `~/.local/share/forager/keys` |
| `GITHUB_TOKEN` | raises GitHub's limit from 60 to 5000 requests/hour; unchanged releases don't count |
| `GITLAB_TOKEN`, `CODEBERG_TOKEN` | for gitlab.com and codeberg.org |
| `FORAGER_TOKEN_<HOST>` | any host, e.g. `FORAGER_TOKEN_GIT_EXAMPLE_ORG`; overrides the others |

Tokens are only sent to their own host.

## How it works

### Choosing an asset

1. Formats in order:
   1. the native package (rpm on dnf/zypper, Arch on pacman, deb on apt);
   2. Flatpak, if installed;
   3. AppImage;
   4. tarball.
2. Assets naming this CPU architecture beat ones naming none. Assets
   naming another architecture or OS are skipped.
3. Debug and source packages are skipped, as are tarballs naming neither
   Linux nor this architecture.

### Existing installs

- Installs made by hand or by the distribution are recognised by package
  name, with versions read from the package manager.
- An app installed in several formats is updated in each.
- An install removed outside Forager shows as `missing`; `update`
  reinstalls it.
- If the distribution also ships the package, Forager warns before the
  first install.

### AppImages

- Installed to `~/Applications` with a menu entry taken from the image.
- **Adopt** (`forager adopt`, "Add AppImage…", or drop on the window): the
  project comes from the AppImage's update info, or `--url`. `--keep`
  copies instead of moving.
- **Delta updates:** with `<asset>.zsync` in the release, only changed
  blocks download. The result is checked against the zsync SHA-1, falling
  back to a full download.

### Tarballs

- Unpacked to `~/.local/opt/<id>`. The executable (at the top, in `bin/`,
  or named after the id; or `--bin`) is linked into `~/.local/bin`.
- Shipped menu entries, man pages and completions are linked into
  `~/.local/share`. zsh needs `fpath+=~/.local/share/zsh/site-functions`.
- Files Forager didn't create are never replaced.

### System packages

Installed through PackageKit (one polkit prompt, with progress), or with
dnf, zypper, apt-get or pacman under `pkexec` when PackageKit or a
graphical polkit agent is missing.

## Security

System packages run scripts as root: installing one means trusting the
project. A checksum only catches corrupted downloads, so with `--yes`
unverified assets are refused unless `--allow-unverified` is given.

Against a compromised release, give Forager the signing key; unsigned
assets are then refused, with no override:

```sh
forager add https://github.com/jedisct1/minisign --bin x86_64/minisign \
    --minisign-key RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3
forager add https://github.com/keepassxreboot/keepassxc --gpg-key keepassxc.asc
forager add https://github.com/owner/repo --attestation   # needs `gh auth login`
forager edit keepassxc --no-signature
```

Forager looks for `<asset>.minisig`, `.asc`, `.sig` or `.gpg`, or a signed
checksum list (`SHA256SUMS.asc`). GPG keys are checked with `gpgv` and kept
out of your keyring. Get keys from the project's website, not the release.

## Building from source

```sh
cargo build --release          # the CLI, target/release/forager
cd gui && npm ci
npx tauri dev                  # run the app from source
npx tauri build                # deb, rpm and AppImage under target/release/bundle
```

Needs Node 20+ and WebKitGTK 4.1 headers (`webkit2gtk4.1-devel` /
`libwebkit2gtk-4.1-dev`). `packaging/build-all.sh` builds every release
file into `target/dist`.

## Layout

- `crates/forager-core`: sources, asset matching, versions, verification,
  installation, config, history
- `crates/forager-cli`: the `forager` command
- `gui/`: the desktop app (Tauri 2). `gui/src-tauri` is the Rust side,
  `gui/src` the React frontend. Its look follows
  [BifroSSH](https://github.com/AalmanSadath/BifroSSH).

## Licence

GPL-3.0-or-later; see `LICENSE`.
