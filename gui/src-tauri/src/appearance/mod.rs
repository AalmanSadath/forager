//! What the desktop says it looks like: light or dark, and its accent colour.
//! As in BifroSSH, read through the XDG desktop portal. Both values are
//! advisory; a desktop that reports nothing leaves the app's own theme and
//! accent in place.

use serde::Serialize;

mod unix;

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct SystemAppearance {
    /// "dark", "light", or "no-preference".
    pub color_scheme: String,
    /// `#rrggbb`, or None when the desktop exposes no accent.
    pub accent: Option<String>,
}

/// Reads both values, blocking. Never fails.
pub fn read() -> SystemAppearance {
    unix::read()
}

/// Event the frontend listens on; payload is a [`SystemAppearance`].
pub const CHANGED_EVENT: &str = "system-appearance-changed";

/// Reports every change for as long as the app runs.
pub fn watch(app: tauri::AppHandle) {
    unix::watch(app)
}
