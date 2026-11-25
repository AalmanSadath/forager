//! The Desktop entries page: menu entries for apps installed outside any
//! package, made, edited and removed in the user's applications folder.

use std::path::PathBuf;

use forager_core::entries::{self, Detected, Summary, Validation};

use crate::state::{CmdResult, blocking};

fn err(e: forager_core::error::Error) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn list_entries() -> CmdResult<Vec<Summary>> {
    blocking(|| entries::list().map_err(err)).await
}

#[tauri::command]
pub async fn read_entry(id: String) -> CmdResult<String> {
    blocking(move || entries::read(&id).map_err(err)).await
}

/// A new entry for the program at `path`, or for the app in the folder
/// `path`: its text, and what was found to choose from.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NewEntry {
    pub detected: Detected,
    pub text: Option<String>,
}

#[tauri::command]
pub async fn detect_entry(path: PathBuf) -> CmdResult<NewEntry> {
    blocking(move || {
        let detected = entries::detect(&path).map_err(err)?;
        let text = detected
            .program
            .as_deref()
            .map(|program| entries::template(program, &detected.name));
        Ok(NewEntry { detected, text })
    })
    .await
}

/// The `Exec` value for `program` (a host path, even from the document
/// portal) with `args`.
#[tauri::command]
pub fn exec_line(program: PathBuf, args: String) -> String {
    entries::exec_line(&entries::host_path(&program), &args)
}

#[tauri::command]
pub async fn validate_entry(text: String) -> CmdResult<Validation> {
    blocking(move || Ok(entries::validate(&text))).await
}

/// Saves `text` as entry `id`, copying `icon_file` as its icon when given.
#[tauri::command]
pub async fn save_entry(id: String, text: String, icon_file: Option<PathBuf>) -> CmdResult<Summary> {
    blocking(move || {
        entries::save(&id, &text, icon_file.as_deref()).map_err(err)?;
        entries::list()
            .map_err(err)?
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| format!("{id} was saved but cannot be read back"))
    })
    .await
}

#[tauri::command]
pub async fn remove_entry(id: String) -> CmdResult<()> {
    blocking(move || entries::remove(&id).map_err(err)).await
}

#[tauri::command]
pub async fn launch_entry(id: String) -> CmdResult<()> {
    blocking(move || entries::launch(&id).map_err(err)).await
}

/// The image at `path` as a `data:` URL, for showing a chosen icon.
#[tauri::command]
pub async fn icon_preview(path: PathBuf) -> CmdResult<Option<String>> {
    blocking(move || Ok(entries::preview(&entries::host_path(&path)))).await
}

/// An id for a new entry, from its name.
#[tauri::command]
pub fn entry_id(name: String) -> String {
    entries::slug(&name)
}
