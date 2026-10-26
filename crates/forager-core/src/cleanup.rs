//! Settings, caches and data an app leaves in the home directory, offered
//! for deletion when the app is removed. Only folders and files named
//! exactly after the app (its id, package, commands or window class) right
//! inside the XDG directories count, plus a Flatpak's `~/.var/app/<id>`;
//! nothing is ever deleted that is not on that list.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::desktop;
use crate::error::{Error, Result};
use crate::model::{App, Format};

/// Names shared by many programs or owned by the desktop, never offered
/// even when an app happens to be called that.
const SHARED: [&str; 20] = [
    "applications",
    "autostart",
    "dconf",
    "flatpak",
    "fontconfig",
    "fonts",
    "forager",
    "gtk-2.0",
    "gtk-3.0",
    "gtk-4.0",
    "icons",
    "keyrings",
    "mime",
    "menus",
    "pulse",
    "recently-used.xbel",
    "systemd",
    "themes",
    "trash",
    "user-dirs.dirs",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Leftover {
    pub path: PathBuf,
    /// Size on disk of everything under it, links not followed.
    pub bytes: u64,
}

/// What `app` left in the home directory. Call before uninstalling: the
/// menu entries Forager wrote name the app's window class.
pub fn leftovers(app: &App) -> Vec<Leftover> {
    let roots: Vec<PathBuf> = [
        dirs::config_dir(),
        dirs::cache_dir(),
        dirs::data_dir(),
        dirs::state_dir(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let flatpaks: Vec<PathBuf> = match dirs::home_dir() {
        Some(home) => app
            .installs()
            .filter(|i| i.format == Format::Flatpak)
            .filter_map(|i| i.package.as_deref())
            .map(|id| home.join(".var/app").join(id))
            .collect(),
        None => Vec::new(),
    };
    find(&roots, &names(app), &flatpaks)
}

/// Deletes `paths`, each of which must be one of `allowed`: the app's
/// `leftovers`, worked out by the caller (before uninstalling) rather than
/// taken from whoever asks. Gives how many were deleted and their size.
pub fn remove(allowed: &[Leftover], paths: &[PathBuf]) -> Result<(usize, u64)> {
    let mut done = (0, 0);
    for path in paths {
        let leftover = allowed
            .iter()
            .find(|l| l.path == *path)
            .ok_or_else(|| Error::CommandFailed(format!("{} is not the app's to delete", path.display())))?;
        match fs::symlink_metadata(path)?.is_dir() {
            // remove_dir_all removes links inside, not what they point to.
            true => fs::remove_dir_all(path)?,
            false => fs::remove_file(path)?,
        }
        done = (done.0 + 1, done.1 + leftover.bytes);
    }
    Ok(done)
}

/// The names `app` may keep its files under, lowercase.
fn names(app: &App) -> BTreeSet<String> {
    let mut names = BTreeSet::from([app.id.clone()]);
    for installed in app.installs() {
        names.extend(
            installed
                .package
                .clone()
                .filter(|_| installed.format != Format::Flatpak),
        );
        names.extend(
            installed
                .links
                .iter()
                .filter_map(|l| Some(l.file_name()?.to_string_lossy().into_owned())),
        );
        for entry in installed
            .desktop
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
        {
            let Ok(text) = fs::read_to_string(entry) else { continue };
            names.extend(desktop::entry_field(&text, "StartupWMClass"));
            names.extend(desktop::entry_field(&text, "Name"));
        }
    }
    names
        .into_iter()
        .map(|n| n.to_lowercase())
        .filter(|n| !n.is_empty() && !n.contains('/') && !SHARED.contains(&n.as_str()))
        .collect()
}

/// Entries right inside `roots` whose name is one of `names` (case aside),
/// and those of `extra` that exist.
fn find(roots: &[PathBuf], names: &BTreeSet<String>, extra: &[PathBuf]) -> Vec<Leftover> {
    let mut found = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else { continue };
        let mut matched: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .filter(|e| names.contains(&e.file_name().to_string_lossy().to_lowercase()))
            .map(|e| e.path())
            .collect();
        matched.sort();
        found.extend(matched);
    }
    found.extend(extra.iter().filter(|p| fs::symlink_metadata(p).is_ok()).cloned());
    found.dedup();
    found
        .into_iter()
        .map(|path| Leftover {
            bytes: size(&path),
            path,
        })
        .collect()
}

/// Bytes under `path`, without following links.
fn size(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path)
        .map(|entries| entries.filter_map(|e| e.ok()).map(|e| size(&e.path())).sum())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Installed;
    use crate::source::Source;

    fn app() -> App {
        let mut app = App::new(
            Source::Direct {
                url: "https://x".into(),
            },
            "tool".into(),
        );
        app.installed = Some(Installed {
            tag: "1".into(),
            format: Format::Tarball,
            asset: "tool.tar.gz".into(),
            package: None,
            path: None,
            links: vec![PathBuf::from("/home/me/.local/bin/tl")],
            desktop: Vec::new(),
        });
        app
    }

    #[test]
    fn finds_folders_named_after_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let (config, cache) = (dir.path().join("config"), dir.path().join("cache"));
        for path in [
            config.join("Tool/settings.json"),
            config.join("tl/x"),
            config.join("toolbox/x"),
            config.join("gtk-3.0/x"),
            cache.join("tool/blob"),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "12345").unwrap();
        }
        let flatpak = dir.path().join("var/app/org.example.Tool");
        fs::create_dir_all(&flatpak).unwrap();
        let mut names = names(&app());
        names.insert("gtk-3.0".into());
        let found = find(
            &[config.clone(), cache.clone()],
            &names,
            &[flatpak.clone(), dir.path().join("missing")],
        );
        let paths: Vec<&Path> = found.iter().map(|l| l.path.as_path()).collect();
        assert_eq!(
            paths,
            [
                config.join("Tool").as_path(),
                config.join("gtk-3.0").as_path(),
                config.join("tl").as_path(),
                cache.join("tool").as_path(),
                flatpak.as_path()
            ]
        );
        assert_eq!(found[0].bytes, 5);
    }

    #[test]
    fn never_offers_shared_names() {
        let mut shared = app();
        shared.id = "Autostart".into();
        let names = names(&shared);
        assert!(!names.contains("autostart"));
        assert!(names.contains("tl"));
    }

    #[test]
    fn deletes_only_what_was_offered_and_not_through_links() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), "x").unwrap();
        let config = dir.path().join("config");
        let offered = config.join("tool");
        fs::create_dir_all(&offered).unwrap();
        std::os::unix::fs::symlink(&outside, offered.join("link")).unwrap();
        let allowed = find(
            std::slice::from_ref(&config),
            &BTreeSet::from(["tool".to_string()]),
            &[],
        );

        assert!(remove(&allowed, std::slice::from_ref(&outside)).is_err());
        assert!(outside.join("keep").exists());
        assert_eq!(remove(&allowed, std::slice::from_ref(&offered)).unwrap().0, 1);
        assert!(!offered.exists());
        assert!(outside.join("keep").exists());
    }
}
