//! Shared helpers for commands: where the config is, and running core code
//! off the async runtime.
//!
//! The GUI never keeps a `Store` open. Each command loads it, acts and drops
//! it, because a loaded store holds the config's lock, and the background
//! timer and the CLI must keep working while the window is open.

use std::path::PathBuf;

use forager_core::store::Store;

/// What every command returns: the error is shown to the user as text.
pub type CmdResult<T> = Result<T, String>;

/// `FORAGER_CONFIG`, as for the CLI, or the default location.
pub fn config_path() -> PathBuf {
    std::env::var_os("FORAGER_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(Store::default_path)
}

pub fn load_store() -> CmdResult<Store> {
    Store::load(&config_path()).map_err(|e| e.to_string())
}

/// Runs blocking core code (network, package managers, file locks) on a
/// worker thread.
pub async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("worker failed: {e}"))?
}
