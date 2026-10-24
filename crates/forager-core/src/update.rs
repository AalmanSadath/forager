//! Update flow shared by every frontend: work out what would be installed,
//! then download, verify and install it.

use std::fs;
use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::matcher::select_asset;
use crate::model::{App, Asset, Format, Installed, Release, Signature};
use crate::platform::Platform;
use crate::signature::{self, Signed};
use crate::source::Source;
use crate::store::{self, Store};
use crate::{install, verify, version};

/// What `apply` would install for an app.
#[derive(Debug, Clone)]
pub struct Plan {
    pub release: Release,
    pub asset: Asset,
    pub format: Format,
    /// SHA-256 the download must match, if the release publishes one.
    pub expected_sha256: Option<String>,
    /// Tag currently installed.
    pub from: Option<String>,
    /// The recorded install was removed outside Forager.
    pub missing: bool,
    /// Signature check required by the app.
    pub signature: Option<SignatureCheck>,
}

#[derive(Debug, Clone)]
pub struct SignatureCheck {
    pub signature: Signature,
    pub signed: Signed,
    pub source: Source,
}

/// How an app's install compares with its latest release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    NotInstalled,
    /// Recorded as installed, but removed outside Forager.
    Missing,
    /// Installed in another format than the one the app is pinned to.
    WrongFormat,
    Outdated,
    UpToDate,
}

impl State {
    /// Whether `update` would install something for an app that is tracked
    /// as installed.
    pub fn needs_update(self) -> bool {
        matches!(self, State::Missing | State::WrongFormat | State::Outdated)
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub state: State,
    pub latest: Release,
}

/// Fetches the latest release and compares it with the install.
pub fn status(app: &App) -> Result<Status> {
    let latest = app.source.latest_release(app.prerelease)?;
    let state = match &app.installed {
        None => State::NotInstalled,
        Some(installed) if !install::is_present(installed) => State::Missing,
        Some(installed) if app.format.is_some_and(|f| f != installed.format) => State::WrongFormat,
        Some(installed) if version::is_newer(&installed.tag, &latest.tag) => State::Outdated,
        Some(_) => State::UpToDate,
    };
    Ok(Status { state, latest })
}

/// The update for `app`, or `None` if it is up to date. An install that is
/// missing, or in another format than the one the app is pinned to, counts
/// as out of date.
pub fn plan(app: &App, platform: &Platform) -> Result<Option<Plan>> {
    let Status { state, latest: release } = status(app)?;
    if state == State::UpToDate {
        return Ok(None);
    }
    let missing = state == State::Missing;
    let (asset, format) = select_asset(app, &release, platform)?;
    let asset = asset.clone();
    let expected_sha256 = verify::expected_sha256(&asset, &release)?;
    let signature = match &app.signature {
        Some(signature) => Some(SignatureCheck {
            signed: signature::find(signature, &asset, &release)?,
            signature: signature.clone(),
            source: app.source.clone(),
        }),
        None => None,
    };
    Ok(Some(Plan {
        from: app.installed.as_ref().map(|i| i.tag.clone()),
        release,
        asset,
        format,
        expected_sha256,
        missing,
        signature,
    }))
}

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
}

impl Drop for Download {
    fn drop(&mut self) {
        fs::remove_file(&self.path).ok();
    }
}

/// Downloads the plan's asset and checks it against the expected SHA-256.
pub fn download(plan: &Plan, platform: &Platform, progress: &mut dyn FnMut(Progress)) -> Result<Download> {
    let (path, actual) = verify::download(&plan.asset, &store::download_dir(), &mut |done, total| {
        progress(Progress::Download { done, total })
    })?;
    let mut download = Download {
        path,
        package: None,
        repo_version: None,
    };
    check_sha256(plan, &actual)?;
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

fn check_sha256(plan: &Plan, actual: &str) -> Result<()> {
    match &plan.expected_sha256 {
        Some(expected) if expected != actual => Err(Error::ChecksumMismatch {
            asset: plan.asset.name.clone(),
            expected: expected.clone(),
            actual: actual.to_string(),
        }),
        _ => Ok(()),
    }
}

/// Installs a download for app `id` and records it in `store`. An install in
/// another format is removed afterwards, so a failed install never leaves
/// the app missing.
pub fn install(
    store: &mut Store,
    id: &str,
    plan: &Plan,
    download: &Download,
    platform: &Platform,
    progress: &mut dyn FnMut(Progress),
) -> Result<Installed> {
    let previous = store.get(id)?.installed.clone();
    let installed = install::install(
        &download.path,
        plan.format,
        store.get(id)?,
        &plan.release.tag,
        platform,
        &mut |percent| progress(Progress::Install { percent }),
    )?;
    store.get_mut(id)?.installed = Some(installed.clone());
    store.save()?;

    if let Some(previous) = previous
        && previous.format != installed.format
        && install::is_present(&previous)
    {
        install::uninstall(&previous, platform).map_err(|source| Error::PreviousNotRemoved {
            format: previous.format,
            source: Box::new(source),
        })?;
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_mismatch_is_an_error() {
        let plan = Plan {
            release: Release {
                tag: "v1".into(),
                name: None,
                prerelease: false,
                published_at: None,
                assets: Vec::new(),
            },
            asset: Asset {
                name: "tool.tar.gz".into(),
                url: String::new(),
                size: 0,
                digest: None,
            },
            format: Format::Tarball,
            expected_sha256: Some("a".repeat(64)),
            from: None,
            missing: false,
            signature: None,
        };
        assert!(matches!(
            check_sha256(&plan, &"b".repeat(64)),
            Err(Error::ChecksumMismatch { .. })
        ));
        assert!(check_sha256(&plan, &"a".repeat(64)).is_ok());
        let unverified = Plan {
            expected_sha256: None,
            ..plan
        };
        assert!(check_sha256(&unverified, &"b".repeat(64)).is_ok());
    }
}
