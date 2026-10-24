//! Update flow shared by every frontend: work out what would be installed,
//! then download, verify and install it.

use std::fs;
use std::ops::ControlFlow;
use std::path::PathBuf;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::matcher::{self, Verdict, select_asset};
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
    /// Installs in other formats that this one replaces, removed once it is
    /// in place: the app was pinned to another format.
    pub replaces: Vec<Installed>,
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
    for previous in &plan.replaces {
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

/// One install of an app and how it compares with the latest release.
#[derive(Debug, Clone)]
pub struct InstallState {
    pub installed: Installed,
    /// `Missing`, `Outdated` or `UpToDate`.
    pub state: State,
}

#[derive(Debug, Clone)]
pub struct Status {
    /// The app as a whole: not installed, installed in the wrong format, or
    /// the worst of its installs.
    pub state: State,
    pub latest: Release,
    /// Every install, as the system reports them, the preferred one first.
    pub installs: Vec<InstallState>,
    /// The installs as the system reports them, when they differ from the
    /// record: one made without Forager (by hand, before the app was
    /// tracked, or in another format next to it), one removed while another
    /// remains, or one upgraded or downgraded outside Forager. Already
    /// counted in `state`; callers save it with `App::set_installs`.
    pub found: Option<Vec<Installed>>,
}

/// Fetches the latest release and compares it with the installs.
pub fn status(app: &App) -> Result<Status> {
    status_in(app, app.source.releases()?)
}

/// `status` against releases already fetched, newest first.
pub fn status_in(app: &App, releases: Vec<Release>) -> Result<Status> {
    let platform = Platform::detect();
    let latest = latest(app, releases, &platform)?;
    let observed = observe(app, &latest, &platform);
    let found = (observed != app.install_list()).then(|| observed.clone());
    let installs: Vec<InstallState> = observed
        .into_iter()
        .map(|installed| {
            let state = if !install::is_present(&installed) {
                State::Missing
            } else if version::is_newer(&installed.tag, &latest.tag) {
                State::Outdated
            } else {
                State::UpToDate
            };
            InstallState { installed, state }
        })
        .collect();
    let state = overall(app.format, &installs);
    Ok(Status {
        state,
        latest,
        installs,
        found,
    })
}

fn overall(pinned: Option<Format>, installs: &[InstallState]) -> State {
    let has = |state| installs.iter().any(|i| i.state == state);
    if installs.is_empty() {
        State::NotInstalled
    } else if pinned.is_some_and(|f| !installs.iter().any(|i| i.installed.format == f)) {
        State::WrongFormat
    } else if has(State::Missing) {
        State::Missing
    } else if has(State::Outdated) {
        State::Outdated
    } else {
        State::UpToDate
    }
}

/// The newest release the app takes (its tag matching the app's tag filter,
/// no prereleases unless it follows them) with an asset for this system. Projects that also publish other builds
/// from the same repository, e.g. `android-v1.0` next to `v2.3`, would
/// otherwise have their newest release be one with nothing to install.
/// Falls back to the newest release, so the reason it has nothing shows.
pub fn latest(app: &App, releases: Vec<Release>, platform: &Platform) -> Result<Release> {
    let tags = app.tag_filter.as_deref().map(regex::Regex::new).transpose()?;
    let mut eligible = releases
        .into_iter()
        .filter(|r| app.prerelease || !r.prerelease)
        .filter(|r| tags.as_ref().is_none_or(|re| re.is_match(&r.tag)))
        .peekable();
    let newest = eligible.peek().cloned().ok_or(Error::NoRelease)?;
    Ok(eligible
        .find(|r| select_asset(app, r, platform).is_ok())
        .unwrap_or(newest))
}

/// The installs as the system has them: the recorded ones still present,
/// at the version the package manager reports, and any found in other
/// formats that suit the system. When none is present the record is kept,
/// so the app shows as missing.
fn observe(app: &App, release: &Release, platform: &Platform) -> Vec<Installed> {
    let present = app
        .installs()
        .filter(|recorded| install::is_present(recorded))
        .map(|recorded| current(app, recorded, release, platform).unwrap_or_else(|| recorded.clone()))
        .collect();
    let found = find_existing(app, release, platform);
    combine(app.install_list(), present, found, &preference(app.format, platform))
}

/// Formats in order of preference: the pinned one, then the system's ranking.
fn preference(pinned: Option<Format>, platform: &Platform) -> Vec<Format> {
    pinned.into_iter().chain(platform.formats()).collect()
}

/// Puts installs in order of `preference`, the preferred one first.
pub fn sort_installs(installs: &mut [Installed], pinned: Option<Format>, platform: &Platform) {
    sort_by(installs, &preference(pinned, platform));
}

fn sort_by(installs: &mut [Installed], preference: &[Format]) {
    installs.sort_by_key(|i| preference.iter().position(|f| *f == i.format).unwrap_or(usize::MAX));
}

fn combine(
    recorded: Vec<Installed>,
    mut present: Vec<Installed>,
    found: Vec<Installed>,
    preference: &[Format],
) -> Vec<Installed> {
    for found in found {
        if !present.iter().any(|i| i.format == found.format) {
            present.push(found);
        }
    }
    if present.is_empty() {
        return recorded;
    }
    sort_by(&mut present, preference);
    present
}

/// A recorded system package or Flatpak at the version installed now, if
/// that is another one.
fn current(app: &App, recorded: &Installed, release: &Release, platform: &Platform) -> Option<Installed> {
    let package = recorded.package.as_deref()?;
    let actual = match recorded.format {
        Format::Flatpak => install::flatpak_version(package)?,
        format if format.needs_root() => install::installed_version(package, format)?,
        // Forager placed AppImages and tarballs itself; nothing else reports their version.
        _ => return None,
    };
    refreshed(
        recorded,
        actual,
        release,
        asset_in(app, release, platform, recorded.format),
    )
}

/// The asset an update in `format` would install.
fn asset_in<'a>(app: &App, release: &'a Release, platform: &Platform, format: Format) -> Option<&'a Asset> {
    let mut in_format = app.clone();
    in_format.format = Some(format);
    select_asset(&in_format, release, platform).ok().map(|(asset, _)| asset)
}

/// The recorded install at the version actually installed, if that is
/// another one.
fn refreshed(recorded: &Installed, actual: String, release: &Release, asset: Option<&Asset>) -> Option<Installed> {
    if same_version(&actual, &recorded.tag) {
        return None;
    }
    let (tag, asset) = as_release(actual, release, asset);
    Some(Installed {
        tag,
        asset,
        ..recorded.clone()
    })
}

fn same_version(a: &str, b: &str) -> bool {
    match (version::parse_tag(a), version::parse_tag(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

/// A version reported by a package manager as tag and asset: the release's
/// own when it is that release, so it reads like an install Forager made.
fn as_release(version: String, release: &Release, asset: Option<&Asset>) -> (String, String) {
    if same_version(&version, &release.tag) {
        (release.tag.clone(), asset.map_or_else(String::new, |a| a.name.clone()))
    } else {
        (version, String::new())
    }
}

/// System packages and Flatpaks of the app, installed with or without
/// Forager, looked for among every asset of the release that suits this
/// system: at most one per format.
fn find_existing(app: &App, release: &Release, platform: &Platform) -> Vec<Installed> {
    let Ok(candidates) = matcher::candidates(app, release, platform) else {
        return Vec::new();
    };
    let mut usable: Vec<_> = candidates
        .into_iter()
        .filter(|c| !matches!(c.verdict, Verdict::Rejected(_)))
        .collect();
    usable.sort_by_key(|c| c.verdict != Verdict::Chosen);
    let mut found: Vec<Installed> = Vec::new();
    for candidate in usable {
        let Some(format) = candidate.format else { continue };
        if found.iter().any(|i| i.format == format) {
            continue;
        }
        let names = install::package_guesses(&candidate.asset.name, format, &app.id);
        let existing = match format {
            Format::Flatpak => install::find_flatpak(&names).map(|(id, version)| {
                // Flatpaks need not declare a version; an unknown one counts
                // as older than any release.
                (id, if version.is_empty() { "unknown".into() } else { version })
            }),
            format if format.needs_root() => names
                .into_iter()
                .find_map(|name| Some((name.clone(), install::installed_version(&name, format)?))),
            _ => None,
        };
        if let Some((package, version)) = existing {
            found.push(existing_install(version, package, format, candidate.asset, release));
        }
    }
    found
}

fn existing_install(version: String, package: String, format: Format, asset: &Asset, release: &Release) -> Installed {
    let (tag, asset) = as_release(version, release, Some(asset));
    Installed {
        tag,
        format,
        asset,
        package: Some(package),
        path: None,
        links: Vec::new(),
    }
}

/// What updating `app` takes: nothing when it is up to date, else one plan
/// per install that is out of date or missing, each in its own format; one
/// in the chosen format when it is not installed, or in the pinned format
/// when no install has it. A plan that cannot be made (a format the release
/// no longer offers) is an error of its own, so the others still go ahead.
///
/// Installs found on the system or changed outside Forager become the
/// app's (see `Status::found`), so the caller should save `app` when its
/// installs changed.
pub fn plans(app: &mut App, platform: &Platform) -> Result<Vec<Result<Plan>>> {
    let status = status(app)?;
    if let Some(found) = status.found {
        app.set_installs(found);
    }
    let release = status.latest;
    let app = &*app;
    let make = |format: Option<Format>, current: Option<&InstallState>, replaces: Vec<Installed>| {
        plan_one(app, &release, platform, format, current, replaces)
    };
    Ok(match status.state {
        State::UpToDate => Vec::new(),
        State::NotInstalled => vec![make(None, None, Vec::new())],
        State::WrongFormat => vec![make(app.format, None, app.install_list())],
        _ => status
            .installs
            .iter()
            .filter(|i| i.state.needs_update())
            .map(|i| make(Some(i.installed.format), Some(i), Vec::new()))
            .collect(),
    })
}

fn plan_one(
    app: &App,
    release: &Release,
    platform: &Platform,
    format: Option<Format>,
    current: Option<&InstallState>,
    replaces: Vec<Installed>,
) -> Result<Plan> {
    let mut target = app.clone();
    if format.is_some() {
        target.format = format;
    }
    let (asset, format) = select_asset(&target, release, platform)?;
    let asset = asset.clone();
    let expected_sha256 = verify::expected_sha256(&asset, release)?;
    let signature = match &app.signature {
        Some(signature) => Some(SignatureCheck {
            signed: signature::find(signature, &asset, release)?,
            signature: signature.clone(),
            source: app.source.clone(),
        }),
        None => None,
    };
    Ok(Plan {
        from: current
            .map(|i| i.installed.tag.clone())
            .or_else(|| replaces.first().map(|i| i.tag.clone())),
        release: release.clone(),
        asset,
        format,
        expected_sha256,
        missing: current.is_some_and(|i| i.state == State::Missing),
        signature,
        replaces,
    })
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
    let app = store.get_mut(id)?;
    let mut installs: Vec<Installed> = app.installs().filter(|i| i.format != plan.format).cloned().collect();
    installs.push(installed.clone());
    sort_installs(&mut installs, app.format, platform);
    app.set_installs(installs);
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
            replaces: Vec::new(),
        }
    }

    fn install(format: Format, tag: &str) -> Installed {
        Installed {
            tag: tag.into(),
            format,
            asset: String::new(),
            package: Some("tool".into()),
            path: None,
            links: Vec::new(),
        }
    }

    #[test]
    fn combines_recorded_and_found_installs() {
        let order = [Format::Rpm, Format::Flatpak, Format::AppImage];
        let rpm = install(Format::Rpm, "v1.0.0");
        let flatpak = install(Format::Flatpak, "1.0.0");
        // A Flatpak next to the recorded rpm is added after it.
        let both = combine(
            vec![rpm.clone()],
            vec![rpm.clone()],
            vec![flatpak.clone(), rpm.clone()],
            &order,
        );
        assert_eq!(both, [rpm.clone(), flatpak.clone()]);
        // Found in preference order even when the Flatpak was recorded first.
        let sorted = combine(vec![flatpak.clone()], vec![flatpak.clone()], vec![rpm.clone()], &order);
        assert_eq!(sorted, [rpm.clone(), flatpak.clone()]);
        // The recorded rpm was removed but the Flatpak remains: follow it.
        let switched = combine(vec![rpm.clone()], Vec::new(), vec![flatpak.clone()], &order);
        assert_eq!(switched, [flatpak]);
        // Nothing left: keep the record, so the app shows as missing.
        assert_eq!(combine(vec![rpm.clone()], Vec::new(), Vec::new(), &order), [rpm]);
    }

    #[test]
    fn overall_state_is_the_worst_install() {
        let state = |installed: Installed, state| InstallState { installed, state };
        let rpm = || install(Format::Rpm, "v1");
        let flatpak = || install(Format::Flatpak, "v1");
        assert_eq!(overall(None, &[]), State::NotInstalled);
        let mixed = [state(rpm(), State::UpToDate), state(flatpak(), State::Outdated)];
        assert_eq!(overall(None, &mixed), State::Outdated);
        let fine = [state(rpm(), State::UpToDate), state(flatpak(), State::UpToDate)];
        assert_eq!(overall(None, &fine), State::UpToDate);
        assert_eq!(overall(Some(Format::Flatpak), &fine), State::UpToDate);
        assert_eq!(overall(Some(Format::AppImage), &fine), State::WrongFormat);
        let gone = [state(rpm(), State::Missing)];
        assert_eq!(overall(None, &gone), State::Missing);
    }

    #[test]
    fn refreshes_a_version_changed_outside_forager() {
        let plan = plan(Format::Rpm, None);
        let release = Release {
            tag: "v0.1.1".into(),
            ..plan.release
        };
        let recorded = Installed {
            tag: "v0.1.0".into(),
            format: Format::Rpm,
            asset: "tool-0.1.0-1.x86_64.rpm".into(),
            package: Some("tool".into()),
            path: None,
            links: Vec::new(),
        };
        assert!(refreshed(&recorded, "0.1.0".into(), &release, Some(&plan.asset)).is_none());
        let upgraded = refreshed(&recorded, "0.1.1".into(), &release, Some(&plan.asset)).unwrap();
        assert_eq!(
            (upgraded.tag.as_str(), upgraded.asset.as_str()),
            ("v0.1.1", "tool.tar.gz")
        );
        assert_eq!(upgraded.package.as_deref(), Some("tool"));
        assert_eq!(upgraded.format, Format::Rpm);
        let other = refreshed(&recorded, "0.0.9".into(), &release, Some(&plan.asset)).unwrap();
        assert_eq!((other.tag.as_str(), other.asset.as_str()), ("0.0.9", ""));
    }

    #[test]
    fn latest_skips_releases_with_nothing_for_this_system() {
        let release = |tag: &str, prerelease: bool, assets: &[&str]| Release {
            tag: tag.into(),
            name: None,
            prerelease,
            published_at: None,
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: name.to_string(),
                    url: String::new(),
                    size: 0,
                    digest: None,
                })
                .collect(),
        };
        let releases = vec![
            release("v2.1.0-rc.1", true, &["tool-2.1.0-1.x86_64.rpm"]),
            release("android-v0.3.0", false, &["tool.apk"]),
            release("v2.0.0", false, &["tool-2.0.0-1.x86_64.rpm", "tool-setup.exe"]),
        ];
        let platform = Platform {
            arch: "x86_64",
            package_manager: Some(crate::platform::PackageManager::Dnf),
            flatpak: false,
        };
        let source = Source::GitHub {
            owner: "o".into(),
            repo: "tool".into(),
        };
        let mut app = App::new(source, "tool".into());
        assert_eq!(latest(&app, releases.clone(), &platform).unwrap().tag, "v2.0.0");
        app.prerelease = true;
        assert_eq!(latest(&app, releases.clone(), &platform).unwrap().tag, "v2.1.0-rc.1");
        // Following only `android-…` tags finds nothing to install, but still names that release.
        app.tag_filter = Some("^android-".into());
        assert_eq!(latest(&app, releases.clone(), &platform).unwrap().tag, "android-v0.3.0");
        app.tag_filter = Some("^v".into());
        assert_eq!(latest(&app, releases.clone(), &platform).unwrap().tag, "v2.1.0-rc.1");
        app.tag_filter = Some("^nothing".into());
        assert!(matches!(
            latest(&app, releases.clone(), &platform),
            Err(Error::NoRelease)
        ));
        app.tag_filter = None;
        // Nothing fits at all: the newest release, to show why.
        app.prerelease = false;
        app.asset_filter = Some("nothing".into());
        assert_eq!(latest(&app, releases, &platform).unwrap().tag, "android-v0.3.0");
    }

    #[test]
    fn found_package_reads_like_foragers_own_install_when_current() {
        let plan = plan(Format::Rpm, None);
        let release = Release {
            tag: "v1.0.0".into(),
            ..plan.release
        };
        let current = existing_install("1.0.0".into(), "tool".into(), Format::Rpm, &plan.asset, &release);
        assert_eq!(
            (current.tag.as_str(), current.asset.as_str()),
            ("v1.0.0", "tool.tar.gz")
        );
        assert_eq!(current.package.as_deref(), Some("tool"));
        let older = existing_install("0.9.1".into(), "tool".into(), Format::Rpm, &plan.asset, &release);
        assert_eq!((older.tag.as_str(), older.asset.as_str()), ("0.9.1", ""));
        assert!(version::is_newer(&older.tag, &release.tag));
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
        let app = App::new(
            Source::GitHub {
                owner: "o".into(),
                repo: "tool".into(),
            },
            "tool".into(),
        );
        let mut plan = plan(Format::Rpm, None);
        plan.replaces = vec![Installed {
            tag: "v0".into(),
            format: Format::AppImage,
            asset: "tool.AppImage".into(),
            package: None,
            path: None,
            links: Vec::new(),
        }];
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
