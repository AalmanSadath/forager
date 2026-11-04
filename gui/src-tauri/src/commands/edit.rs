//! Looking up a project, previewing which asset settings would pick, and
//! adding and saving apps.

use std::path::PathBuf;

use forager_core::appimage::{self, AppImageInfo};
use forager_core::history::{self, Event, Kind, Via};
use forager_core::matcher::{self, Verdict};
use forager_core::model::{App, Format, Signature};
use forager_core::platform::Platform;
use forager_core::signature::{self, SignatureInput};
use forager_core::source::{Forge, Source};
use forager_core::{Error, update, version};
use serde::{Deserialize, Serialize};
use tauri::State;

use super::apps::AppView;
use super::releases::{ReleaseCache, ReleaseView, latest_tag, releases_of};
use crate::state::{CmdResult, blocking, load_store};

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub spec: Source,
    pub display: String,
    pub web_url: String,
    pub forge: &'static str,
    pub default_id: String,
}

/// Either the parsed project, or the host whose forge kind must be chosen.
#[derive(Debug, Clone, Serialize)]
pub struct ParsedSource {
    pub info: Option<SourceInfo>,
    pub unknown_host: Option<String>,
}

#[tauri::command]
pub fn parse_source(url: String, forge: Option<String>, pattern: Option<String>) -> CmdResult<ParsedSource> {
    let forge = forge.map(|f| f.parse::<Forge>()).transpose()?;
    let parsed = match (forge, pattern.filter(|p| !p.trim().is_empty())) {
        (Some(Forge::Html), Some(pattern)) => Source::html(&url, &pattern),
        (forge, _) => Source::from_url(&url, forge),
    };
    match parsed {
        Ok(spec) => Ok(ParsedSource {
            info: Some(SourceInfo {
                display: spec.to_string(),
                web_url: spec.web_url(),
                forge: spec.forge(),
                default_id: spec.default_id(),
                spec,
            }),
            unknown_host: None,
        }),
        Err(Error::UnknownForge(host)) => Ok(ParsedSource {
            info: None,
            unknown_host: Some(host),
        }),
        Err(e) => Err(e.to_string()),
    }
}

/// The settings an app page edits.
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsInput {
    pub format: Option<Format>,
    pub asset_filter: Option<String>,
    /// Regex a release tag must match.
    #[serde(default)]
    pub tag_filter: Option<String>,
    /// Versions to follow, as a semver requirement.
    #[serde(default)]
    pub versions: Option<String>,
    /// Days a release must be out before background updates install it.
    #[serde(default)]
    pub cooldown_days: u32,
    pub prerelease: bool,
    pub bins: Vec<String>,
    /// New download pattern, for an app tracked from a web page.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Only report new releases instead of installing them.
    #[serde(default)]
    pub track_only: bool,
}

impl SettingsInput {
    fn apply(&self, app: &mut App) -> CmdResult<()> {
        if let (Source::Html { url, .. }, Some(pattern)) = (&app.source, &self.pattern) {
            app.source = Source::html(&url.clone(), pattern).map_err(|e| e.to_string())?;
        }
        app.format = self.format;
        app.asset_filter = self.asset_filter.clone().filter(|f| !f.is_empty());
        app.tag_filter = self.tag_filter.clone().filter(|f| !f.is_empty());
        app.versions = self.versions.clone().filter(|v| !v.trim().is_empty());
        app.cooldown_days = self.cooldown_days;
        app.prerelease = self.prerelease;
        app.bins = self.bins.iter().filter(|b| !b.trim().is_empty()).cloned().collect();
        app.track_only = self.track_only;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateView {
    pub name: String,
    pub size: u64,
    pub format: Option<Format>,
    pub verdict: Verdict,
    /// The app's install that updating would replace with this asset.
    pub updates: Option<Format>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub release: ReleaseView,
    pub candidates: Vec<CandidateView>,
    /// The filter does not compile; candidates are shown without it.
    pub filter_error: Option<String>,
    /// The tag filter does not compile; releases are shown without it.
    pub tag_filter_error: Option<String>,
    /// The version limit does not parse; releases are shown without it.
    pub versions_error: Option<String>,
    /// The formats the newest release offers this system, for the format
    /// setting to choose from.
    pub formats: Vec<Format>,
}

/// What the latest release looks like under `settings`. Fetches the release
/// only when `refresh` is set or it is not cached yet. With the `id` of a
/// tracked app, assets that would update one of its installs are marked.
#[tauri::command]
pub async fn preview(
    cache: State<'_, ReleaseCache>,
    spec: Source,
    settings: SettingsInput,
    refresh: bool,
    id: Option<String>,
) -> CmdResult<Preview> {
    // A new pattern changes which links are releases, so it is part of the source.
    let mut app = App::new(spec, "preview".into());
    settings.apply(&mut app)?;
    let releases = releases_of(&cache, &app, refresh).await?;

    let filter_error = app
        .asset_filter
        .as_deref()
        .and_then(|f| regex::Regex::new(f).err())
        .map(|e| e.to_string());
    if filter_error.is_some() {
        app.asset_filter = None;
    }
    let tag_filter_error = app
        .tag_filter
        .as_deref()
        .and_then(|f| regex::Regex::new(f).err())
        .map(|e| e.to_string());
    if tag_filter_error.is_some() {
        app.tag_filter = None;
    }
    let versions_error = app
        .versions
        .as_deref()
        .and_then(|v| version::requirement(v).err())
        .map(|e| e.to_string());
    if versions_error.is_some() {
        app.versions = None;
    }
    let platform = Platform::detect();
    // From the newest release, not the one shown: that one is the newest
    // with something for these settings, which could hide a format only a
    // newer release offers.
    let formats = update::eligible(&app, releases.clone())
        .map_err(|e| e.to_string())?
        .first()
        .map(|newest| matcher::available_formats(&app, newest, &platform))
        .unwrap_or_default();
    let release = update::latest(&app, releases, &platform).map_err(|e| e.to_string())?;
    let installs = match id {
        Some(id) => blocking(move || Ok(load_store()?.get(&id).map(|a| a.install_list()).unwrap_or_default())).await?,
        None => Vec::new(),
    };
    let updates: Vec<(String, Format)> = installs
        .iter()
        .filter_map(|i| {
            update::asset_in(&app, &release, &platform, i.format).map(|asset| (asset.name.clone(), i.format))
        })
        .collect();
    let candidates = matcher::candidates(&app, &release, &platform)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|c| CandidateView {
            name: c.asset.name.clone(),
            size: c.asset.size,
            format: c.format,
            verdict: c.verdict,
            updates: updates.iter().find(|(name, _)| *name == c.asset.name).map(|(_, f)| *f),
        })
        .collect();
    Ok(Preview {
        release: ReleaseView::from(&release),
        candidates,
        filter_error,
        tag_filter_error,
        versions_error,
        formats,
    })
}

/// A signature setting from the form. `keep` leaves the current one alone.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignatureForm {
    Keep,
    None,
    Minisign { key: String },
    Gpg { key_file: PathBuf },
    Attestation,
}

/// Applies a signature form, storing a GPG key under the app's id and
/// dropping a keyring the app no longer uses.
fn apply_signature(app: &mut App, form: &SignatureForm) -> CmdResult<()> {
    let new = match form {
        SignatureForm::Keep => return Ok(()),
        SignatureForm::None => None,
        SignatureForm::Minisign { key } => Some(SignatureInput::Minisign(key.clone())),
        SignatureForm::Gpg { key_file } => Some(SignatureInput::GpgKeyFile(key_file.clone())),
        SignatureForm::Attestation => Some(SignatureInput::Attestation),
    }
    .map(|input| signature::resolve(&input, &app.id, &app.source))
    .transpose()
    .map_err(|e| e.to_string())?;
    if let Some(Signature::Gpg { keyring }) = &app.signature
        && !matches!(&new, Some(Signature::Gpg { keyring: k }) if k == keyring)
    {
        std::fs::remove_file(keyring).ok();
    }
    app.signature = new;
    Ok(())
}

/// What an AppImage chosen or dropped by the user says about itself.
#[tauri::command]
pub async fn inspect_appimage(path: PathBuf) -> CmdResult<AppImageInfo> {
    blocking(move || appimage::inspect(&path).map_err(|_| format!("{} is not an AppImage", path.display()))).await
}

/// An AppImage the user already has, to become the new app's install.
#[derive(Debug, Clone, Deserialize)]
pub struct AdoptInput {
    pub path: PathBuf,
    pub version: Option<String>,
    /// Copy the file rather than move it.
    pub keep: bool,
}

#[tauri::command]
pub async fn add_app(
    cache: State<'_, ReleaseCache>,
    spec: Source,
    id: String,
    settings: SettingsInput,
    signature: SignatureForm,
    adopt: Option<AdoptInput>,
) -> CmdResult<AppView> {
    let mut app = App::new(spec, id.trim().to_string());
    settings.apply(&mut app)?;
    // A tracked-only app reports releases after the one out now.
    if app.track_only {
        app.seen = Some(latest_tag(&cache, &app).await?);
    }
    blocking(move || {
        let mut store = load_store()?;
        app.validate().map_err(|e| e.to_string())?;
        if store.get(&app.id).is_ok() {
            return Err(format!("{} is already tracked", app.id));
        }
        // Last, since a GPG key is stored under the app's id.
        apply_signature(&mut app, &signature)?;
        let Some(adopt) = adopt else {
            let view = AppView::from(&app);
            store.add(app).map_err(|e| e.to_string())?;
            store.save().map_err(|e| e.to_string())?;
            return Ok(view);
        };
        let id = app.id.clone();
        let installed = update::adopt(&mut store, app, &adopt.path, adopt.version.as_deref(), adopt.keep)
            .map_err(|e| e.to_string())?;
        let mut event = Event::new(&id, Kind::Installed, Via::Gui);
        event.to = Some(installed.tag.clone());
        event.format = Some(Format::AppImage);
        event.message = Some(format!("adopted {}", adopt.path.display()));
        history::record(&event).ok();
        Ok(AppView::from(store.get(&id).map_err(|e| e.to_string())?))
    })
    .await
}

/// One app's result from `create_menu_entries`.
#[derive(Debug, Clone, Serialize)]
pub struct MenuEntryResult {
    pub id: String,
    pub error: Option<String>,
}

/// Adds the menu entries missing from AppImage and tarball installs, of the
/// apps named or of every app.
#[tauri::command]
pub async fn create_menu_entries(ids: Option<Vec<String>>) -> CmdResult<Vec<MenuEntryResult>> {
    blocking(move || {
        let mut store = load_store()?;
        let results = update::create_menu_entries(&mut store, &ids.unwrap_or_default()).map_err(|e| e.to_string())?;
        Ok(results
            .into_iter()
            .map(|(id, result)| MenuEntryResult {
                id,
                error: result.err().map(|e| e.to_string()),
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn save_app(
    cache: State<'_, ReleaseCache>,
    id: String,
    settings: SettingsInput,
    signature: SignatureForm,
) -> CmdResult<AppView> {
    let key = id.clone();
    let mut current = blocking(move || load_store()?.get(&key).cloned().map_err(|e| e.to_string())).await?;
    settings.apply(&mut current)?;
    // Turning tracking-only on: what is out now counts as seen.
    let seen = match current.track_only && current.seen.is_none() {
        true => Some(latest_tag(&cache, &current).await?),
        false => None,
    };
    blocking(move || {
        let mut store = load_store()?;
        let app = store.get_mut(&id).map_err(|e| e.to_string())?;
        let mut edited = app.clone();
        settings.apply(&mut edited)?;
        if seen.is_some() {
            edited.seen = seen;
        }
        edited.validate().map_err(|e| e.to_string())?;
        apply_signature(&mut edited, &signature)?;
        *app = edited;
        let view = AppView::from(&*app);
        store.save().map_err(|e| e.to_string())?;
        Ok(view)
    })
    .await
}

/// The app's own icon as a `data:` URL, or `None` when no install has one.
#[tauri::command]
pub async fn app_icon(id: String) -> CmdResult<Option<String>> {
    blocking(move || {
        let app = load_store()?.get(&id).cloned().map_err(|e| e.to_string())?;
        Ok(forager_core::icons::find(&app).map(|icon| icon.data_url()))
    })
    .await
}
