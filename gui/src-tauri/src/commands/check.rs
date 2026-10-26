//! Checking tracked apps for new releases.

use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use forager_core::install;
use forager_core::matcher::select_asset;
use forager_core::model::{App, Format, Installed, Release};
use forager_core::notice::{self, Notice};
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
    /// Each install and how it compares with the latest release.
    pub installs: Vec<InstallStatusView>,
    /// The latest release is the skipped one.
    pub skipped: bool,
    /// The latest release's page on the forge.
    pub release_url: String,
    /// When background updates may install the latest release, while the
    /// app's cooldown holds it back (ISO 8601).
    pub waits_until: Option<String>,
    /// Installs were found on the system, removed, or at another version
    /// than recorded, and saved, so the app list is out of date.
    pub found: bool,
    /// The repository was archived or moved, or nothing was released for long.
    pub notices: Vec<Notice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallStatusView {
    pub format: Format,
    pub tag: String,
    pub state: State,
    /// Version of the same package in the distro's repositories.
    pub repo_version: Option<String>,
}

/// Payload of the `app-status` event: one app's result.
#[derive(Debug, Clone, Serialize)]
pub struct StatusEvent {
    pub id: String,
    pub status: Option<StatusView>,
    pub error: Option<String>,
}

pub const STATUS_EVENT: &str = "app-status";

/// `app`'s status, from `prefetched` releases when there are some.
pub fn status_of(
    app: &App,
    platform: &Platform,
    cache: &ReleaseCache,
    prefetched: Option<Vec<Release>>,
) -> StatusEvent {
    let releases = prefetched.map_or_else(|| app.releases(), Ok);
    if let Ok(releases) = &releases {
        cache.put(app, releases.clone());
    }
    let result = releases
        .and_then(|releases| update::status_in(app, releases))
        .map(|status| {
            let found = status
                .found
                .clone()
                .is_some_and(|installs| record_found(&app.id, &app.install_list(), installs));
            let mut app = app.clone();
            if let Some(installs) = status.found {
                app.set_installs(installs);
            }
            let (asset, asset_format, asset_error) = match select_asset(&app, &status.latest, platform) {
                Ok((asset, format)) => (Some(asset.name.clone()), Some(format), None),
                Err(e) => (None, None, Some(e.to_string())),
            };
            let installs: Vec<InstallStatusView> = status
                .installs
                .iter()
                .map(|i| InstallStatusView {
                    format: i.installed.format,
                    tag: i.installed.tag.clone(),
                    state: i.state,
                    repo_version: i
                        .installed
                        .package
                        .as_deref()
                        .filter(|_| i.installed.format.needs_root())
                        .and_then(|p| install::repo_version(p, platform)),
                })
                .collect();
            let repo_version = installs.first().and_then(|i| i.repo_version.clone());
            StatusView {
                notices: notice::for_app(&app, &status.latest, update::now()),
                release_url: app.source.release_url(&status.latest.tag),
                waits_until: update::cooldown_until(&app, &status.latest, update::now()),
                state: status.state,
                latest_tag: status.latest.tag,
                latest_name: status.latest.name,
                published_at: status.latest.published_at,
                asset,
                asset_format,
                asset_error,
                repo_version,
                checked_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                installs,
                skipped: status.skipped,
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

/// Saves the installs found on the system as the app's own, unless the
/// record changed since the check read it (another process installed).
fn record_found(id: &str, checked: &[Installed], found: Vec<Installed>) -> bool {
    let Ok(mut store) = load_store() else {
        return false;
    };
    match store.get_mut(id) {
        Ok(app) if app.install_list() == checked => app.set_installs(found),
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
        // GitHub apps together through GraphQL when a token allows it.
        let prefetched = update::prefetch(&apps);
        let indexed: Vec<(usize, &App)> = apps.iter().enumerate().collect();
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            for chunk in indexed.chunks(indexed.len().div_ceil(PARALLEL).max(1)) {
                let tx = tx.clone();
                let platform = &platform;
                let cache = &cache;
                let prefetched = &prefetched;
                scope.spawn(move || {
                    for (i, app) in chunk {
                        tx.send(status_of(app, platform, cache, prefetched.get(i).cloned()))
                            .ok();
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
