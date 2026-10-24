//! Update flow shared by every frontend: work out what would be installed,
//! then download, verify and install it.

use std::fs;
use std::ops::ControlFlow;
use std::path::PathBuf;

use serde::Serialize;

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

impl Plan {
    /// Neither a checksum nor a signature can vouch for the download.
    pub fn unverified(&self) -> bool {
        self.expected_sha256.is_none() && self.signature.is_none()
    }
}

/// How much attention a summary line needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Normal,
    Warn,
    Danger,
}

/// One line of what an update will do, for confirmation prompts.
#[derive(Debug, Clone, Serialize)]
pub struct SummaryLine {
    pub label: &'static str,
    pub value: String,
    pub tone: Tone,
}

/// What installing `plan` for `app` involves: the asset, how it is verified,
/// and anything the user should know before agreeing.
pub fn summary(app: &App, plan: &Plan) -> Vec<SummaryLine> {
    let line = |label, value: String, tone| SummaryLine { label, value, tone };
    // GitLab does not report asset sizes.
    let size = match plan.asset.size {
        0 => String::new(),
        bytes => format!("{:.1} MiB, ", bytes as f64 / 1048576.0),
    };
    let mut lines = vec![
        line(
            "asset",
            format!("{} ({size}{})", plan.asset.name, plan.format),
            Tone::Normal,
        ),
        line("source", app.source.to_string(), Tone::Normal),
    ];
    lines.push(match (&plan.expected_sha256, &plan.signature) {
        (Some(_), _) => line("sha256", "published, will verify".into(), Tone::Normal),
        (None, Some(_)) => line("sha256", "not published".into(), Tone::Normal),
        (None, None) => line(
            "sha256",
            "NOT PUBLISHED, download cannot be verified".into(),
            Tone::Danger,
        ),
    });
    if let Some(check) = &plan.signature {
        let value = format!("{} ({}), will verify", check.signature, check.signed.describe());
        lines.push(line("signed", value, Tone::Normal));
    }
    if plan.format.needs_root() {
        lines.push(line("note", "package scripts run as root".into(), Tone::Warn));
    }
    if plan.format == Format::Flatpak {
        let value = "a missing runtime is fetched from the bundle's runtime repo (usually Flathub)".into();
        lines.push(line("note", value, Tone::Normal));
    }
    if let Some(previous) = &app.installed
        && previous.format != plan.format
    {
        let value = format!("replaces the {} install, which will be removed", previous.format);
        lines.push(line("note", value, Tone::Warn));
    }
    lines
}

#[derive(Debug, Clone)]
pub struct SignatureCheck {
    pub signature: Signature,
    pub signed: Signed,
    pub source: Source,
}

/// How an app's install compares with its latest release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
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
/// `progress` may return `Break` to cancel (`Error::Cancelled`).
pub fn download(
    plan: &Plan,
    platform: &Platform,
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<Download> {
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
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<Installed> {
    let previous = store.get(id)?.installed.clone();
    let installed = install::install(
        &download.path,
        plan.format,
        store.get(id)?,
        &plan.release.tag,
        platform,
        // A package manager cannot be stopped halfway, so `Break` is ignored.
        &mut |percent| {
            let _ = progress(Progress::Install { percent });
        },
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

    fn plan(format: Format, expected_sha256: Option<String>) -> Plan {
        Plan {
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
            format,
            expected_sha256,
            from: None,
            missing: false,
            signature: None,
        }
    }

    #[test]
    fn checksum_mismatch_is_an_error() {
        let plan = plan(Format::Tarball, Some("a".repeat(64)));
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

    #[test]
    fn summary_flags_unverified_root_installs() {
        let mut app = App::new(
            Source::GitHub {
                owner: "o".into(),
                repo: "tool".into(),
            },
            "tool".into(),
        );
        app.installed = Some(Installed {
            tag: "v0".into(),
            format: Format::AppImage,
            asset: "tool.AppImage".into(),
            package: None,
            path: None,
            links: Vec::new(),
        });
        let plan = plan(Format::Rpm, None);
        assert!(plan.unverified());
        let lines = summary(&app, &plan);
        let tone = |label: &str, needle: &str| {
            lines
                .iter()
                .find(|l| l.label == label && l.value.contains(needle))
                .map(|l| l.tone)
        };
        assert_eq!(tone("sha256", "NOT PUBLISHED"), Some(Tone::Danger));
        assert_eq!(tone("note", "as root"), Some(Tone::Warn));
        assert_eq!(tone("note", "replaces the appimage"), Some(Tone::Warn));
        assert_eq!(tone("source", "github.com/o/tool"), Some(Tone::Normal));
    }
}
