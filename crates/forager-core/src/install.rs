//! Hands downloaded packages to the system package manager.
//!
//! deb and rpm installs run through `pkexec`, so polkit asks the user for
//! authorisation. AppImages and tarballs need no root: AppImages are placed in
//! `~/Applications`, tarballs are unpacked into `~/.local/opt` with their
//! executables linked into `~/.local/bin`.

mod tarball;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};
use crate::model::{App, Format, Installed};
use crate::platform::{self, PackageManager, Platform};

/// Installs `file` for `app` and returns what is needed to update or remove
/// it later. `app.installed` is taken to be the version being replaced.
pub fn install(file: &Path, format: Format, app: &App, tag: &str, platform: &Platform) -> Result<Installed> {
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

    match format {
        Format::Rpm => {
            installed.package = Some(output(Command::new("rpm").args(["-qp", "--qf", "%{NAME}"]).arg(&file))?);
            match platform.package_manager {
                Some(PackageManager::Dnf) => run(pkexec(["dnf", "install", "-y"]).arg(&file))?,
                Some(PackageManager::Zypper) => run(pkexec([
                    "zypper",
                    "--non-interactive",
                    "install",
                    "--allow-unsigned-rpm",
                ])
                .arg(&file))?,
                _ => return Err(Error::NoPackageManager("rpm")),
            }
        }
        Format::Deb => {
            installed.package = Some(output(Command::new("dpkg-deb").arg("-f").arg(&file).arg("Package"))?);
            match platform.package_manager {
                Some(PackageManager::Apt) => run(pkexec(["apt-get", "install", "-y"]).arg(&file))?,
                _ => return Err(Error::NoPackageManager("deb")),
            }
        }
        Format::AppImage => {
            let dir = platform::appimage_dir().ok_or(Error::NoHome)?;
            fs::create_dir_all(&dir)?;
            let target = dir.join(format!("{}.AppImage", app.id));
            fs::copy(&file, &target)?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
            installed.path = Some(target);
        }
        Format::Tarball => {
            let opt_dir = platform::opt_dir().ok_or(Error::NoHome)?;
            let bin_dir = platform::bin_dir().ok_or(Error::NoHome)?;
            let old_links = match &app.installed {
                Some(previous) if previous.format == Format::Tarball => previous.links.as_slice(),
                _ => &[],
            };
            let placed = tarball::install(&file, &app.id, &app.bins, &opt_dir, &bin_dir, old_links)?;
            installed.path = Some(placed.dir);
            installed.links = placed.links;
        }
    }
    Ok(installed)
}

pub fn uninstall(installed: &Installed, platform: &Platform) -> Result<()> {
    match (installed.format, installed.package.as_deref(), installed.path.as_deref()) {
        (Format::Rpm, Some(package), _) => match platform.package_manager {
            Some(PackageManager::Dnf) => run(&mut pkexec(["dnf", "remove", "-y", package])),
            Some(PackageManager::Zypper) => run(&mut pkexec(["zypper", "--non-interactive", "remove", package])),
            _ => Err(Error::NoPackageManager("rpm")),
        },
        (Format::Deb, Some(package), _) => match platform.package_manager {
            Some(PackageManager::Apt) => run(&mut pkexec(["apt-get", "remove", "-y", package])),
            _ => Err(Error::NoPackageManager("deb")),
        },
        (Format::AppImage, _, Some(path)) => match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        },
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
        _ => Err(Error::CommandFailed("install record is missing package name or path".into())),
    }
}

fn pkexec<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut command = Command::new("pkexec");
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
