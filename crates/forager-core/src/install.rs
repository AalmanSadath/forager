//! Hands downloaded packages to the system package manager.
//!
//! deb, rpm and Arch packages go through PackageKit, or the native tool
//! under `pkexec` where PackageKit is missing; either way polkit asks the
//! user for authorisation. AppImages and tarballs need no root: AppImages are placed in
//! `~/Applications`, tarballs are unpacked into `~/.local/opt` with their
//! executables linked into `~/.local/bin`, and Flatpak bundles go into the
//! user's Flatpak installation.

mod flatpak;
mod packagekit;
mod tarball;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};
use crate::host;
use crate::model::{App, Format, Installed};
use crate::platform::{self, PackageManager, Platform};

/// Installs `file` for `app` and returns what is needed to update or remove
/// it later. The app's install in the same format is taken to be the
/// version being replaced.
/// `progress` gets percentages from PackageKit installs.
pub fn install(
    file: &Path,
    format: Format,
    app: &App,
    tag: &str,
    platform: &Platform,
    progress: &mut dyn FnMut(u32),
) -> Result<Installed> {
    let file = fs::canonicalize(file)?;
    let asset = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut installed = Installed {
        tag: tag.to_string(),
        format,
        asset,
        package: None,
        path: None,
        links: Vec::new(),
    };

    installed.package = package_name(&file, format)?;
    match format {
        Format::Rpm => system_install(&file, progress, || match platform.package_manager {
            Some(PackageManager::Dnf) => run(pkexec(["dnf", "install", "-y"]).arg(&file)),
            Some(PackageManager::Zypper) => {
                run(pkexec(["zypper", "--non-interactive", "install", "--allow-unsigned-rpm"]).arg(&file))
            }
            _ => Err(Error::NoPackageManager("rpm")),
        })?,
        Format::Deb => system_install(&file, progress, || match platform.package_manager {
            Some(PackageManager::Apt) => run(pkexec(["apt-get", "install", "-y"]).arg(&file)),
            _ => Err(Error::NoPackageManager("deb")),
        })?,
        Format::Pacman => system_install(&file, progress, || match platform.package_manager {
            Some(PackageManager::Pacman) => run(pkexec(["pacman", "-U", "--noconfirm"]).arg(&file)),
            _ => Err(Error::NoPackageManager("pacman")),
        })?,
        Format::AppImage => {
            let dir = platform::appimage_dir().ok_or(Error::NoHome)?;
            fs::create_dir_all(&dir)?;
            let target = dir.join(format!("{}.AppImage", app.id));
            fs::copy(&file, &target)?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
            installed.path = Some(target);
        }
        Format::Flatpak => {
            if !platform.flatpak {
                return Err(Error::NoPackageManager("flatpak"));
            }
            let id = flatpak::app_id(&file)?;
            flatpak::install(&file)?;
            installed.package = Some(id);
        }
        Format::Tarball => {
            let opt_dir = platform::opt_dir().ok_or(Error::NoHome)?;
            let bin_dir = platform::bin_dir().ok_or(Error::NoHome)?;
            let old_links = app
                .install_of(Format::Tarball)
                .map_or(&[][..], |previous| previous.links.as_slice());
            let placed = tarball::install(&file, &app.id, &app.bins, &opt_dir, &bin_dir, old_links)?;
            installed.path = Some(placed.dir);
            installed.links = placed.links;
        }
    }
    Ok(installed)
}

/// Name of the package inside a deb, rpm or Arch package; `None` for
/// formats that are not system packages.
pub fn package_name(file: &Path, format: Format) -> Result<Option<String>> {
    let mut command = match format {
        Format::Rpm => host::query("rpm"),
        Format::Deb => host::query("dpkg-deb"),
        Format::Pacman => host::query("pacman"),
        _ => return Ok(None),
    };
    match format {
        Format::Rpm => command.args(["-qp", "--qf", "%{NAME}"]).arg(file),
        Format::Deb => command.arg("-f").arg(file).arg("Package"),
        _ => command.arg("-Qqp").arg(file),
    };
    output(&mut command).map(Some)
}

/// Newest version of `package` in the distribution's repositories, if they
/// carry it. Uses only cached repository metadata, so it is quick and needs
/// no network.
pub fn repo_version(package: &str, platform: &Platform) -> Option<String> {
    let query = |program: &str, args: &[&str]| {
        let out = host::query(program).args(args).arg(package).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    match platform.package_manager? {
        PackageManager::Dnf => last_line(&query(
            "dnf",
            &["-q", "--cacheonly", "repoquery", "--available", "--qf", "%{version}\n"],
        )?),
        PackageManager::Zypper => zypper_repo_version(&query("zypper", &["--no-refresh", "-q", "info"])?),
        PackageManager::Apt => madison_version(&query("apt-cache", &["madison"])?),
        PackageManager::Pacman => field(&query("pacman", &["-Si"])?, "Version"),
    }
}

/// dnf lists every available version, oldest first.
fn last_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string)
}

/// `Key : value` output of `pacman -Si` and `zypper info`.
fn field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })
}

/// zypper also describes installed packages; `@System` means no repository has it.
fn zypper_repo_version(text: &str) -> Option<String> {
    match field(text, "Repository") {
        Some(repo) if repo != "@System" => field(text, "Version"),
        _ => None,
    }
}

/// `apt-cache madison` prints `name | version | source` per repository version.
fn madison_version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.split('|').nth(1))
        .map(|v| v.trim().to_string())
}

/// Version of `package` installed by the system package manager, without
/// epoch or packaging revision: `rpm` gives the upstream version, `dpkg`
/// and `pacman` append `-<revision>` and may prefix `<epoch>:`.
pub fn installed_version(package: &str, format: Format) -> Option<String> {
    let text = match format {
        Format::Rpm => output(host::query("rpm").args(["-q", "--qf", "%{VERSION}\\n", package])).ok()?,
        Format::Deb => {
            output(host::query("dpkg-query").args(["-W", "-f=${db:Status-Status} ${Version}", package])).ok()?
        }
        Format::Pacman => output(host::query("pacman").args(["-Q", package])).ok()?,
        _ => return None,
    };
    upstream_version(query_version(&text, format)?, format)
}

/// The version in the output of the queries `installed_version` runs.
fn query_version(text: &str, format: Format) -> Option<&str> {
    match format {
        // One line per installed architecture of a multilib package.
        Format::Rpm => text.lines().next(),
        // A removed package keeps `config-files`, a broken one `half-installed`.
        Format::Deb => text.strip_prefix("installed "),
        // `name version-revision`
        _ => text.split_whitespace().nth(1),
    }
}

fn upstream_version(text: &str, format: Format) -> Option<String> {
    let text = text.trim();
    let text = text.split_once(':').map_or(text, |(_, v)| v);
    let text = match format {
        Format::Deb | Format::Pacman => text.rsplit_once('-').map_or(text, |(v, _)| v),
        _ => text,
    };
    (!text.is_empty()).then(|| text.to_string())
}

/// Package names an asset is likely to install: the start of its file name
/// (`tool-1.2.0-1.x86_64.rpm`, `tool_1.2.0_amd64.deb`,
/// `Tool-v1.2.0-x86_64.flatpak`, `tool.flatpak`), then the app's id.
pub fn package_guesses(asset: &str, format: Format, id: &str) -> Vec<String> {
    let stem = [".rpm", ".deb", ".flatpak", ".pkg.tar.zst", ".pkg.tar.xz"]
        .iter()
        .find_map(|ext| asset.strip_suffix(ext))
        .unwrap_or(asset);
    let from_file = match format {
        Format::Deb => stem.split('_').next().unwrap_or(stem),
        _ => {
            // Up to the first `-<version>` or `-v<version>`.
            let version = |rest: &str| {
                let rest = rest.strip_prefix('v').unwrap_or(rest);
                rest.starts_with(|c: char| c.is_ascii_digit())
            };
            stem.char_indices()
                .find(|&(i, c)| c == '-' && version(&stem[i + 1..]))
                .map_or(stem, |(i, _)| &stem[..i])
        }
    };
    // dpkg and pacman names are lowercase; rpm names keep their case, so
    // the name as written comes first there.
    let as_written = (format == Format::Rpm).then_some(from_file);
    let mut names = Vec::new();
    for name in as_written.into_iter().chain([from_file, id]) {
        let name = if as_written == Some(name) && names.is_empty() {
            name.to_string()
        } else {
            name.to_ascii_lowercase()
        };
        let valid = !name.is_empty()
            && !name.starts_with(|c: char| c == '-' || c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || "+-._".contains(c));
        if valid && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Version of the installed Flatpak app `id`, if it declares one.
pub fn flatpak_version(id: &str) -> Option<String> {
    version_of(&flatpak::installed_apps(), id)
}

fn version_of(apps: &[(String, String)], id: &str) -> Option<String> {
    apps.iter()
        .find(|(app, version)| app == id && !version.is_empty())
        .map(|(_, version)| version.clone())
}

/// The installed Flatpak app named like one of `names`: its id, or the last
/// part of it (`org.example.Tool` for `tool`), compared without case. Gives
/// `(id, version)`.
pub fn find_flatpak(names: &[String]) -> Option<(String, String)> {
    let apps = flatpak::installed_apps();
    names.iter().find_map(|name| {
        apps.iter()
            .find(|(id, _)| {
                let id = id.to_ascii_lowercase();
                id == *name || id.rsplit('.').next() == Some(name.as_str())
            })
            .cloned()
    })
}

/// Whether the install is still on the system, i.e. was not removed outside
/// Forager.
pub fn is_present(installed: &Installed) -> bool {
    match (
        installed.format,
        installed.package.as_deref(),
        installed.path.as_deref(),
    ) {
        (Format::Rpm, Some(package), _) => host::query("rpm")
            .args(["-q", "--quiet", package])
            .status()
            .is_ok_and(|s| s.success()),
        (Format::Deb, Some(package), _) => {
            output(host::query("dpkg-query").args(["-W", "-f=${db:Status-Status}", package]))
                .is_ok_and(|status| status == "installed")
        }
        (Format::Pacman, Some(package), _) => host::query("pacman")
            .args(["-Q", package])
            .output()
            .is_ok_and(|out| out.status.success()),
        (Format::Flatpak, Some(id), _) => flatpak::is_present(id),
        (Format::AppImage | Format::Tarball, _, Some(path)) => path.exists(),
        _ => false,
    }
}

pub fn uninstall(installed: &Installed, platform: &Platform) -> Result<()> {
    match (
        installed.format,
        installed.package.as_deref(),
        installed.path.as_deref(),
    ) {
        (Format::Rpm, Some(package), _) => system_remove(package, || match platform.package_manager {
            Some(PackageManager::Dnf) => run(&mut pkexec(["dnf", "remove", "-y", package])),
            Some(PackageManager::Zypper) => run(&mut pkexec(["zypper", "--non-interactive", "remove", package])),
            _ => Err(Error::NoPackageManager("rpm")),
        }),
        (Format::Deb, Some(package), _) => system_remove(package, || match platform.package_manager {
            Some(PackageManager::Apt) => run(&mut pkexec(["apt-get", "remove", "-y", package])),
            _ => Err(Error::NoPackageManager("deb")),
        }),
        (Format::Pacman, Some(package), _) => system_remove(package, || match platform.package_manager {
            Some(PackageManager::Pacman) => run(&mut pkexec(["pacman", "-R", "--noconfirm", package])),
            _ => Err(Error::NoPackageManager("pacman")),
        }),
        (Format::AppImage, _, Some(path)) => match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        },
        (Format::Flatpak, Some(id), _) => flatpak::uninstall(id),
        (Format::Tarball, _, Some(dir)) => {
            // The path comes from the config file; never delete outside ~/.local/opt.
            let opt_dir = platform::opt_dir().ok_or(Error::NoHome)?;
            if dir.parent() != Some(opt_dir.as_path()) {
                return Err(Error::CommandFailed(format!(
                    "refusing to remove {}: not in {}",
                    dir.display(),
                    opt_dir.display()
                )));
            }
            tarball::uninstall(dir, &installed.links)
        }
        _ => Err(Error::CommandFailed(
            "install record is missing package name or path".into(),
        )),
    }
}

/// Installs a system package through PackageKit when it is available, else
/// with `fallback` (the native tool under pkexec). Also falls back when
/// polkit refuses PackageKit, as happens without a graphical authentication
/// agent (e.g. over SSH); pkexec can then ask on the terminal.
fn system_install(file: &Path, progress: &mut dyn FnMut(u32), fallback: impl FnOnce() -> Result<()>) -> Result<()> {
    if packagekit::available() {
        match packagekit::install_file(file, progress) {
            Err(Error::PackageKit {
                code: packagekit::ERROR_NOT_AUTHORIZED,
                ..
            }) => {}
            other => return other,
        }
    }
    fallback()
}

fn system_remove(package: &str, fallback: impl FnOnce() -> Result<()>) -> Result<()> {
    if packagekit::available() {
        match packagekit::remove(package, &mut |_| {}) {
            Err(Error::PackageKit {
                code: packagekit::ERROR_NOT_AUTHORIZED,
                ..
            }) => {}
            other => return other,
        }
    }
    fallback()
}

fn pkexec<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut command = host::command("pkexec");
    command.args(args);
    command
}

fn run(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed(format!("{command:?} exited with {status}")))
    }
}

fn output(command: &mut Command) -> Result<String> {
    let out = command.output()?;
    if !out.status.success() {
        return Err(Error::CommandFailed(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_package_names_from_asset_names() {
        assert_eq!(
            package_guesses("forager-0.1.0-1.x86_64.rpm", Format::Rpm, "forager"),
            ["forager"]
        );
        assert_eq!(
            package_guesses("Tool_1.2.0_amd64.deb", Format::Deb, "my-tool"),
            ["tool", "my-tool"]
        );
        assert_eq!(
            package_guesses("foo-bar-2.0-1-x86_64.pkg.tar.zst", Format::Pacman, "foo"),
            ["foo-bar", "foo"]
        );
        assert_eq!(package_guesses("1.0.rpm", Format::Rpm, "x"), ["x"]);
        assert_eq!(
            package_guesses("OpenLogi-v0.8.17-x86_64.flatpak", Format::Flatpak, "openlogi-flatpak"),
            ["openlogi", "openlogi-flatpak"]
        );
        assert_eq!(
            package_guesses("bifrossh.flatpak", Format::Flatpak, "BifroSSH"),
            ["bifrossh"]
        );
    }

    #[test]
    fn finds_a_flatpaks_version() {
        let apps = [
            ("org.example.Tool".to_string(), "1.2.0".to_string()),
            ("org.example.Bare".into(), String::new()),
        ];
        assert_eq!(version_of(&apps, "org.example.Tool").as_deref(), Some("1.2.0"));
        assert_eq!(version_of(&apps, "org.example.Bare"), None);
        assert_eq!(version_of(&apps, "org.example.tool"), None);
    }

    #[test]
    fn strips_epoch_and_revision() {
        assert_eq!(
            upstream_version("1:2.3.4-1ubuntu2", Format::Deb).as_deref(),
            Some("2.3.4")
        );
        assert_eq!(upstream_version("0.1.0-1", Format::Pacman).as_deref(), Some("0.1.0"));
        assert_eq!(upstream_version("0.1.0", Format::Rpm).as_deref(), Some("0.1.0"));
        assert_eq!(upstream_version("1.0-rc1-2", Format::Deb).as_deref(), Some("1.0-rc1"));
    }

    #[test]
    fn reads_package_manager_queries() {
        assert_eq!(query_version("1.2.0\n1.2.0\n", Format::Rpm), Some("1.2.0"));
        assert_eq!(
            query_version("installed 1:2.0-1ubuntu1", Format::Deb),
            Some("1:2.0-1ubuntu1")
        );
        assert_eq!(query_version("config-files 2.0-1", Format::Deb), None);
        assert_eq!(query_version("tool 0.1.1-1", Format::Pacman), Some("0.1.1-1"));
    }

    #[test]
    fn placed_install_is_present_while_its_path_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tool.AppImage");
        let installed = Installed {
            tag: "v1".into(),
            format: Format::AppImage,
            asset: "tool.AppImage".into(),
            package: None,
            path: Some(path.clone()),
            links: Vec::new(),
        };
        assert!(!is_present(&installed));
        fs::write(&path, "").unwrap();
        assert!(is_present(&installed));
    }

    #[test]
    fn parses_repository_versions() {
        assert_eq!(last_line("14.1.1\n15.2.0\n\n").as_deref(), Some("15.2.0"));
        assert_eq!(last_line(""), None);

        let pacman = "Repository      : extra\nName            : tool\nVersion         : 1:2.3-1\n";
        assert_eq!(field(pacman, "Version").as_deref(), Some("1:2.3-1"));

        let zypper = "Information for package tool:\nRepository     : repo-oss\nName           : tool\nVersion        : 1.2-3.1\n";
        assert_eq!(zypper_repo_version(zypper).as_deref(), Some("1.2-3.1"));
        assert_eq!(zypper_repo_version(&zypper.replace("repo-oss", "@System")), None);

        let madison = "      tool | 1.2-1 | http://deb.debian.org/debian trixie/main amd64 Packages\n";
        assert_eq!(madison_version(madison).as_deref(), Some("1.2-1"));
        assert_eq!(madison_version(""), None);
    }
}
