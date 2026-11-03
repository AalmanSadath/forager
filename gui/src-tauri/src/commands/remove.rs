//! Uninstalling and removing apps, with what they leave behind.

use std::path::PathBuf;

use forager_core::history::{self, Event, Kind, Via};
use forager_core::model::{App, Format, Signature};
use forager_core::platform::Platform;
use forager_core::{cleanup, install};
use serde::Serialize;

use super::apps::AppView;
use crate::state::{CmdResult, blocking, load_store};

/// What may go with an app when it is uninstalled or removed: what it left
/// in the home directory, and the Flatpak repositories only it uses.
#[derive(Debug, Clone, Serialize)]
pub struct CleanupView {
    pub leftovers: Vec<cleanup::Leftover>,
    pub remotes: Vec<install::flatpak::Remote>,
}

impl CleanupView {
    /// Worked out before uninstalling: menu entries name the app, and its
    /// Flatpak's repository is known while it is installed.
    fn of(app: &App) -> CleanupView {
        CleanupView {
            leftovers: cleanup::leftovers(app),
            remotes: install::removable_remotes(app),
        }
    }

    /// Fails unless every path in `delete` and repository in `remotes` is
    /// one of these, whatever the window asks for.
    fn check(&self, id: &str, delete: &[PathBuf], remotes: &[String]) -> CmdResult<()> {
        if let Some(path) = delete.iter().find(|p| !self.leftovers.iter().any(|l| l.path == **p)) {
            return Err(format!("{} is not {id}'s to delete", path.display()));
        }
        if let Some(name) = remotes.iter().find(|n| !self.remotes.iter().any(|r| r.name == **n)) {
            return Err(format!("{name} is not {id}'s repository to remove"));
        }
        Ok(())
    }

    /// Deletes `delete` and, once the Flatpak is uninstalled
    /// (`flatpak_gone`), removes `remotes`. Gives the note for history.
    fn apply(&self, delete: &[PathBuf], remotes: &[String], flatpak_gone: bool) -> CmdResult<Option<String>> {
        let (count, bytes) = cleanup::remove(&self.leftovers, delete).map_err(|e| e.to_string())?;
        let removed = match flatpak_gone {
            true => install::remove_remotes(&self.remotes, remotes).map_err(|e| e.to_string())?,
            false => Vec::new(),
        };
        let mut parts = Vec::new();
        if count > 0 {
            parts.push(format!(
                "deleted {count} folders and files ({} KiB)",
                bytes.div_ceil(1024)
            ));
        }
        parts.extend(removed.iter().map(|name| format!("removed the {name} repository")));
        Ok((!parts.is_empty()).then(|| parts.join("; ")))
    }
}

/// What may go with `id`, for the Uninstall and Remove dialogs to offer.
#[tauri::command]
pub async fn app_cleanup(id: String) -> CmdResult<CleanupView> {
    blocking(move || Ok(CleanupView::of(load_store()?.get(&id).map_err(|e| e.to_string())?))).await
}

/// Uninstalls `id`'s installs in `formats` and keeps tracking it, deleting
/// the leftovers in `delete` and removing the repositories in `remotes`.
#[tauri::command]
pub async fn uninstall_app(
    id: String,
    formats: Vec<Format>,
    delete: Vec<PathBuf>,
    remotes: Vec<String>,
) -> CmdResult<AppView> {
    blocking(move || {
        let mut store = load_store()?;
        let allowed = CleanupView::of(store.get(&id).map_err(|e| e.to_string())?);
        allowed.check(&id, &delete, &remotes)?;
        let app = store.get_mut(&id).map_err(|e| e.to_string())?;
        let result = install::uninstall_formats(app, &formats, &Platform::detect());
        let view = AppView::from(&*app);
        // Saved either way: what was uninstalled before a failure is gone.
        store.save().map_err(|e| e.to_string())?;
        let uninstalled = result.map_err(|e| e.to_string())?;
        let flatpak_gone = uninstalled.iter().any(|i| i.format == Format::Flatpak);
        let note = allowed.apply(&delete, &remotes, flatpak_gone)?;
        for installed in &uninstalled {
            let mut event = Event::new(&id, Kind::Removed, Via::Gui);
            event.from = Some(installed.tag.clone());
            event.format = Some(installed.format);
            event.message = note.clone();
            history::record(&event).ok();
        }
        Ok(view)
    })
    .await
}

/// Stops tracking `id`, first uninstalling its installs in the `uninstall`
/// formats, deleting the leftovers in `delete` and removing the
/// repositories in `remotes`.
#[tauri::command]
pub async fn remove_app(
    id: String,
    uninstall: Vec<Format>,
    delete: Vec<PathBuf>,
    remotes: Vec<String>,
) -> CmdResult<()> {
    blocking(move || {
        let mut store = load_store()?;
        let app = store.get(&id).map_err(|e| e.to_string())?;
        let allowed = CleanupView::of(app);
        allowed.check(&id, &delete, &remotes)?;
        let chosen: Vec<_> = app
            .installs()
            .filter(|i| uninstall.contains(&i.format) && install::is_present(i))
            .cloned()
            .collect();
        let platform = Platform::detect();
        for installed in &chosen {
            install::uninstall(installed, &platform).map_err(|e| e.to_string())?;
        }
        let flatpak_gone = chosen.iter().any(|i| i.format == Format::Flatpak);
        let note = allowed.apply(&delete, &remotes, flatpak_gone)?;
        let app = store.remove(&id).map_err(|e| e.to_string())?;
        store.save().map_err(|e| e.to_string())?;
        if let Some(Signature::Gpg { keyring }) = &app.signature {
            std::fs::remove_file(keyring).ok();
        }
        if chosen.is_empty() {
            let mut event = Event::new(&id, Kind::Removed, Via::Gui);
            event.message = note.clone();
            history::record(&event).ok();
        }
        for installed in &chosen {
            let mut event = Event::new(&id, Kind::Removed, Via::Gui);
            event.from = Some(installed.tag.clone());
            event.format = Some(installed.format);
            event.message = note.clone();
            history::record(&event).ok();
        }
        Ok(())
    })
    .await
}
