//! A tracked app's releases: listing them, skipping one, marking one seen,
//! and following a repository that moved.

use std::collections::HashMap;
use std::sync::Mutex;

use forager_core::model::{App, Release};
use forager_core::platform::Platform;
use forager_core::update;
use serde::Serialize;
use tauri::State;

use super::apps::AppView;
use crate::state::{CmdResult, blocking, load_store};

/// Latest release per source and prerelease setting, so previews while the
/// user types a filter need no network.
#[derive(Default)]
pub struct ReleaseCache(Mutex<HashMap<String, Vec<Release>>>);

impl ReleaseCache {
    /// The whole source, not its display name: a web page's releases depend
    /// on its pattern too. Apps with a version limit fetch a longer list.
    fn key(app: &App) -> String {
        format!("{:?} {}", app.source, app.versions.is_some())
    }

    pub fn put(&self, app: &App, releases: Vec<Release>) {
        self.0.lock().unwrap().insert(Self::key(app), releases);
    }

    fn get(&self, app: &App) -> Option<Vec<Release>> {
        self.0.lock().unwrap().get(&Self::key(app)).cloned()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseView {
    pub tag: String,
    pub name: Option<String>,
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub notes: Option<String>,
    /// For an app whose Flatpak updates from its repository, whether this
    /// release installs from there rather than from the release file.
    pub in_repo: Option<bool>,
}

impl From<&Release> for ReleaseView {
    fn from(release: &Release) -> ReleaseView {
        ReleaseView {
            tag: release.tag.clone(),
            name: release.name.clone(),
            published_at: release.published_at.clone(),
            prerelease: release.prerelease,
            notes: release.notes.clone(),
            in_repo: None,
        }
    }
}

/// The tag of the latest release `app` takes, from the cache if it is there.
pub(super) async fn latest_tag(cache: &ReleaseCache, app: &App) -> CmdResult<String> {
    let releases = releases_of(cache, app, false).await?;
    Ok(update::latest(app, releases, &Platform::detect())
        .map_err(|e| e.to_string())?
        .tag)
}

/// Marks a tracked-only app's release `tag` (the latest when `None`) as
/// seen, so it is no longer reported as new.
#[tauri::command]
pub async fn mark_seen(cache: State<'_, ReleaseCache>, id: String, tag: Option<String>) -> CmdResult<AppView> {
    let tag = match tag {
        Some(tag) => tag,
        None => {
            let key = id.clone();
            let app = blocking(move || load_store()?.get(&key).cloned().map_err(|e| e.to_string())).await?;
            latest_tag(&cache, &app).await?
        }
    };
    blocking(move || {
        let mut store = load_store()?;
        let app = store.get_mut(&id).map_err(|e| e.to_string())?;
        app.seen = Some(tag);
        let view = AppView::from(&*app);
        store.save().map_err(|e| e.to_string())?;
        Ok(view)
    })
    .await
}

/// Points the app at the address its repository moved to, as the forge
/// reports it. Installs and settings stay.
#[tauri::command]
pub async fn follow_move(id: String) -> CmdResult<AppView> {
    blocking(move || {
        let mut store = load_store()?;
        let source = store.get(&id).map_err(|e| e.to_string())?.source.clone();
        let moved = source
            .repo_info()
            .and_then(|info| source.moved_to(&info))
            .ok_or_else(|| format!("{source} has not moved"))?;
        if let Some(other) = store.apps.iter().find(|a| a.id != id && a.source == moved) {
            return Err(format!("{} already tracks {moved}", other.id));
        }
        let app = store.get_mut(&id).map_err(|e| e.to_string())?;
        app.source = moved;
        let view = AppView::from(&*app);
        store.save().map_err(|e| e.to_string())?;
        Ok(view)
    })
    .await
}

/// Releases of `app`'s source, from the cache unless `refresh` is set.
pub(super) async fn releases_of(cache: &ReleaseCache, app: &App, refresh: bool) -> CmdResult<Vec<Release>> {
    if let Some(releases) = (!refresh).then(|| cache.get(app)).flatten() {
        return Ok(releases);
    }
    let fetch = app.clone();
    let releases = blocking(move || fetch.releases().map_err(|e| e.to_string())).await?;
    cache.put(app, releases.clone());
    Ok(releases)
}

/// The app's releases, newest first, as its settings see them: tags its tag
/// filter and version limit accept, and prereleases only if it follows them.
#[tauri::command]
pub async fn list_releases(cache: State<'_, ReleaseCache>, id: String, refresh: bool) -> CmdResult<Vec<ReleaseView>> {
    let app = blocking(move || load_store()?.get(&id).cloned().map_err(|e| e.to_string())).await?;
    let releases = releases_of(&cache, &app, refresh).await?;
    let releases = update::eligible(&app, releases).map_err(|e| e.to_string())?;
    blocking(move || {
        let in_repo = update::repo_releases(&app, &releases);
        Ok(releases
            .iter()
            .enumerate()
            .map(|(i, release)| ReleaseView {
                in_repo: in_repo.as_ref().map(|flags| flags[i]),
                ..ReleaseView::from(release)
            })
            .collect())
    })
    .await
}

/// Stops offering `tag` (the latest release when `None`) as an update.
#[tauri::command]
pub async fn skip_release(cache: State<'_, ReleaseCache>, id: String, tag: Option<String>) -> CmdResult<AppView> {
    let tag = match tag {
        Some(tag) => tag,
        None => {
            let key = id.clone();
            let app = blocking(move || load_store()?.get(&key).cloned().map_err(|e| e.to_string())).await?;
            latest_tag(&cache, &app).await?
        }
    };
    set_skipped(id, Some(tag)).await
}

/// Offers the skipped release again.
#[tauri::command]
pub async fn unskip_release(id: String) -> CmdResult<AppView> {
    set_skipped(id, None).await
}

async fn set_skipped(id: String, tag: Option<String>) -> CmdResult<AppView> {
    blocking(move || {
        let mut store = load_store()?;
        let app = store.get_mut(&id).map_err(|e| e.to_string())?;
        app.skipped = tag;
        let view = AppView::from(&*app);
        store.save().map_err(|e| e.to_string())?;
        Ok(view)
    })
    .await
}
