//! The history log the CLI, the timer and the GUI all write to.

use forager_core::history::{self, Event};

use crate::state::{CmdResult, blocking};

/// Newest first, at most `limit`.
#[tauri::command]
pub async fn list_history(limit: usize) -> CmdResult<Vec<Event>> {
    blocking(move || history::read(limit).map_err(|e| e.to_string())).await
}
