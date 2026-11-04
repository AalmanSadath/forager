//! Carrying a plan out: downloading and verifying the asset, installing it,
//! and adopting files the user already has.

use std::fs;
use std::ops::ControlFlow;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::matcher::select_asset;
use crate::model::{App, Format, Installed};
use crate::platform::Platform;
use crate::signature;
use crate::store::{self, Store};
use crate::verify::Algorithm;
use crate::{install, verify, version, zsync};

use super::plan::Plan;
use super::status::{same_version, sort_installs};

/// Progress of `download` and `install`, for frontends to display.
#[derive(Debug, Clone, Copy)]
pub enum Progress {
    Download {
        done: u64,
        total: Option<u64>,
    },
    /// Percentage reported by PackageKit while installing a system package.
    Install {
        percent: u32,
    },
}

/// A downloaded and verified asset, deleted when dropped.
#[derive(Debug)]
pub struct Download {
    pub path: PathBuf,
    /// Package name for system packages (deb, rpm, Arch).
    pub package: Option<String>,
    /// Version of the same package in the distribution's repositories. A
    /// distro update may then replace the forge's build, or the reverse.
    pub repo_version: Option<String>,
    /// How the delta download went, when the plan had one.
    pub delta: Option<DeltaOutcome>,
    /// The plan's `.flatpakrepo`, downloaded.
    pub repo_file: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOutcome {
    /// Bytes taken from the installed version, of the new one's `length`.
    Reused { reused: u64, length: u64 },
    /// Why it was downloaded whole instead.
    Failed(String),
}

impl std::fmt::Display for DeltaOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeltaOutcome::Reused { reused, length } => write!(
                f,
                "reused {:.1} of {:.1} MiB from the installed version",
                *reused as f64 / 1048576.0,
                *length as f64 / 1048576.0
            ),
            DeltaOutcome::Failed(reason) => write!(f, "{reason}; downloaded in full"),
        }
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        fs::remove_file(&self.path).ok();
        if let Some(repo_file) = &self.repo_file {
            fs::remove_file(repo_file).ok();
        }
    }
}

/// Downloads the plan's asset and checks it against the expected checksum.
/// `progress` may return `Break` to cancel (`Error::Cancelled`).
pub fn download(
    plan: &Plan,
    platform: &Platform,
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<Download> {
    if plan.via_repo.is_some() {
        return Ok(Download {
            path: PathBuf::new(),
            package: None,
            repo_version: None,
            delta: None,
            repo_file: None,
        });
    }
    let dir = store::download_dir();
    let mut on_bytes = |done, total| progress(Progress::Download { done, total });
    let mut delta = None;
    let mut fetched = None;
    if let Some(d) = &plan.delta {
        match zsync::download(&plan.asset, &d.control.url, &d.seed, &dir, &mut on_bytes) {
            Ok((path, hash, stats)) => {
                delta = Some(DeltaOutcome::Reused {
                    reused: stats.reused,
                    length: stats.length,
                });
                fetched = Some((path, hash));
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => delta = Some(DeltaOutcome::Failed(e.to_string())),
        }
    }
    let (path, actual) = match fetched {
        Some(fetched) => fetched,
        None => verify::download(&plan.asset, &dir, &mut on_bytes)?,
    };
    let mut download = Download {
        path,
        package: None,
        repo_version: None,
        delta,
        repo_file: None,
    };
    check_checksum(plan, &download.path, &actual)?;
    if let Some(repo) = &plan.repo_file {
        let (path, sha256) = verify::download(repo, &dir, &mut |_, _| ControlFlow::Continue(()))?;
        download.repo_file = Some(path.clone());
        if let Some(expected) = verify::expected(repo, &plan.release)? {
            let actual = match expected.algorithm {
                Algorithm::Sha256 => sha256,
                algorithm => verify::hash_file(&path, algorithm)?,
            };
            if actual != expected.hex {
                return Err(Error::ChecksumMismatch {
                    asset: repo.name.clone(),
                    expected: expected.hex,
                    actual,
                });
            }
        }
    }
    if let Some(check) = &plan.signature {
        signature::check(&check.signature, &check.signed, &check.source, &download.path, &actual)?;
    }
    download.package = install::package_name(&download.path, plan.format)?;
    download.repo_version = download
        .package
        .as_deref()
        .and_then(|p| install::repo_version(p, platform));
    Ok(download)
}

/// Checks the download at `path`, whose SHA-256 is `sha256`, against the
/// plan's checksum. A SHA-512 means reading the file once more.
fn check_checksum(plan: &Plan, path: &Path, sha256: &str) -> Result<()> {
    let Some(expected) = &plan.checksum else {
        return Ok(());
    };
    let actual = match expected.algorithm {
        Algorithm::Sha256 => sha256.to_string(),
        algorithm => verify::hash_file(path, algorithm)?,
    };
    match expected.hex == actual {
        true => Ok(()),
        false => Err(Error::ChecksumMismatch {
            asset: plan.asset.name.clone(),
            expected: expected.hex.clone(),
            actual,
        }),
    }
}

/// Installs a download for app `id` and records it in `store`, in place of
/// any install in the same format. Installs the plan replaces are removed
/// afterwards, so a failed install never leaves the app missing.
pub fn install(
    store: &mut Store,
    id: &str,
    plan: &Plan,
    download: &Download,
    platform: &Platform,
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<Installed> {
    let installed = match &plan.via_repo {
        Some(remote) => update_from_repo(
            store.get(id)?,
            &remote.name,
            plan.repo_commit.as_deref(),
            &plan.release.tag,
        )?,
        None => {
            let installed = install::install(
                &download.path,
                plan.format,
                store.get(id)?,
                &plan.release.tag,
                plan.downgrade,
                platform,
                // A package manager cannot be stopped halfway, so `Break` is ignored.
                &mut |percent| {
                    let _ = progress(Progress::Install { percent });
                },
            )?;
            // A bundle naming no repository: install from the release's.
            if let (Some(repo_file), Some(flatpak_id)) = (&download.repo_file, installed.package.as_deref())
                && install::flatpak::origin(flatpak_id).is_none()
            {
                install::flatpak::install_from_repo_file(repo_file, flatpak_id)?;
            }
            installed
        }
    };
    let app = store.get_mut(id)?;
    let mut installs: Vec<Installed> = app.installs().filter(|i| i.format != plan.format).cloned().collect();
    installs.push(installed.clone());
    sort_installs(&mut installs, app.format, platform);
    app.set_installs(installs);
    if plan.skip_after.is_some() {
        app.skipped = plan.skip_after.clone();
    }
    store.save()?;

    for previous in plan.replaces.iter().filter(|p| p.format != installed.format) {
        if install::is_present(previous) {
            install::uninstall(previous, platform).map_err(|source| Error::PreviousNotRemoved {
                format: previous.format,
                source: Box::new(source),
            })?;
        }
        let app = store.get_mut(id)?;
        let installs = app
            .installs()
            .filter(|i| i.format != previous.format)
            .cloned()
            .collect();
        app.set_installs(installs);
        store.save()?;
    }
    Ok(installed)
}

/// Updates `app`'s Flatpak from the repository `remote` it came from, and
/// gives the install as at `tag`. Fails when the repository has not caught
/// up with the release yet, leaving the record as it was.
fn update_from_repo(app: &App, remote: &str, commit: Option<&str>, tag: &str) -> Result<Installed> {
    let current = app
        .install_of(Format::Flatpak)
        .filter(|i| i.package.is_some())
        .ok_or_else(|| Error::CommandFailed(format!("{} has no Flatpak install to update", app.id)))?;
    let flatpak_id = current.package.as_deref().unwrap_or_default();
    if let Some(commit) = commit {
        install::flatpak::update_to(flatpak_id, commit)?;
        return Ok(Installed {
            tag: tag.to_string(),
            ..current.clone()
        });
    }
    install::flatpak::update(flatpak_id)?;
    if let Some(actual) = install::flatpak_version(flatpak_id)
        && repo_behind(&actual, tag)
    {
        return Err(Error::CommandFailed(format!(
            "the {remote} repository has {actual}, not {tag} yet; try again later"
        )));
    }
    Ok(Installed {
        tag: tag.to_string(),
        ..current.clone()
    })
}

/// Whether the version a repository installed, `actual`, is older than the
/// release `tag`. Unknown when either is not a version: then not behind.
pub(super) fn repo_behind(actual: &str, tag: &str) -> bool {
    match (version::parse_tag(actual), version::parse_tag(tag)) {
        (Some(actual), Some(tag)) => actual < tag,
        _ => false,
    }
}

/// Tracks `app` with an AppImage the user already has at `file` as its
/// install, as if Forager had installed it: the file goes to
/// `~/Applications/<id>.AppImage` (moved, or copied when `keep` is set or
/// it is a file the desktop portal handed over) and gets a menu entry.
/// `version` is the AppImage's own; the release of that version names the
/// install when the source has one. Saves `store`.
pub fn adopt(store: &mut Store, mut app: App, file: &Path, version: Option<&str>, keep: bool) -> Result<Installed> {
    app.validate()?;
    if store.apps.iter().any(|a| a.id == app.id || a.source == app.source) {
        return Err(Error::AlreadyTracked(app.id));
    }
    if crate::desktop::appimage_entry(file).is_err() {
        return Err(Error::Archive(format!("{} is not an AppImage", file.display())));
    }
    let release = version.and_then(|version| {
        app.source
            .releases()
            .ok()?
            .into_iter()
            .find(|r| same_version(&r.tag, version))
    });
    let tag = match (&release, version) {
        (Some(release), _) => release.tag.clone(),
        (None, Some(version)) => version.to_string(),
        (None, None) => "unknown".into(),
    };
    // The release's own file name, so the next update does not take a
    // renamed file for a different build.
    let release_asset = release.as_ref().and_then(|release| {
        let mut target = app.clone();
        target.format = Some(Format::AppImage);
        let (asset, _) = select_asset(&target, release, &Platform::detect()).ok()?;
        Some(asset.name.clone())
    });

    let dir = crate::platform::appimage_dir().ok_or(Error::NoHome)?;
    fs::create_dir_all(&dir)?;
    let target = dir.join(format!("{}.AppImage", app.id));
    let source = fs::canonicalize(file)?;
    if target.exists() && fs::canonicalize(&target).ok().as_ref() != Some(&source) {
        return Err(Error::LinkConflict(target));
    }
    // Files from the document portal are views of the user's own; they stay.
    let keep = keep || (source.starts_with("/run/user") && source.components().any(|c| c.as_os_str() == "doc"));
    if source != target {
        if keep {
            fs::copy(&source, &target)?;
        } else if fs::rename(&source, &target).is_err() {
            // Another filesystem: copy, then remove the original.
            fs::copy(&source, &target)?;
            fs::remove_file(&source)?;
        }
    }
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;

    let installed = Installed {
        tag,
        format: Format::AppImage,
        asset: release_asset.unwrap_or_else(|| {
            source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        }),
        package: None,
        desktop: crate::desktop::for_appimage(&app.id, &target).unwrap_or_default(),
        path: Some(target),
        links: Vec::new(),
    };
    app.set_installs(vec![installed.clone()]);
    store.add(app)?;
    store.save()?;
    Ok(installed)
}

/// Adds the menu entries missing from the AppImage and tarball installs of
/// the apps named (all when `ids` is empty), recording them in `store`.
/// Gives, per app with a missing entry, how many were added or the error.
pub fn create_menu_entries(store: &mut Store, ids: &[String]) -> Result<Vec<(String, Result<usize>)>> {
    let mut results = Vec::new();
    for app in store.apps.iter_mut().filter(|a| ids.is_empty() || ids.contains(&a.id)) {
        let missing = app
            .installs()
            .any(|i| install::menu_entry_state(i) == install::MenuEntry::Missing);
        if !missing {
            continue;
        }
        let id = app.id.clone();
        let mut installs = app.install_list();
        let mut added = 0;
        let mut error = None;
        for installed in installs
            .iter_mut()
            .filter(|i| install::menu_entry_state(i) == install::MenuEntry::Missing)
        {
            match install::create_menu_entry(&id, installed) {
                Ok(files) => {
                    installed.desktop = files;
                    added += 1;
                }
                Err(e) => error = Some(e),
            }
        }
        app.set_installs(installs);
        results.push((id, error.map_or(Ok(added), Err)));
    }
    store.save()?;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::fixtures::plan;
    use crate::verify::Checksum;

    #[test]
    fn checksum_mismatch_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool.tar.gz");
        fs::write(&file, "test").unwrap();
        let plan = plan(Format::Tarball, Some("a".repeat(64)));
        assert!(matches!(
            check_checksum(&plan, &file, &"b".repeat(64)),
            Err(Error::ChecksumMismatch { .. })
        ));
        assert!(check_checksum(&plan, &file, &"a".repeat(64)).is_ok());

        // A SHA-512 is computed from the file itself.
        let sha512 = verify::hash_file(&file, Algorithm::Sha512).unwrap();
        let mut plan512 = plan.clone();
        plan512.checksum = Some(Checksum {
            algorithm: Algorithm::Sha512,
            hex: sha512,
            from: Some("latest-linux.yml".into()),
        });
        assert!(check_checksum(&plan512, &file, "unused").is_ok());
        plan512.checksum.as_mut().unwrap().hex = "0".repeat(128);
        assert!(matches!(
            check_checksum(&plan512, &file, "unused"),
            Err(Error::ChecksumMismatch { .. })
        ));

        let unverified = Plan { checksum: None, ..plan };
        assert!(check_checksum(&unverified, &file, &"b".repeat(64)).is_ok());
    }
}
