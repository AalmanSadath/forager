//! Installs Flatpak bundles into the user installation, so no root is needed.
//! A runtime the app needs is fetched from the repository the bundle names
//! (usually Flathub) if it is not installed yet.

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
    super::run(
        host::command("flatpak")
            .args(["install", "--user", "--noninteractive", "--reinstall", "--bundle"])
            .arg(file),
    )
}

pub fn uninstall(id: &str) -> Result<()> {
    super::run(host::command("flatpak").args(["uninstall", "--user", "--noninteractive", id]))
}

pub fn is_present(id: &str) -> bool {
    host::command("flatpak")
        .args(["info", "--user", id])
        .output()
        .is_ok_and(|out| out.status.success())
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
}
