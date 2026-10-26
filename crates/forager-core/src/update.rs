//! Update flow shared by every frontend: work out what would be installed,
//! then download, verify and install it.

use std::fs;
use std::ops::ControlFlow;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::matcher::{self, Verdict, select_asset};
use crate::model::{App, Asset, Format, Installed, Release, Signature};
use crate::platform::Platform;
use crate::signature::{self, Signed};
use crate::source::Source;
use crate::store::{self, Store};
use crate::{install, verify, version, zsync};

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
    /// The release is older than the install it replaces.
    pub downgrade: bool,
    /// Release to mark as skipped once this is installed: rolling back to an
    /// older release skips the latest, so it is not offered again at once.
    pub skip_after: Option<String>,
    /// Fetch only what changed since the installed AppImage, using the
    /// release's zsync control file.
    pub delta: Option<Delta>,
    /// The releases this update brings, newest first: the target and those
    /// between it and the installed one, for their notes.
    pub changes: Vec<Release>,
    /// The installed file's name when the new one differs from it by more
    /// than the version (another architecture, libc or format), so the
    /// user is asked before it is installed; see `asset_shape`.
    pub asset_changed: Option<String>,
}

/// An installed file and the control file to update it from; see `zsync`.
#[derive(Debug, Clone)]
pub struct Delta {
    pub control: Asset,
    pub seed: PathBuf,
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
    if let Some(previous) = &plan.asset_changed {
        let value = format!("changed from {previous}; check this is the build you want");
        lines.push(line("file", value, Tone::Warn));
    }
    if plan.delta.is_some() {
        let from = plan.from.as_deref().unwrap_or("the installed version");
        let value = format!("only the parts changed since {from} are downloaded (zsync)");
        lines.push(line("delta", value, Tone::Normal));
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
pub fn status(app: &App) -> Result<Status> {
    status_in(app, app.releases()?)
}

/// `status` against releases already fetched, newest first.
pub fn status_in(app: &App, releases: Vec<Release>) -> Result<Status> {
    let platform = Platform::detect();
    let latest = latest(app, releases, &platform)?;
    if app.track_only {
        return Ok(track_only_status(app, latest));
    }
    let observed = observe(app, &latest, &platform);
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
        desktop: Vec::new(),
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
    if app.track_only {
        return Ok(Vec::new());
    }
    let releases = app.releases()?;
    let status = status_in(app, releases.clone())?;
    if let Some(found) = status.found {
        app.set_installs(found);
    }
    let release = status.latest;
    let app = &*app;
    let make = |format: Option<Format>, current: Option<&InstallState>, replaces: Vec<Installed>| {
        plan_one(app, &release, platform, format, current, replaces, None)
            .map(|plan| with_changes(app, plan, &releases))
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

/// Plans installing the release tagged `tag` instead of the latest: every
/// install moves to it in its own format (one install in the chosen format
/// when there is none). Going back from the latest release skips it.
pub fn plans_for(app: &mut App, platform: &Platform, tag: &str) -> Result<Vec<Result<Plan>>> {
    if app.track_only {
        return Err(Error::TrackOnly(app.id.clone()));
    }
    let releases = app.releases()?;
    let target = releases
        .iter()
        .find(|r| r.tag == tag)
        .cloned()
        .ok_or_else(|| Error::ReleaseNotFound(tag.to_string()))?;
    let status = status_in(app, releases.clone())?;
    if let Some(found) = status.found {
        app.set_installs(found);
    }
    let app = &*app;
    let skip_after = version::is_newer(&target.tag, &status.latest.tag).then(|| status.latest.tag.clone());
    let make = |format: Option<Format>, current: Option<&InstallState>| {
        plan_one(app, &target, platform, format, current, Vec::new(), skip_after.clone())
            .map(|plan| with_changes(app, plan, &releases))
    };
    Ok(match status.installs.as_slice() {
        [] => vec![make(None, None)],
        installs => installs
            .iter()
            .map(|i| make(Some(i.installed.format), Some(i)))
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
    skip_after: Option<String>,
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
    let delta = current.and_then(|i| delta_for(&i.installed, i.state, &asset, format, release));
    let asset_changed = current
        .map(|i| &i.installed)
        .filter(|i| !i.asset.is_empty() && asset_shape(&i.asset, &i.tag) != asset_shape(&asset.name, &release.tag))
        .map(|i| i.asset.clone());
    Ok(Plan {
        asset_changed,
        delta,
        from: current
            .map(|i| i.installed.tag.clone())
            .or_else(|| replaces.first().map(|i| i.tag.clone())),
        release: release.clone(),
        asset,
        format,
        expected_sha256,
        missing: current.is_some_and(|i| i.state == State::Missing),
        signature,
        downgrade: current.is_some_and(|i| version::is_newer(&release.tag, &i.installed.tag)),
        replaces,
        skip_after,
        changes: vec![release.clone()],
    })
}

/// `name` with its versions blanked out, so release files of the same
/// build compare equal from one release to the next: the tag's version and
/// any dotted version become `#`, as do long numbers (build ids, dates).
/// Architecture, libc and extension stay, so `x86_64` becoming `amd64`, or
/// `gnu` becoming `musl`, is a change.
pub fn asset_shape(name: &str, tag: &str) -> String {
    static DOTTED: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\d+(\.\d+)+|\d{6,}").unwrap());
    let name = name.to_ascii_lowercase();
    let mut shape = DOTTED.replace_all(&name, "#").into_owned();
    // A version without dots (`v32`) only where it stands alone; `_`
    // joins words, so the 64 of `x86_64` is not one.
    let in_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let number = tag.trim_start_matches(|c: char| !c.is_ascii_digit());
    if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
        let parts: Vec<&str> = shape.split(|c: char| !in_word(c)).collect();
        if parts.contains(&number) {
            let mut out = String::new();
            let mut word = String::new();
            for c in shape.chars().chain([' ']) {
                if in_word(c) {
                    word.push(c);
                    continue;
                }
                out.push_str(if word == number { "#" } else { &word });
                word.clear();
                out.push(c);
            }
            out.pop();
            shape = out;
        }
    }
    shape
}

fn with_changes(app: &App, mut plan: Plan, releases: &[Release]) -> Plan {
    plan.changes = changes(app, releases.to_vec(), plan.from.as_deref(), &plan.release);
    plan
}

/// Most releases whose notes `changes` gathers, so an app installed long
/// ago does not print pages of them.
const MAX_CHANGES: usize = 10;

/// The releases `app` takes that are newer than `from` up to and including
/// `to`, newest first; just `to` when nothing is installed. A downgrade
/// brings no new releases, so it is `to` alone too.
pub fn changes(app: &App, releases: Vec<Release>, from: Option<&str>, to: &Release) -> Vec<Release> {
    let Some(from) = from.filter(|from| version::is_newer(from, &to.tag)) else {
        return vec![to.clone()];
    };
    let mut changes: Vec<Release> = eligible(app, releases)
        .unwrap_or_default()
        .into_iter()
        .skip_while(|r| r.tag != to.tag)
        // Moving tags such as `stable` or `nightly` have no place between
        // two versions.
        .filter(|r| r.tag == to.tag || version::parse_tag(&r.tag).is_some() || version::parse_tag(from).is_none())
        .take_while(|r| r.tag != from && version::is_newer(from, &r.tag))
        .take(MAX_CHANGES)
        .collect();
    if changes.is_empty() {
        changes.push(to.clone());
    }
    changes
}

/// The notes of `releases` as Markdown: one release's notes as they are,
/// several each under a heading with its tag.
pub fn combined_notes(releases: &[Release]) -> Option<String> {
    match releases {
        [] => None,
        [one] => one.notes.clone(),
        several => {
            let parts: Vec<String> = several
                .iter()
                .map(|r| {
                    let title = match r.name.as_deref().filter(|n| *n != r.tag) {
                        Some(name) => format!("{} ({name})", r.tag),
                        None => r.tag.clone(),
                    };
                    let notes = r.notes.as_deref().unwrap_or("_No release notes._");
                    format!("## {title}\n\n{notes}")
                })
                .collect();
            Some(parts.join("\n\n"))
        }
    }
}

/// Until when background updates leave `release` alone because of the
/// app's cooldown, as ISO 8601; `None` once it is old enough, or when it
/// has no date to wait on. `now` is seconds since the Unix epoch.
pub fn cooldown_until(app: &App, release: &Release, now: u64) -> Option<String> {
    if app.cooldown_days == 0 {
        return None;
    }
    let published = release.published_at.as_deref().and_then(crate::source::unix_from_iso)?;
    let ready = published + u64::from(app.cooldown_days) * 86400;
    (ready > now).then(|| crate::source::iso_from_unix(ready))
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A delta update for `asset` from `installed`: both AppImages, the old one
/// still in place, and the release has `<asset>.zsync`.
fn delta_for(installed: &Installed, state: State, asset: &Asset, format: Format, release: &Release) -> Option<Delta> {
    if format != Format::AppImage || installed.format != Format::AppImage || state == State::Missing {
        return None;
    }
    let seed = installed.path.clone().filter(|p| p.is_file())?;
    let name = format!("{}.zsync", asset.name);
    let control = release.assets.iter().find(|a| a.name == name)?.clone();
    Some(Delta { control, seed })
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
    /// How the delta download went, when the plan had one.
    pub delta: Option<DeltaOutcome>,
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
    }
}

/// Downloads the plan's asset and checks it against the expected SHA-256.
/// `progress` may return `Break` to cancel (`Error::Cancelled`).
pub fn download(
    plan: &Plan,
    platform: &Platform,
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<Download> {
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
        plan.downgrade,
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

    fn plan(format: Format, expected_sha256: Option<String>) -> Plan {
        Plan {
            release: Release {
                tag: "v1".into(),
                name: None,
                prerelease: false,
                published_at: None,
                notes: None,
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
            downgrade: false,
            delta: None,
            changes: Vec::new(),
            skip_after: None,
            asset_changed: None,
        }
    }

    #[test]
    fn version_bumps_keep_the_asset_shape() {
        let same = [
            (
                "nvim-linux-x86_64.appimage",
                "v0.12.5",
                "nvim-linux-x86_64.appimage",
                "v0.12.6",
            ),
            (
                "drawio-x86_64-28.2.5.rpm",
                "v28.2.5",
                "drawio-x86_64-29.0.1.rpm",
                "v29.0.1",
            ),
            (
                "fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz",
                "v10.2.0",
                "fd-v10.5.0-x86_64-unknown-linux-gnu.tar.gz",
                "v10.5.0",
            ),
            ("tool-31-x86_64.AppImage", "v31", "tool-32-x86_64.AppImage", "v32"),
            (
                "tool-20260101-x86_64.tar.gz",
                "nightly",
                "tool-20261026-x86_64.tar.gz",
                "nightly",
            ),
            ("tool-1.0.0-1.x86_64.rpm", "1.0.0", "tool-1.1.0-1.x86_64.rpm", "1.1.0"),
        ];
        for (old, old_tag, new, new_tag) in same {
            assert_eq!(asset_shape(old, old_tag), asset_shape(new, new_tag), "{old} -> {new}");
        }
    }

    #[test]
    fn other_builds_change_the_asset_shape() {
        let changed = [
            (
                "nvim-linux64.appimage",
                "v0.10.4",
                "nvim-linux-x86_64.appimage",
                "v0.12.6",
            ),
            ("tool_1.0_x86_64.deb", "v1.0", "tool_1.1_amd64.deb", "v1.1"),
            (
                "tool-1.0-x86_64-linux-gnu.tar.gz",
                "1.0",
                "tool-1.1-x86_64-linux-musl.tar.gz",
                "1.1",
            ),
            ("Tool-1.0.AppImage", "1.0", "Tool-1.1.tar.gz", "1.1"),
        ];
        for (old, old_tag, new, new_tag) in changed {
            assert_ne!(asset_shape(old, old_tag), asset_shape(new, new_tag), "{old} -> {new}");
        }
        // A bare number is only a version where it stands alone.
        assert_eq!(asset_shape("tool-x86_64.AppImage", "v64"), "tool-x86_64.appimage");
    }

    fn install(format: Format, tag: &str) -> Installed {
        Installed {
            tag: tag.into(),
            format,
            asset: String::new(),
            package: Some("tool".into()),
            path: None,
            links: Vec::new(),
            desktop: Vec::new(),
        }
    }

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
    fn rolling_back_downgrades_and_skips_the_latest() {
        let release = Release {
            tag: "v1.0.0".into(),
            name: None,
            prerelease: false,
            published_at: None,
            notes: None,
            assets: vec![Asset {
                name: "tool-1.0.0-1.x86_64.rpm".into(),
                url: String::new(),
                size: 0,
                digest: None,
            }],
        };
        let platform = Platform {
            arch: "x86_64",
            package_manager: Some(crate::platform::PackageManager::Dnf),
            flatpak: false,
        };
        let app = App::new(
            Source::GitHub {
                owner: "o".into(),
                repo: "tool".into(),
            },
            "tool".into(),
        );
        let current = InstallState {
            installed: install(Format::Rpm, "v1.1.0"),
            state: State::UpToDate,
        };
        let plan = plan_one(
            &app,
            &release,
            &platform,
            Some(Format::Rpm),
            Some(&current),
            Vec::new(),
            Some("v1.1.0".into()),
        )
        .unwrap();
        assert!(plan.downgrade);
        assert_eq!(plan.from.as_deref(), Some("v1.1.0"));
        assert_eq!(plan.skip_after.as_deref(), Some("v1.1.0"));
        let fresh = plan_one(&app, &release, &platform, None, None, Vec::new(), None).unwrap();
        assert!(!fresh.downgrade);
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

    #[test]
    fn gathers_the_releases_since_the_installed_one() {
        let release = |tag: &str, prerelease: bool| Release {
            tag: tag.into(),
            name: None,
            prerelease,
            published_at: None,
            notes: Some(format!("notes of {tag}")),
            assets: Vec::new(),
        };
        let releases = vec![
            release("v1.4.0", false),
            release("v1.3.0", false),
            release("v1.3.0-rc.1", true),
            release("android-v9", false),
            release("vstable", false),
            release("v1.2.0", false),
            release("v1.1.0", false),
        ];
        let source = Source::GitHub {
            owner: "o".into(),
            repo: "tool".into(),
        };
        let mut app = App::new(source, "tool".into());
        app.tag_filter = Some("^v".into());
        let tags = |changes: Vec<Release>| changes.into_iter().map(|r| r.tag).collect::<Vec<_>>();
        // Up to the target, past the installed one, without prereleases or other tags.
        assert_eq!(
            tags(changes(&app, releases.clone(), Some("v1.1.0"), &releases[1])),
            ["v1.3.0", "v1.2.0"]
        );
        assert_eq!(tags(changes(&app, releases.clone(), None, &releases[0])), ["v1.4.0"]);
        // A rollback brings nothing new.
        assert_eq!(
            tags(changes(&app, releases.clone(), Some("v1.4.0"), &releases[5])),
            ["v1.2.0"]
        );

        let notes = combined_notes(&changes(&app, releases.clone(), Some("v1.2.0"), &releases[0])).unwrap();
        assert_eq!(notes, "## v1.4.0\n\nnotes of v1.4.0\n\n## v1.3.0\n\nnotes of v1.3.0");
        assert_eq!(combined_notes(&releases[..1]).as_deref(), Some("notes of v1.4.0"));
        assert_eq!(combined_notes(&[]), None);
    }

    #[test]
    fn cooldown_holds_fresh_releases_back() {
        let source = Source::GitHub {
            owner: "o".into(),
            repo: "tool".into(),
        };
        let mut app = App::new(source, "tool".into());
        let mut release = plan(Format::AppImage, None).release;
        release.published_at = Some("2026-10-25T00:00:00Z".into());
        let day = 86400;
        let published = crate::source::unix_from_iso("2026-10-25T00:00:00Z").unwrap();
        assert_eq!(cooldown_until(&app, &release, published), None);
        app.cooldown_days = 3;
        assert_eq!(
            cooldown_until(&app, &release, published + day).as_deref(),
            Some("2026-10-28T00:00:00Z")
        );
        assert_eq!(cooldown_until(&app, &release, published + 3 * day), None);
        release.published_at = None;
        assert_eq!(cooldown_until(&app, &release, published), None);
    }

    #[test]
    fn delta_needs_an_installed_appimage_and_a_control_file() {
        let tmp = tempfile::tempdir().unwrap();
        let seed = tmp.path().join("tool.AppImage");
        fs::write(&seed, b"old").unwrap();
        let asset = |name: &str| Asset {
            name: name.into(),
            url: format!("https://example.org/{name}"),
            size: 0,
            digest: None,
        };
        let new = asset("Tool-2-x86_64.AppImage");
        let mut release = Release {
            tag: "v2".into(),
            name: None,
            prerelease: false,
            published_at: None,
            notes: None,
            assets: vec![new.clone(), asset("Tool-2-x86_64.AppImage.zsync")],
        };
        let mut installed = install(Format::AppImage, "v1");
        installed.path = Some(seed.clone());
        let delta = delta_for(&installed, State::Outdated, &new, Format::AppImage, &release).unwrap();
        assert_eq!(delta.control.name, "Tool-2-x86_64.AppImage.zsync");
        assert_eq!(delta.seed, seed);

        assert!(delta_for(&installed, State::Missing, &new, Format::AppImage, &release).is_none());
        let tarball = install(Format::Tarball, "v1");
        assert!(delta_for(&tarball, State::Outdated, &new, Format::AppImage, &release).is_none());
        release.assets.pop();
        assert!(delta_for(&installed, State::Outdated, &new, Format::AppImage, &release).is_none());
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
            desktop: Vec::new(),
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
