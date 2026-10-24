//! Installs Flatpak bundles into the user installation, so no root is needed,
//! unless the app is already in the system installation: then it is updated
//! there, with polkit asking for authorisation. A runtime the app needs is
//! fetched from the repository the bundle names (usually Flathub) if it is
//! not installed yet.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use regex::bytes::Regex;

use crate::error::{Error, Result};
use crate::host;

/// The bundle header is a GVariant that stores the ref (`app/<id>/<arch>/<branch>`)
/// as plain text near the start of the file.
static APP_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"app/([A-Za-z0-9_.-]+)/[A-Za-z0-9_]+/[A-Za-z0-9_.-]+").unwrap());

/// Application id of the bundle, e.g. `org.example.Tool`.
pub fn app_id(file: &Path) -> Result<String> {
    let mut header = Vec::new();
    File::open(file)?.take(1 << 20).read_to_end(&mut header)?;
    find_app_id(&header).ok_or_else(|| Error::Archive("not a Flatpak app bundle".into()))
}

fn find_app_id(header: &[u8]) -> Option<String> {
    let captures = APP_REF.captures(header)?;
    Some(String::from_utf8_lossy(&captures[1]).into_owned())
}

/// `--reinstall` lets the same command install, upgrade, or replace a copy
/// installed from somewhere else. App data in `~/.var/app` is kept.
pub fn install(file: &Path) -> Result<()> {
    let id = app_id(file)?;
    super::run(
        host::command("flatpak")
            .args([
                "install",
                installation(&id),
                "--noninteractive",
                "--reinstall",
                "--bundle",
            ])
            .arg(file),
    )
}

pub fn uninstall(id: &str) -> Result<()> {
    super::run(host::command("flatpak").args(["uninstall", installation(id), "--noninteractive", id]))
}

/// Whether `id` is installed, in the user or the system installation.
pub fn is_present(id: &str) -> bool {
    installed_in(id, "--user") || installed_in(id, "--system")
}

/// The installation holding `id`: the system one only if the app is there
/// and not in the user's, so an app installed system-wide is updated in
/// place instead of gaining a second copy.
fn installation(id: &str) -> &'static str {
    if !installed_in(id, "--user") && installed_in(id, "--system") {
        "--system"
    } else {
        "--user"
    }
}

fn installed_in(id: &str, installation: &str) -> bool {
    host::query("flatpak")
        .args(["info", installation, id])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Installed apps of both installations, as `(id, version)`; the version is
/// empty when the app does not declare one.
pub fn installed_apps() -> Vec<(String, String)> {
    let Ok(out) = host::query("flatpak")
        .args(["list", "--app", "--columns=application,version"])
        .output()
    else {
        return Vec::new();
    };
    parse_list(&String::from_utf8_lossy(&out.stdout))
}

fn parse_list(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let id = columns.next()?.trim();
            (!id.is_empty()).then(|| (id.to_string(), columns.next().unwrap_or("").trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_app_id_in_header() {
        let header = b"flatpak\0\0\x01\x02app/it.example.Tool/x86_64/stable\0metadata\0[Application]\nname=x";
        assert_eq!(find_app_id(header).as_deref(), Some("it.example.Tool"));
        assert_eq!(find_app_id(b"flatpak\0runtime/org.example.Platform/x86_64/1\0"), None);
    }

    #[test]
    fn parses_flatpak_list() {
        let apps = parse_list("org.example.Tool\t1.2.0\nio.github.o.Other\t\n\n");
        assert_eq!(
            apps,
            [
                ("org.example.Tool".into(), "1.2.0".into()),
                ("io.github.o.Other".into(), String::new())
            ]
        );
    }
}
