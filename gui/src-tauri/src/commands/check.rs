//! Checking tracked apps for new releases.

use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use forager_core::install;
use forager_core::matcher::select_asset;
use forager_core::model::{App, Format, Installed};
use forager_core::platform::Platform;
use forager_core::update::{self, State};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::edit::ReleaseCache;

use crate::state::{CmdResult, blocking, load_store};

/// Apps checked at once: enough to be quick, few enough to be polite to the
/// forges' rate limits.
const PARALLEL: usize = 4;

#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    pub state: State,
    pub latest_tag: String,
    pub latest_name: Option<String>,
    pub published_at: Option<String>,
    /// The asset an update would install, or why there is none.
    pub asset: Option<String>,
    pub asset_format: Option<Format>,
    pub asset_error: Option<String>,
    /// Version of the same package in the distro's repositories.
    pub repo_version: Option<String>,
    /// Seconds since the Unix epoch.
    pub checked_at: u64,
    /// A system package installed without Forager was found and recorded as
    /// the app's install, so the app list is out of date.
    pub found: bool,
}

/// Payload of the `app-status` event: one app's result.
#[derive(Debug, Clone, Serialize)]
pub struct StatusEvent {
    pub id: String,
    pub status: Option<StatusView>,
    pub error: Option<String>,
}

pub const STATUS_EVENT: &str = "app-status";

pub fn status_of(app: &App, platform: &Platform, cache: &ReleaseCache) -> StatusEvent {
    let releases = app.source.releases();
    if let Ok(releases) = &releases {
        cache.put(&app.source, releases.clone());
    }
    let result = releases
        .and_then(|releases| update::status_in(app, releases))
        .map(|status| {
            let found = status
                .found
                .clone()
                .is_some_and(|installed| record_found(&app.id, installed));
            let app = &App {
                installed: app.installed.clone().or(status.found),
                ..app.clone()
            };
            let (asset, asset_format, asset_error) = match select_asset(app, &status.latest, platform) {
                Ok((asset, format)) => (Some(asset.name.clone()), Some(format), None),
                Err(e) => (None, None, Some(e.to_string())),
            };
            let repo_version = app
                .installed
                .as_ref()
                .filter(|i| i.format.needs_root())
                .and_then(|i| i.package.as_deref())
                .and_then(|p| install::repo_version(p, platform));
            StatusView {
                state: status.state,
                latest_tag: status.latest.tag,
                latest_name: status.latest.name,
                published_at: status.latest.published_at,
                asset,
                asset_format,
                asset_error,
                repo_version,
                checked_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                found,
            }
        });
    let (status, error) = match result {
        Ok(status) => (Some(status), None),
        Err(e) => (None, Some(e.to_string())),
    };
    StatusEvent {
        id: app.id.clone(),
        status,
        error,
    }
}

/// Saves an install found on the system as the app's own, unless one was
/// recorded meanwhile.
fn record_found(id: &str, installed: Installed) -> bool {
    let Ok(mut store) = load_store() else {
        return false;
    };
    match store.get_mut(id) {
        Ok(app) if app.installed.is_none() => app.installed = Some(installed),
        _ => return false,
    }
    store.save().is_ok()
}

/// Checks every app (or the ones named), emitting `app-status` for each as
/// it finishes, and returns once all are done.
#[tauri::command]
pub async fn check_apps(app: AppHandle, ids: Option<Vec<String>>) -> CmdResult<()> {
    blocking(move || {
        let apps: Vec<App> = load_store()?
            .apps
            .into_iter()
            .filter(|a| ids.as_ref().is_none_or(|ids| ids.contains(&a.id)))
            .collect();
        let platform = Platform::detect();
        let cache = app.state::<ReleaseCache>();
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            for chunk in apps.chunks(apps.len().div_ceil(PARALLEL).max(1)) {
                let tx = tx.clone();
                let platform = &platform;
                let cache = &cache;
                scope.spawn(move || {
                    for app in chunk {
                        tx.send(status_of(app, platform, cache)).ok();
                    }
                });
            }
            drop(tx);
            for event in rx {
                app.emit(STATUS_EVENT, event).ok();
            }
        });
        Ok(())
    })
    .await
}
