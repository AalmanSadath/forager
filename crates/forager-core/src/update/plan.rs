//! What an update would install: one plan per install that needs it, and
//! how the plan reads to the user.

use std::path::PathBuf;
use std::sync::LazyLock;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::matcher::select_asset;
use crate::model::{App, Asset, Format, Installed, Release, Signature};
use crate::platform::Platform;
use crate::signature::{self, Signed};
use crate::source::Source;
use crate::verify::Checksum;
use crate::{install, verify, version};

use super::status::{InstallState, State, eligible, status_in};

/// What `apply` would install for an app.
#[derive(Debug, Clone)]
pub struct Plan {
    pub release: Release,
    pub asset: Asset,
    pub format: Format,
    /// Checksum the download must match, if the release publishes one.
    pub checksum: Option<Checksum>,
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
    /// The Flatpak repository the install came from: the update is a
    /// `flatpak update` from it, and nothing is downloaded.
    pub via_repo: Option<install::flatpak::Remote>,
    /// The repository's commit of the release, when it is not the newest
    /// one there: going back, or to a release short of the latest.
    pub repo_commit: Option<String>,
    /// The release's `.flatpakrepo`, fetched with a Flatpak bundle that
    /// names no repository of its own, so the app is then installed from
    /// that repository and updated from it.
    pub repo_file: Option<Asset>,
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
        // Flatpak checks what comes from a repository against its key.
        self.via_repo.is_none() && self.checksum.is_none() && self.signature.is_none()
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
    if let Some(remote) = &plan.via_repo {
        let how = match &plan.repo_commit {
            Some(commit) => format!("flatpak update --commit={}", &commit[..commit.len().min(12)]),
            None => "flatpak update".into(),
        };
        let mut lines = vec![
            line(
                "from",
                format!("the {} Flatpak repository ({how}), no download", remote.name),
                Tone::Normal,
            ),
            line("source", app.source.to_string(), Tone::Normal),
            line(
                "note",
                "flatpak checks the update against the repository's key".into(),
                Tone::Normal,
            ),
        ];
        // Flatpak's system helper only follows the newest commit, so another
        // one goes through pkexec.
        if remote.system && plan.repo_commit.is_some() {
            let value = "flatpak asks for your password to move a system-wide install to another commit";
            lines.push(line("note", value.into(), Tone::Warn));
        }
        return lines;
    }
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
    lines.push(match (&plan.checksum, &plan.signature) {
        (Some(checksum), _) => {
            let from = checksum.from.as_deref().unwrap_or("the forge");
            line(
                "hash",
                format!("{} from {from}, will verify", checksum.algorithm),
                Tone::Normal,
            )
        }
        (None, Some(_)) => line("hash", "no checksum published".into(), Tone::Normal),
        (None, None) => line(
            "hash",
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
        let value = match plan.asset.name.to_ascii_lowercase().ends_with(".flatpakref") {
            true => "installed from the project's repository, which later updates come from",
            false => "a missing runtime is fetched from the bundle's runtime repo (usually Flathub)",
        };
        lines.push(line("note", value.into(), Tone::Normal));
    }
    if let Some(repo) = &plan.repo_file {
        let value = format!("then installed from the repository in {}, for later updates", repo.name);
        lines.push(line("repo", value, Tone::Normal));
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
    let status = status_in(app, releases.clone(), platform)?;
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
    let status = status_in(app, releases.clone(), platform)?;
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
    let checksum = verify::expected(&asset, release)?;
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
    let downgrade = current.is_some_and(|i| version::is_newer(&release.tag, &i.installed.tag));
    let origin = current
        .filter(|_| format == Format::Flatpak)
        .and_then(|current| repo_of(app, &current.installed));
    // Any release but the latest (going back, or short of the latest) is
    // the repository's commit of that version, when it still has one;
    // otherwise the release file is installed instead.
    let (via_repo, repo_commit) = match origin {
        Some((id, remote)) if downgrade || skip_after.is_some() => {
            match install::flatpak::history(&remote, id)
                .into_iter()
                .find(|(_, subject)| names_version(subject, &release.tag))
            {
                Some((commit, _)) => (Some(remote), Some(commit)),
                None => (None, None),
            }
        }
        Some((_, remote)) => (Some(remote), None),
        None => (None, None),
    };
    let repo_file = match via_repo.is_none() && format == Format::Flatpak && asset.name.ends_with(".flatpak") {
        true => release
            .assets
            .iter()
            .find(|a| a.name.to_ascii_lowercase().ends_with(".flatpakrepo"))
            .cloned(),
        false => None,
    };
    Ok(Plan {
        // Nothing is downloaded from the release when the repository updates it.
        asset_changed: asset_changed.filter(|_| via_repo.is_none()),
        via_repo,
        repo_commit,
        repo_file,
        delta,
        from: current
            .map(|i| i.installed.tag.clone())
            .or_else(|| replaces.first().map(|i| i.tag.clone())),
        release: release.clone(),
        asset,
        format,
        checksum,
        missing: current.is_some_and(|i| i.state == State::Missing),
        signature,
        downgrade,
        replaces,
        skip_after,
        changes: vec![release.clone()],
    })
}

/// The Flatpak id and repository `installed` updates from: an installed
/// Flatpak with a repository behind it, unless the app must be checked
/// against a signature on the release file.
fn repo_of<'a>(app: &App, installed: &'a Installed) -> Option<(&'a str, install::flatpak::Remote)> {
    if installed.format != Format::Flatpak || app.signature.is_some() {
        return None;
    }
    let id = installed.package.as_deref()?;
    Some((id, install::flatpak::origin(id)?))
}

/// For an app whose Flatpak updates from its repository, whether each of
/// `releases` (newest first, as `eligible` gives them) installs from there
/// rather than from the release file; `None` for other apps.
pub fn repo_releases(app: &App, releases: &[Release]) -> Option<Vec<bool>> {
    let (id, remote) = repo_of(app, app.install_of(Format::Flatpak)?)?;
    Some(in_history(&install::flatpak::history(&remote, id), releases))
}

/// The newest release is whatever the repository has last; any other is
/// there when one of its commits names that version.
fn in_history(history: &[(String, String)], releases: &[Release]) -> Vec<bool> {
    releases
        .iter()
        .enumerate()
        .map(|(i, release)| i == 0 || history.iter().any(|(_, subject)| names_version(subject, &release.tag)))
        .collect()
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

/// Whether a repository commit's `subject` (`Tool 1.2.0`) names the version
/// of the release `tag`.
fn names_version(subject: &str, tag: &str) -> bool {
    static VERSION: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"v?\d+(\.\d+)+(-[0-9A-Za-z.]+)?").unwrap());
    let Some(wanted) = version::parse_tag(tag) else {
        return false;
    };
    VERSION
        .find_iter(subject)
        .any(|m| version::parse_tag(m.as_str()).is_some_and(|v| v == wanted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::update::apply::repo_behind;
    use crate::update::fixtures::{install, plan};

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
    fn flatpaks_update_from_their_repository() {
        let app = App::new(
            Source::GitHub {
                owner: "o".into(),
                repo: "tool".into(),
            },
            "tool".into(),
        );
        let mut plan = plan(Format::Flatpak, None);
        assert!(plan.unverified());
        plan.via_repo = Some(install::flatpak::Remote {
            name: "tool-origin".into(),
            url: "https://example.org/repo".into(),
            system: false,
        });
        assert!(!plan.unverified());
        let lines = summary(&app, &plan);
        assert_eq!(lines[0].label, "from");
        assert!(lines[0].value.contains("tool-origin"));
        assert!(!lines.iter().any(|l| l.label == "hash" || l.label == "asset"));

        // Only another commit in the system installation needs a password.
        let asks = |plan: &Plan| summary(&app, plan).iter().any(|l| l.value.contains("password"));
        plan.repo_commit = Some("0123456789abcdef".into());
        assert!(!asks(&plan));
        plan.via_repo.as_mut().unwrap().system = true;
        assert!(asks(&plan));
        plan.repo_commit = None;
        assert!(!asks(&plan));

        assert!(repo_behind("1.2.0", "v1.3.0"));
        assert!(!repo_behind("1.3.0", "v1.3.0"));
        assert!(!repo_behind("1.4.0", "v1.3.0"));
        assert!(!repo_behind("nightly", "v1.3.0"));

        assert!(names_version("BifroSSH 0.17.0", "v0.17.0"));
        assert!(!names_version("BifroSSH 0.17.1", "v0.17.0"));
        assert!(!names_version("BifroSSH 0.17.0", "nightly"));
        assert!(names_version("Release v2.0.0-rc.1", "2.0.0-rc.1"));
    }

    #[test]
    fn marks_releases_the_repository_still_has() {
        let release = |tag: &str| Release {
            tag: tag.into(),
            name: None,
            prerelease: false,
            published_at: None,
            notes: None,
            assets: Vec::new(),
        };
        let history = [
            ("c3".to_string(), "Tool 1.2.0".to_string()),
            ("c1".to_string(), "Tool 1.0.0".to_string()),
        ];
        let releases = [
            release("v1.3.0"),
            release("v1.2.0"),
            release("v1.1.0"),
            release("v1.0.0"),
        ];
        assert_eq!(in_history(&history, &releases), [true, true, false, true]);
        assert_eq!(in_history(&[], &releases[1..]), [true, false, false]);
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
        assert_eq!(tone("hash", "NOT PUBLISHED"), Some(Tone::Danger));
        let verified = self::plan(Format::Tarball, Some("a".repeat(64)));
        let lines = summary(&app, &verified);
        assert!(
            lines
                .iter()
                .any(|l| l.label == "hash" && l.value == "sha256 from SHA256SUMS, will verify")
        );
        assert_eq!(tone("note", "as root"), Some(Tone::Warn));
        assert_eq!(tone("note", "replaces the appimage"), Some(Tone::Warn));
        assert_eq!(tone("source", "github.com/o/tool"), Some(Tone::Normal));
    }
}
