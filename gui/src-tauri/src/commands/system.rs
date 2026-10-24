use crate::appearance::{self, SystemAppearance};

/// Desktop colour scheme and accent, read through the portal.
#[tauri::command]
pub async fn system_appearance() -> SystemAppearance {
    tauri::async_runtime::spawn_blocking(appearance::read)
        .await
        .unwrap_or_default()
}
