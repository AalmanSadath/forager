use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::App;

#[derive(Default, Serialize, Deserialize)]
struct AppsFile {
    #[serde(default)]
    apps: Vec<App>,
}

/// Tracked apps, persisted as TOML. Holds an exclusive lock on
/// `<path>.lock` while alive, so two Forager processes never interleave
/// installs or overwrite each other's changes.
pub struct Store {
    path: PathBuf,
    pub apps: Vec<App>,
    _lock: File,
}

impl Store {
    /// Default location: `$XDG_CONFIG_HOME/forager/apps.toml`.
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("forager")
            .join("apps.toml")
    }

    /// Loads the store, waiting for any other process holding it.
    pub fn load(path: &Path) -> Result<Store> {
        Store::load_waiting(path, || {})
    }

    /// Like `load`, but calls `on_wait` before blocking if another process
    /// holds the store.
    pub fn load_waiting(path: &Path, on_wait: impl FnOnce()) -> Result<Store> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        let lock = File::create(lock_path)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                on_wait();
                lock.lock()?;
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }

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
        // Ids become file paths, and the file may have been edited by hand.
        for app in &apps {
            crate::model::validate_id(&app.id)?;
        }
        Ok(Store {
            path: path.to_path_buf(),
            apps,
            _lock: lock,
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
/// The apps in the config at `path`, read without taking its lock: for
/// looking only, never for changing them.
pub fn read_apps(path: &Path) -> Result<Vec<App>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(toml::from_str::<AppsFile>(&text)
            .map_err(|source| Error::Config {
                path: path.to_path_buf(),
                source,
            })?
            .apps),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("forager")
}

/// Directory for downloaded assets: `$XDG_CACHE_HOME/forager/downloads`.
pub fn download_dir() -> PathBuf {
    cache_dir().join("downloads")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn second_load_waits_for_first_to_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("apps.toml");
        let first = Store::load(&path).unwrap();

        let (tx, rx) = mpsc::channel();
        let waiter = {
            let path = path.clone();
            thread::spawn(move || Store::load_waiting(&path, || tx.send(()).unwrap()).map(|_| ()))
        };
        rx.recv().unwrap();
        drop(first);
        waiter.join().unwrap().unwrap();
    }
}
