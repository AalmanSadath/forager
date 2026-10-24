//! The Settings page: appearance, the background check, tokens, and where
//! Forager keeps things.

use std::collections::BTreeMap;
use std::path::PathBuf;

use forager_core::{envfile, history, platform, store, timer};
use serde::{Deserialize, Serialize};

use crate::state::{CmdResult, blocking, config_path};

/// The window's own settings, in `~/.config/forager/gui.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiSettings {
    /// `system`, `dark`, `light` or `amoled`.
    pub app_theme: String,
    /// `#rrggbb`, or None to follow the desktop.
    pub accent_color: Option<String>,
}

impl Default for GuiSettings {
    fn default() -> Self {
        GuiSettings {
            app_theme: "system".into(),
            accent_color: None,
        }
    }
}

fn gui_settings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("forager")
        .join("gui.toml")
}

#[tauri::command]
pub fn get_gui_settings() -> GuiSettings {
    std::fs::read_to_string(gui_settings_path())
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn set_gui_settings(settings: GuiSettings) -> CmdResult<()> {
    let path = gui_settings_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string(&settings).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct TimerState {
    pub enabled: bool,
    pub auto_update: bool,
    pub next_run: Option<String>,
    /// The `forager` binary the timer would run, if one was found.
    pub cli: Option<PathBuf>,
}

/// The CLI the timer runs: shipped next to the app, else on PATH. Not one
/// inside a running AppImage, whose mount disappears when the app exits.
fn cli_path() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.parent()?.join("forager");
    let in_appimage = std::env::var_os("APPDIR").is_some_and(|dir| beside.starts_with(dir));
    if beside.is_file() && !in_appimage {
        return Some(beside);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join("forager"))
            .find(|p| p.is_file())
    })
}

fn timer_state() -> CmdResult<TimerState> {
    let next_run = timer::next_run().map_err(|e| e.to_string())?;
    Ok(TimerState {
        enabled: next_run.is_some(),
        auto_update: timer::auto_update_enabled(),
        next_run,
        cli: cli_path(),
    })
}

#[tauri::command]
pub async fn get_timer() -> CmdResult<TimerState> {
    blocking(timer_state).await
}

#[tauri::command]
pub async fn set_timer(enabled: bool, auto_update: bool) -> CmdResult<TimerState> {
    blocking(move || {
        if enabled {
            let exe = cli_path().ok_or("the forager command-line tool was not found next to the app or on PATH")?;
            let config = std::env::var_os("FORAGER_CONFIG").map(|_| config_path());
            timer::enable(&timer::Settings {
                exe,
                config,
                auto_update,
            })
            .map_err(|e| e.to_string())?;
        } else {
            timer::disable().map_err(|e| e.to_string())?;
        }
        timer_state()
    })
    .await
}

/// Variables the Tokens section edits; any others in the file are kept.
const TOKEN_KEYS: [&str; 3] = ["GITHUB_TOKEN", "GITLAB_TOKEN", "CODEBERG_TOKEN"];

#[tauri::command]
pub fn get_tokens() -> BTreeMap<String, String> {
    envfile::read_from(&envfile::path())
}

/// Saves the forge tokens and any `FORAGER_TOKEN_*` entries given, keeping
/// other variables in the file.
#[tauri::command]
pub fn set_tokens(tokens: BTreeMap<String, String>) -> CmdResult<()> {
    let path = envfile::path();
    let mut values = envfile::read_from(&path);
    values.retain(|k, _| !TOKEN_KEYS.contains(&k.as_str()) && !k.starts_with("FORAGER_TOKEN_"));
    for (k, v) in tokens {
        let allowed = TOKEN_KEYS.contains(&k.as_str())
            || (k.starts_with("FORAGER_TOKEN_") && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        if !allowed {
            return Err(format!("not a token variable: {k}"));
        }
        values.insert(k, v.trim().to_string());
    }
    envfile::write_to(&path, &values).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct AboutInfo {
    pub version: &'static str,
    pub config: PathBuf,
    pub env_file: PathBuf,
    pub history: PathBuf,
    pub cache: PathBuf,
    pub keys: PathBuf,
    pub apps_dir: Option<PathBuf>,
    pub opt_dir: Option<PathBuf>,
}

#[tauri::command]
pub fn about_info() -> AboutInfo {
    AboutInfo {
        version: env!("CARGO_PKG_VERSION"),
        config: config_path(),
        env_file: envfile::path(),
        history: history::path(),
        cache: store::cache_dir(),
        keys: dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("forager")
            .join("keys"),
        apps_dir: platform::appimage_dir(),
        opt_dir: platform::opt_dir(),
    }
}
