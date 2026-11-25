//! Starting an installed app, as its menu entry would: with the entry's own
//! arguments and window class, so it opens where the user is and its running
//! window has its icon.

use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::model::{App, Format, Installed};
use crate::{host, icons, install};

/// What starts an install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launcher {
    /// A menu entry: the one Forager wrote for an AppImage or tarball, or
    /// a system package's own.
    Entry(PathBuf),
    /// A Flatpak, by its application id.
    Flatpak(String),
}

/// How to start `installed`, if it has a way.
pub fn entry(installed: &Installed) -> Option<Launcher> {
    match installed.format {
        Format::AppImage | Format::Tarball => installed
            .desktop
            .iter()
            .find(|f| f.extension().is_some_and(|x| x == "desktop") && f.exists())
            .cloned()
            .map(Launcher::Entry),
        Format::Rpm | Format::Deb | Format::Pacman => {
            let list = icons::package_files(installed.package.as_deref()?, installed.format)?;
            let files: Vec<&str> = list.lines().map(str::trim).collect();
            icons::entry_in_list(&files).map(|f| Launcher::Entry(PathBuf::from(f)))
        }
        Format::Flatpak => installed.package.clone().map(Launcher::Flatpak),
    }
}

/// Starts `app` from its first install that is there and has a way to
/// start, in the app's order of preference. Returns once the launcher is
/// running; the app goes on by itself.
pub fn launch(app: &App) -> Result<()> {
    let launcher = app
        .installs()
        .filter(|i| install::is_present(i))
        .find_map(entry)
        .ok_or_else(|| Error::NotLaunchable(app.display_name().to_string()))?;
    let mut command = match &launcher {
        Launcher::Entry(file) => {
            let mut command = host::command_at_home("gio");
            command.arg("launch").arg(file);
            command
        }
        Launcher::Flatpak(id) => {
            let mut command = host::command_at_home("flatpak");
            command.arg("run").arg(id);
            command
        }
    };
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::CommandFailed(format!("cannot start {}: {e}", app.display_name())))?;
    // Reaped when it ends, so it does not linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(format: Format, package: Option<&str>, desktop: Vec<PathBuf>) -> Installed {
        Installed {
            tag: "v1".into(),
            format,
            asset: String::new(),
            package: package.map(str::to_string),
            path: None,
            links: Vec::new(),
            desktop,
            extract_and_run: false,
        }
    }

    #[test]
    fn starts_an_appimage_from_the_entry_forager_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let entry_file = dir.path().join("forager-tool.desktop");
        std::fs::write(&entry_file, "[Desktop Entry]\n").unwrap();
        let icon = dir.path().join("forager-tool.png");
        let appimage = installed(Format::AppImage, None, vec![icon, entry_file.clone()]);
        assert_eq!(entry(&appimage), Some(Launcher::Entry(entry_file.clone())));

        std::fs::remove_file(&entry_file).unwrap();
        assert_eq!(entry(&appimage), None);
    }

    #[test]
    fn starts_a_flatpak_by_its_id() {
        let flatpak = installed(Format::Flatpak, Some("org.example.Tool"), Vec::new());
        assert_eq!(entry(&flatpak), Some(Launcher::Flatpak("org.example.Tool".into())));
    }

    #[test]
    fn finds_a_packages_menu_entry_in_its_files() {
        let files = [
            "/usr/bin/tool",
            "/usr/share/applications/tool.desktop",
            "/usr/share/icons/tool.png",
        ];
        assert_eq!(
            icons::entry_in_list(&files),
            Some("/usr/share/applications/tool.desktop")
        );
        assert_eq!(icons::entry_in_list(&["/usr/bin/tool"]), None);
    }
}
