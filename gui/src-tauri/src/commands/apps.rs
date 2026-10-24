//! Tracked apps as the frontend sees them.

use std::path::PathBuf;

use forager_core::model::{App, Format, Installed, Signature};
use forager_core::source::Source;
use serde::Serialize;

use crate::state::{CmdResult, blocking, load_store};

/// One tracked app: its settings and what is installed.
#[derive(Debug, Clone, Serialize)]
pub struct AppView {
    pub id: String,
    /// The source as the backend stores it, handed back for previews.
    pub spec: Source,
    /// `github.com/owner/repo` and the like.
    pub source: String,
    pub web_url: String,
    pub forge: &'static str,
    /// Pinned format, or None for automatic.
    pub format: Option<Format>,
    pub asset_filter: Option<String>,
    pub tag_filter: Option<String>,
    pub prerelease: bool,
    pub bins: Vec<String>,
    /// `minisign`, `gpg` or `attestation` when a signature is required.
    pub signature: Option<String>,
    pub minisign_key: Option<String>,
    pub gpg_keyring: Option<PathBuf>,
    /// The install in the preferred format.
    pub installed: Option<InstalledView>,
    /// Every install, the preferred one first.
    pub installs: Vec<InstalledView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledView {
    pub tag: String,
    pub format: Format,
    pub asset: String,
    pub package: Option<String>,
    pub path: Option<PathBuf>,
    pub links: Vec<PathBuf>,
}

impl From<&Installed> for InstalledView {
    fn from(i: &Installed) -> InstalledView {
        InstalledView {
            tag: i.tag.clone(),
            format: i.format,
            asset: i.asset.clone(),
            package: i.package.clone(),
            path: i.path.clone(),
            links: i.links.clone(),
        }
    }
}

impl From<&App> for AppView {
    fn from(app: &App) -> AppView {
        AppView {
            id: app.id.clone(),
            spec: app.source.clone(),
            source: app.source.to_string(),
            web_url: app.source.web_url(),
            forge: app.source.forge(),
            format: app.format,
            asset_filter: app.asset_filter.clone(),
            tag_filter: app.tag_filter.clone(),
            prerelease: app.prerelease,
            bins: app.bins.clone(),
            signature: app.signature.as_ref().map(|s| s.to_string()),
            minisign_key: match &app.signature {
                Some(Signature::Minisign { public_key }) => Some(public_key.clone()),
                _ => None,
            },
            gpg_keyring: match &app.signature {
                Some(Signature::Gpg { keyring }) => Some(keyring.clone()),
                _ => None,
            },
            installed: app.installed.as_ref().map(InstalledView::from),
            installs: app.installs().map(InstalledView::from).collect(),
        }
    }
}

#[tauri::command]
pub async fn list_apps() -> CmdResult<Vec<AppView>> {
    blocking(|| Ok(load_store()?.apps.iter().map(AppView::from).collect())).await
}
