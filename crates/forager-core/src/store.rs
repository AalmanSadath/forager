use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::App;

#[derive(Default, Serialize, Deserialize)]
struct AppsFile {
    #[serde(default)]
    apps: Vec<App>,
}

/// Tracked apps, persisted as TOML.
pub struct Store {
    path: PathBuf,
    pub apps: Vec<App>,
}

impl Store {
    /// Default location: `$XDG_CONFIG_HOME/forager/apps.toml`.
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("forager")
            .join("apps.toml")
    }

    pub fn load(path: &Path) -> Result<Store> {
        let apps = match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str::<AppsFile>(&text)
                    .map_err(|source| Error::Config {
                        path: path.to_path_buf(),
                        source,
                    })?
                    .apps
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Store {
            path: path.to_path_buf(),
            apps,
        })
    }

    /// Writes to a temporary file first so a crash never leaves a truncated config.
    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(&AppsFile {
            apps: self.apps.clone(),
        })?;
        let tmp = self.path.with_extension("toml.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<&App> {
        self.apps
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| Error::AppNotFound(id.to_string()))
    }

    pub fn get_mut(&mut self, id: &str) -> Result<&mut App> {
        self.apps
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| Error::AppNotFound(id.to_string()))
    }

    pub fn add(&mut self, app: App) -> Result<()> {
        if self.apps.iter().any(|a| a.id == app.id || a.source == app.source) {
            return Err(Error::AlreadyTracked(app.id));
        }
        self.apps.push(app);
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<App> {
        let index = self
            .apps
            .iter()
            .position(|a| a.id == id)
            .ok_or_else(|| Error::AppNotFound(id.to_string()))?;
        Ok(self.apps.remove(index))
    }
}

/// Forager's cache directory: `$XDG_CACHE_HOME/forager`.
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("forager")
}

/// Directory for downloaded assets: `$XDG_CACHE_HOME/forager/downloads`.
pub fn download_dir() -> PathBuf {
    cache_dir().join("downloads")
}
