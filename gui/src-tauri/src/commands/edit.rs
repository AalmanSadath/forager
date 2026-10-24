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
use forager_core::{Error, install};
use serde::{Deserialize, Serialize};
use tauri::State;

use super::apps::AppView;
use crate::state::{CmdResult, blocking, load_store};

/// Latest release per source and prerelease setting, so previews while the
/// user types a filter need no network.
#[derive(Default)]
pub struct ReleaseCache(Mutex<HashMap<String, Release>>);

impl ReleaseCache {
    fn key(source: &Source, prerelease: bool) -> String {
        format!("{source}|{prerelease}")
    }

    pub fn put(&self, source: &Source, prerelease: bool, release: Release) {
        self.0.lock().unwrap().insert(Self::key(source, prerelease), release);
    }

    fn get(&self, source: &Source, prerelease: bool) -> Option<Release> {
        self.0.lock().unwrap().get(&Self::key(source, prerelease)).cloned()
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
pub fn parse_source(url: String, forge: Option<String>) -> CmdResult<ParsedSource> {
    let forge = forge.map(|f| f.parse::<Forge>()).transpose()?;
    match Source::from_url(&url, forge) {
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
    pub prerelease: bool,
    pub bins: Vec<String>,
}

impl SettingsInput {
    fn apply(&self, app: &mut App) {
        app.format = self.format;
        app.asset_filter = self.asset_filter.clone().filter(|f| !f.is_empty());
        app.prerelease = self.prerelease;
        app.bins = self.bins.iter().filter(|b| !b.trim().is_empty()).cloned().collect();
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
    let cached = (!refresh).then(|| cache.get(&spec, settings.prerelease)).flatten();
    let release = match cached {
        Some(release) => release,
        None => {
            let (source, prerelease) = (spec.clone(), settings.prerelease);
            let release = blocking(move || source.latest_release(prerelease).map_err(|e| e.to_string())).await?;
            cache.put(&spec, settings.prerelease, release.clone());
            release
        }
    };

    let mut app = App::new(spec, "preview".into());
    settings.apply(&mut app);
    let filter_error = app
        .asset_filter
        .as_deref()
        .and_then(|f| regex::Regex::new(f).err())
        .map(|e| e.to_string());
    if filter_error.is_some() {
        app.asset_filter = None;
    }
    let candidates = matcher::candidates(&app, &release, &Platform::detect())
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
        settings.apply(&mut app);
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
        settings.apply(&mut edited);
        edited.validate().map_err(|e| e.to_string())?;
        apply_signature(&mut edited, &signature)?;
        *app = edited;
        let view = AppView::from(&*app);
        store.save().map_err(|e| e.to_string())?;
        Ok(view)
    })
    .await
}

/// Stops tracking `id`, uninstalling it first when `uninstall` is set.
#[tauri::command]
pub async fn remove_app(id: String, uninstall: bool) -> CmdResult<()> {
    blocking(move || {
        let mut store = load_store()?;
        let app = store.get(&id).map_err(|e| e.to_string())?;
        let removed_tag = app.installed.as_ref().map(|i| i.tag.clone());
        if let Some(installed) = &app.installed
            && uninstall
            && install::is_present(installed)
        {
            install::uninstall(installed, &Platform::detect()).map_err(|e| e.to_string())?;
        }
        let app = store.remove(&id).map_err(|e| e.to_string())?;
        store.save().map_err(|e| e.to_string())?;
        if let Some(Signature::Gpg { keyring }) = &app.signature {
            std::fs::remove_file(keyring).ok();
        }
        let mut event = Event::new(&id, Kind::Removed, Via::Gui);
        event.from = removed_tag.filter(|_| uninstall);
        history::record(&event).ok();
        Ok(())
    })
    .await
}
