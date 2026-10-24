//! Looking up a project, previewing which asset settings would pick, and
//! adding, saving and removing apps.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use forager_core::history::{self, Event, Kind, Via};
use forager_core::matcher::{self, Verdict};
use forager_core::model::{App, Format, Release, Signature};
use forager_core::platform::Platform;
use forager_core::signature::{self, SignatureInput};
use forager_core::source::{Forge, Source};
use forager_core::{Error, install, update};
use serde::{Deserialize, Serialize};
use tauri::State;

use super::apps::AppView;
use crate::state::{CmdResult, blocking, load_store};

/// Latest release per source and prerelease setting, so previews while the
/// user types a filter need no network.
#[derive(Default)]
pub struct ReleaseCache(Mutex<HashMap<String, Vec<Release>>>);

impl ReleaseCache {
    /// The whole source, not its display name: a web page's releases depend
    /// on its pattern too.
    fn key(source: &Source) -> String {
        format!("{source:?}")
    }

    pub fn put(&self, source: &Source, releases: Vec<Release>) {
        self.0.lock().unwrap().insert(Self::key(source), releases);
    }

    fn get(&self, source: &Source) -> Option<Vec<Release>> {
        self.0.lock().unwrap().get(&Self::key(source)).cloned()
    }
}

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
    pub prerelease: bool,
    pub bins: Vec<String>,
    /// New download pattern, for an app tracked from a web page.
    #[serde(default)]
    pub pattern: Option<String>,
}

impl SettingsInput {
    fn apply(&self, app: &mut App) -> CmdResult<()> {
        if let (Source::Html { url, .. }, Some(pattern)) = (&app.source, &self.pattern) {
            app.source = Source::html(&url.clone(), pattern).map_err(|e| e.to_string())?;
        }
        app.format = self.format;
        app.asset_filter = self.asset_filter.clone().filter(|f| !f.is_empty());
        app.tag_filter = self.tag_filter.clone().filter(|f| !f.is_empty());
        app.prerelease = self.prerelease;
        app.bins = self.bins.iter().filter(|b| !b.trim().is_empty()).cloned().collect();
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseView {
    pub tag: String,
    pub name: Option<String>,
    pub published_at: Option<String>,
    pub prerelease: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateView {
    pub name: String,
    pub size: u64,
    pub format: Option<Format>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub release: ReleaseView,
    pub candidates: Vec<CandidateView>,
    /// The filter does not compile; candidates are shown without it.
    pub filter_error: Option<String>,
    /// The tag filter does not compile; releases are shown without it.
    pub tag_filter_error: Option<String>,
}

/// What the latest release looks like under `settings`. Fetches the release
/// only when `refresh` is set or it is not cached yet.
#[tauri::command]
pub async fn preview(
    cache: State<'_, ReleaseCache>,
    spec: Source,
    settings: SettingsInput,
    refresh: bool,
) -> CmdResult<Preview> {
    // A new pattern changes which links are releases, so it is part of the source.
    let mut app = App::new(spec, "preview".into());
    settings.apply(&mut app)?;
    let spec = app.source.clone();
    let cached = (!refresh).then(|| cache.get(&spec)).flatten();
    let releases = match cached {
        Some(releases) => releases,
        None => {
            let source = spec.clone();
            let releases = blocking(move || source.releases().map_err(|e| e.to_string())).await?;
            cache.put(&spec, releases.clone());
            releases
        }
    };

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
    let platform = Platform::detect();
    let release = update::latest(&app, releases, &platform).map_err(|e| e.to_string())?;
    let candidates = matcher::candidates(&app, &release, &platform)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|c| CandidateView {
            name: c.asset.name.clone(),
            size: c.asset.size,
            format: c.format,
            verdict: c.verdict,
        })
        .collect();
    Ok(Preview {
        release: ReleaseView {
            tag: release.tag.clone(),
            name: release.name.clone(),
            published_at: release.published_at.clone(),
            prerelease: release.prerelease,
        },
        candidates,
        filter_error,
        tag_filter_error,
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

#[tauri::command]
pub async fn add_app(
    spec: Source,
    id: String,
    settings: SettingsInput,
    signature: SignatureForm,
) -> CmdResult<AppView> {
    blocking(move || {
        let mut store = load_store()?;
        let mut app = App::new(spec, id.trim().to_string());
        settings.apply(&mut app)?;
        app.validate().map_err(|e| e.to_string())?;
        if store.get(&app.id).is_ok() {
            return Err(format!("{} is already tracked", app.id));
        }
        // Last, since a GPG key is stored under the app's id.
        apply_signature(&mut app, &signature)?;
        let view = AppView::from(&app);
        store.add(app).map_err(|e| e.to_string())?;
        store.save().map_err(|e| e.to_string())?;
        Ok(view)
    })
    .await
}

#[tauri::command]
pub async fn save_app(id: String, settings: SettingsInput, signature: SignatureForm) -> CmdResult<AppView> {
    blocking(move || {
        let mut store = load_store()?;
        let app = store.get_mut(&id).map_err(|e| e.to_string())?;
        let mut edited = app.clone();
        settings.apply(&mut edited)?;
        edited.validate().map_err(|e| e.to_string())?;
        apply_signature(&mut edited, &signature)?;
        *app = edited;
        let view = AppView::from(&*app);
        store.save().map_err(|e| e.to_string())?;
        Ok(view)
    })
    .await
}

/// Stops tracking `id`, first uninstalling its installs in the `uninstall` formats.
#[tauri::command]
pub async fn remove_app(id: String, uninstall: Vec<Format>) -> CmdResult<()> {
    blocking(move || {
        let mut store = load_store()?;
        let app = store.get(&id).map_err(|e| e.to_string())?;
        let chosen: Vec<_> = app
            .installs()
            .filter(|i| uninstall.contains(&i.format) && install::is_present(i))
            .cloned()
            .collect();
        let platform = Platform::detect();
        for installed in &chosen {
            install::uninstall(installed, &platform).map_err(|e| e.to_string())?;
        }
        let app = store.remove(&id).map_err(|e| e.to_string())?;
        store.save().map_err(|e| e.to_string())?;
        if let Some(Signature::Gpg { keyring }) = &app.signature {
            std::fs::remove_file(keyring).ok();
        }
        if chosen.is_empty() {
            history::record(&Event::new(&id, Kind::Removed, Via::Gui)).ok();
        }
        for installed in &chosen {
            let mut event = Event::new(&id, Kind::Removed, Via::Gui);
            event.from = Some(installed.tag.clone());
            event.format = Some(installed.format);
            history::record(&event).ok();
        }
        Ok(())
    })
    .await
}
