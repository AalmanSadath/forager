//! How tracked apps compare with their latest release: which release counts
//! as latest, which installs the system has, and the state of each.

use serde::Serialize;

use crate::error::{Error, Result};
use crate::matcher::{self, Verdict, select_asset};
use crate::model::{App, Asset, Format, Installed, Release};
use crate::platform::Platform;
use crate::source::Source;
use crate::{install, version};

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
    /// The latest release is the one the user skipped, so installs older
    /// than it count as up to date.
    pub skipped: bool,
}

/// Releases of the GitHub apps among `apps`, by index into `apps`, fetched
/// together through GraphQL when a token allows it: one request for up to
/// 25 apps instead of one each. Apps missing from the result (no token,
/// another source, a version limit that needs a longer list, or a failed
/// request) are fetched one by one as before, with `status`.
pub fn prefetch(apps: &[App]) -> std::collections::HashMap<usize, Vec<Release>> {
    let repos: Vec<(usize, (String, String))> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, app)| match &app.source {
            Source::GitHub { owner, repo } if app.versions.is_none() => Some((i, (owner.clone(), repo.clone()))),
            _ => None,
        })
        .collect();
    // One app costs one request either way.
    if repos.len() < 2 {
        return Default::default();
    }
    let found = Source::github_batch(&repos.iter().map(|(_, repo)| repo.clone()).collect::<Vec<_>>());
    repos
        .into_iter()
        .filter_map(|(i, repo)| found.get(&repo).map(|releases| (i, releases.clone())))
        .collect()
}

/// Fetches the latest release and compares it with the installs.
pub fn status(app: &App, platform: &Platform) -> Result<Status> {
    status_in(app, app.releases()?, platform)
}

/// `status` against releases already fetched, newest first.
pub fn status_in(app: &App, releases: Vec<Release>, platform: &Platform) -> Result<Status> {
    let latest = latest(app, releases, platform)?;
    if app.track_only {
        return Ok(track_only_status(app, latest));
    }
    let observed = observe(app, &latest, platform);
    let found = (observed != app.install_list()).then(|| observed.clone());
    let skipped = app.skipped.as_deref() == Some(latest.tag.as_str());
    let installs: Vec<InstallState> = observed
        .into_iter()
        .map(|installed| {
            let state = match install::is_present(&installed) {
                true => compare(&installed.tag, &latest.tag, skipped),
                false => State::Missing,
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
        skipped,
    })
}

/// A tracked-only app has no installs: its latest release is new until the
/// user has seen it.
fn track_only_status(app: &App, latest: Release) -> Status {
    let skipped = app.skipped.as_deref() == Some(latest.tag.as_str());
    let new = app
        .seen
        .as_deref()
        .is_none_or(|seen| version::is_newer(seen, &latest.tag))
        && !skipped;
    Status {
        state: if new { State::Outdated } else { State::UpToDate },
        latest,
        installs: Vec::new(),
        found: None,
        skipped,
    }
}

/// A present install at `tag` against the latest release; older counts as
/// up to date when the user skipped the latest.
fn compare(tag: &str, latest: &str, skipped: bool) -> State {
    match version::is_newer(tag, latest) && !skipped {
        true => State::Outdated,
        false => State::UpToDate,
    }
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

/// The releases the app takes, newest first: tags matching its tag filter
/// and version limit, prereleases only if it follows them.
pub fn eligible(app: &App, releases: Vec<Release>) -> Result<Vec<Release>> {
    let tags = app.tag_filter.as_deref().map(regex::Regex::new).transpose()?;
    let limit = app.versions.as_deref().map(version::requirement).transpose()?;
    Ok(releases
        .into_iter()
        .filter(|r| app.prerelease || !r.prerelease)
        .filter(|r| tags.as_ref().is_none_or(|re| re.is_match(&r.tag)))
        .filter(|r| limit.as_ref().is_none_or(|limit| version::within(limit, &r.tag)))
        .collect())
}

/// The newest release the app takes (its tag matching the app's tag filter,
/// no prereleases unless it follows them) with an asset for this system. Projects that also publish other builds
/// from the same repository, e.g. `android-v1.0` next to `v2.3`, would
/// otherwise have their newest release be one with nothing to install.
/// Falls back to the newest release, so the reason it has nothing shows.
pub fn latest(app: &App, releases: Vec<Release>, platform: &Platform) -> Result<Release> {
    let mut eligible = eligible(app, releases)?.into_iter().peekable();
    let newest = eligible.peek().cloned().ok_or_else(|| match &app.versions {
        Some(limit) => Error::NoReleaseWithin(limit.clone()),
        None => Error::NoRelease,
    })?;
    if app.track_only {
        // Nothing is installed, so the release need not have an asset.
        return Ok(newest);
    }
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
pub fn asset_in<'a>(app: &App, release: &'a Release, platform: &Platform, format: Format) -> Option<&'a Asset> {
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

pub(super) fn same_version(a: &str, b: &str) -> bool {
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
        desktop: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::fixtures::{install, plan};
    use crate::update::{plans, plans_for};

    #[test]
    fn tracked_only_apps_report_releases_they_have_not_seen() {
        let release = |tag: &str, prerelease| Release {
            tag: tag.into(),
            name: None,
            prerelease,
            published_at: None,
            notes: None,
            // An Android build only: nothing for this system.
            assets: vec![Asset {
                name: "tool.apk".into(),
                url: String::new(),
                size: 0,
                digest: None,
            }],
        };
        let platform = Platform {
            arch: "x86_64",
            package_manager: None,
            flatpak: false,
        };
        let mut app = App::new(
            Source::GitHub {
                owner: "o".into(),
                repo: "tool".into(),
            },
            "tool".into(),
        );
        app.track_only = true;
        let releases = vec![
            release("v2.1-rc1", true),
            release("v2.0", false),
            release("v1.9", false),
        ];
        let latest = latest(&app, releases, &platform).unwrap();
        assert_eq!(latest.tag, "v2.0");

        assert_eq!(track_only_status(&app, latest.clone()).state, State::Outdated);
        app.seen = Some("v1.9".into());
        assert_eq!(track_only_status(&app, latest.clone()).state, State::Outdated);
        app.seen = Some("v2.0".into());
        let status = track_only_status(&app, latest.clone());
        assert_eq!(status.state, State::UpToDate);
        assert!(status.installs.is_empty() && status.found.is_none());
        app.seen = Some("v1.9".into());
        app.skipped = Some("v2.0".into());
        assert_eq!(track_only_status(&app, latest).state, State::UpToDate);

        assert!(plans(&mut app, &platform).unwrap().is_empty());
        assert!(matches!(
            plans_for(&mut app, &platform, "v1.9"),
            Err(Error::TrackOnly(_))
        ));
    }

    #[test]
    fn a_skipped_release_is_not_an_update() {
        assert_eq!(compare("v1.0", "v1.1", false), State::Outdated);
        assert_eq!(compare("v1.0", "v1.1", true), State::UpToDate);
        assert_eq!(compare("v1.1", "v1.1", false), State::UpToDate);
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
            desktop: Vec::new(),
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
            notes: None,
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
        // A version limit leaves out the 2.1 prerelease.
        app.versions = Some("<2.1".into());
        assert_eq!(latest(&app, releases.clone(), &platform).unwrap().tag, "v2.0.0");
        app.versions = None;
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
}
