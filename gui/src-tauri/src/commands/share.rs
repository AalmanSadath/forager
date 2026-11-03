//! Sharing apps: export and import files, `forager://add` links and
//! "Add to Forager" badges.

use std::path::PathBuf;

use forager_core::share;
use serde::Serialize;

use crate::state::{CmdResult, blocking, load_store};

/// Writes every tracked app's settings to `path`, as `forager export` does.
#[tauri::command]
pub async fn export_apps(path: PathBuf) -> CmdResult<usize> {
    blocking(move || {
        let store = load_store()?;
        let text = share::export(&store).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(store.apps.len())
    })
    .await
}

/// One app's result from `import_apps`.
#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub id: String,
    pub error: Option<String>,
}

/// Tracks the apps in an export file that are not tracked yet.
#[tauri::command]
pub async fn import_apps(path: PathBuf) -> CmdResult<Vec<ImportResult>> {
    blocking(move || {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut store = load_store()?;
        let results = share::import(&mut store, &text).map_err(|e| e.to_string())?;
        store.save().map_err(|e| e.to_string())?;
        Ok(results
            .into_iter()
            .map(|(id, result)| ImportResult {
                id,
                error: result.err().map(|e| e.to_string()),
            })
            .collect())
    })
    .await
}

/// A `forager://add?…` link with the app's settings, to share it.
#[tauri::command]
pub async fn app_link(id: String) -> CmdResult<String> {
    blocking(move || Ok(share::link(load_store()?.get(&id).map_err(|e| e.to_string())?))).await
}

/// Markdown for an "Add to Forager" badge for `id`, for its project's README.
#[tauri::command]
pub async fn app_badge(id: String) -> CmdResult<String> {
    blocking(move || Ok(share::badge(load_store()?.get(&id).map_err(|e| e.to_string())?))).await
}

/// Reads a `forager://add?…` link, or the badge page's address, pasted
/// into the Add page.
#[tauri::command]
pub fn parse_link(text: String) -> CmdResult<share::AddRequest> {
    share::parse_link(&text).map_err(|e| e.to_string())
}
