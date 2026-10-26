//! An app's own icon, for showing it in Forager: the one Forager copied for
//! an AppImage or tarball's menu entry, the one a system package installed
//! (found through the package's file list and its menu entry), or a
//! Flatpak's exported icon.
//!
//! Finding a system package's icon runs the package manager, so results
//! are kept in the cache until the install changes.

use std::fs;
use std::path::{Path, PathBuf};

use crate::desktop;
use crate::host;
use crate::model::{App, Format, Installed};
use crate::store;

/// Icons bigger than this are not shown: they are not icons.
const MAX_BYTES: usize = 512 * 1024;

/// hicolor sizes, best first: crisp at the card's size without being huge.
const SIZES: [&str; 9] = [
    "scalable", "256x256", "128x128", "512x512", "192x192", "96x96", "64x64", "48x48", "32x32",
];

/// An icon's bytes and their type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub data: Vec<u8>,
    /// `image/svg+xml` or `image/png`.
    pub mime: &'static str,
}

impl Icon {
    /// The icon as a `data:` URL, for an `<img>`.
    pub fn data_url(&self) -> String {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(&self.data);
        format!("data:{};base64,{data}", self.mime)
    }

    fn new(data: Vec<u8>) -> Option<Icon> {
        if data.len() > MAX_BYTES {
            return None;
        }
        let mime = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if String::from_utf8_lossy(&data[..data.len().min(1024)])
            .to_ascii_lowercase()
            .contains("<svg")
        {
            "image/svg+xml"
        } else {
            return None;
        };
        Some(Icon { data, mime })
    }
}

/// The icon of `app`'s installs, the preferred one first; `None` when no
/// install has one, or nothing is installed.
pub fn find(app: &App) -> Option<Icon> {
    app.installs().find_map(|installed| cached(&app.id, installed))
}

/// The icon of `installed` from the cache, or found and then cached. Not
/// finding one is cached too, so a package without an icon is not asked
/// about again until it changes.
fn cached(id: &str, installed: &Installed) -> Option<Icon> {
    let dir = store::cache_dir().join("icons");
    let key = format!(
        "{}\n{}\n{}\n",
        installed.format,
        installed.tag,
        installed.package.as_deref().unwrap_or("")
    );
    let name = format!("{id}.{}", installed.format);
    let (key_path, data_path) = (dir.join(format!("{name}.key")), dir.join(format!("{name}.icon")));
    if fs::read_to_string(&key_path).is_ok_and(|k| k == key) {
        return fs::read(&data_path).ok().and_then(Icon::new);
    }
    let icon = locate(installed).and_then(|path| read(&path)).and_then(Icon::new);
    // Best effort: without the cache the next look costs a query again.
    if fs::create_dir_all(&dir).is_ok() {
        match &icon {
            Some(icon) => fs::write(&data_path, &icon.data).ok(),
            None => fs::remove_file(&data_path).ok(),
        };
        fs::write(&key_path, key).ok();
    }
    icon
}

/// Where the icon of `installed` lives.
fn locate(installed: &Installed) -> Option<PathBuf> {
    match installed.format {
        // The icon Forager copied next to the menu entry it wrote.
        Format::AppImage | Format::Tarball => installed
            .desktop
            .iter()
            .find(|p| p.extension().is_some_and(|e| e == "png" || e == "svg"))
            .cloned(),
        Format::Rpm | Format::Deb | Format::Pacman => package_icon(installed.package.as_deref()?, installed.format),
        Format::Flatpak => first_existing(&flatpak_candidates(installed.package.as_deref()?)),
    }
}

/// The icon a system package installed: its menu entry names it, and its
/// file list or the system's icon theme has it.
fn package_icon(package: &str, format: Format) -> Option<PathBuf> {
    let list = match format {
        Format::Rpm => host::query("rpm").args(["-ql", package]).output(),
        Format::Deb => host::query("dpkg").args(["-L", package]).output(),
        _ => host::query("pacman").args(["-Qlq", package]).output(),
    }
    .ok()
    .filter(|out| out.status.success())?;
    let list = String::from_utf8_lossy(&list.stdout).into_owned();
    let files: Vec<&str> = list.lines().map(str::trim).collect();
    let entry = files
        .iter()
        .find(|f| f.contains("/share/applications/") && f.ends_with(".desktop"))?;
    let name = desktop::entry_field(&String::from_utf8_lossy(&read(Path::new(entry))?), "Icon")?;
    if name.starts_with('/') {
        return Some(PathBuf::from(name));
    }
    best_in_list(&files, &name)
        .map(PathBuf::from)
        .or_else(|| first_existing(&theme_candidates(&[Path::new("/usr/share")], &name)))
}

/// The best icon called `name` among a package's files.
fn best_in_list<'a>(files: &[&'a str], name: &str) -> Option<&'a str> {
    let rank = |file: &str| {
        let path = Path::new(file);
        let stem = path.file_stem()?.to_str()?;
        let ext = path.extension()?.to_str()?;
        if stem != name || !matches!(ext, "png" | "svg") {
            return None;
        }
        // Theme icons by size; others (pixmaps) after them.
        Some(
            SIZES
                .iter()
                .position(|s| file.contains(&format!("/{s}/")))
                .unwrap_or(SIZES.len()),
        )
    };
    files
        .iter()
        .filter_map(|f| Some((rank(f)?, *f)))
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, f)| f)
}

/// Paths an icon called `name` may have under each data directory in
/// `roots`, best first: hicolor by size, then pixmaps.
fn theme_candidates(roots: &[&Path], name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots {
        for size in SIZES {
            let ext = if size == "scalable" { "svg" } else { "png" };
            paths.push(root.join(format!("icons/hicolor/{size}/apps/{name}.{ext}")));
        }
        paths.push(root.join(format!("pixmaps/{name}.png")));
        paths.push(root.join(format!("pixmaps/{name}.svg")));
    }
    paths
}

/// Where a Flatpak's exported icon may be: per user, then system-wide. Its
/// icon is named after its id.
fn flatpak_candidates(flatpak_id: &str) -> Vec<PathBuf> {
    let user = dirs::data_dir().map(|d| d.join("flatpak/exports/share"));
    let system = PathBuf::from("/var/lib/flatpak/exports/share");
    let roots: Vec<&Path> = user.iter().map(PathBuf::as_path).chain([system.as_path()]).collect();
    theme_candidates(&roots, flatpak_id)
}

/// The first of `paths` that is a file. Inside Flatpak, system paths are
/// the host's, so one shell on the host looks.
fn first_existing(paths: &[PathBuf]) -> Option<PathBuf> {
    if !host::in_flatpak() {
        return paths.iter().find(|p| p.is_file()).cloned();
    }
    let out = host::query("sh")
        .args([
            "-c",
            r#"for f; do [ -f "$f" ] && { printf %s "$f"; exit; }; done"#,
            "sh",
        ])
        .args(paths)
        .output()
        .ok()?;
    let found = String::from_utf8_lossy(&out.stdout).into_owned();
    (!found.is_empty()).then(|| PathBuf::from(found))
}

/// Reads a file that may be the host's when Forager runs inside Flatpak.
fn read(path: &Path) -> Option<Vec<u8>> {
    if let Ok(data) = fs::read(path) {
        return Some(data);
    }
    if !host::in_flatpak() {
        return None;
    }
    let out = host::query("cat").arg(path).output().ok()?;
    out.status.success().then_some(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_best_size_from_a_package() {
        let files = [
            "/usr/bin/drawio",
            "/usr/share/applications/drawio.desktop",
            "/usr/share/icons/hicolor/16x16/apps/drawio.png",
            "/usr/share/icons/hicolor/512x512/apps/drawio.png",
            "/usr/share/icons/hicolor/256x256/apps/drawio.png",
            "/usr/share/icons/hicolor/256x256/apps/other.png",
            "/usr/share/pixmaps/drawio.xpm",
        ];
        assert_eq!(
            best_in_list(&files, "drawio"),
            Some("/usr/share/icons/hicolor/256x256/apps/drawio.png")
        );
        let pixmap = ["/usr/share/pixmaps/tool.png"];
        assert_eq!(best_in_list(&pixmap, "tool"), Some("/usr/share/pixmaps/tool.png"));
        assert_eq!(best_in_list(&files, "missing"), None);
    }

    #[test]
    fn looks_in_the_theme_by_size_then_pixmaps() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let candidates = theme_candidates(&[root], "tool");
        assert_eq!(candidates[0], root.join("icons/hicolor/scalable/apps/tool.svg"));
        assert_eq!(first_existing(&candidates), None);
        let pixmap = root.join("pixmaps/tool.png");
        fs::create_dir_all(pixmap.parent().unwrap()).unwrap();
        fs::write(&pixmap, b"x").unwrap();
        assert_eq!(first_existing(&candidates), Some(pixmap));
        let themed = root.join("icons/hicolor/128x128/apps/tool.png");
        fs::create_dir_all(themed.parent().unwrap()).unwrap();
        fs::write(&themed, b"x").unwrap();
        assert_eq!(first_existing(&candidates), Some(themed));
    }

    #[test]
    fn knows_pngs_and_svgs_only() {
        let png = b"\x89PNG\r\n\x1a\n....".to_vec();
        assert_eq!(Icon::new(png).map(|i| i.mime), Some("image/png"));
        let svg = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>"#.to_vec();
        assert_eq!(Icon::new(svg).map(|i| i.mime), Some("image/svg+xml"));
        assert_eq!(Icon::new(b"/* XPM */".to_vec()), None);
        let mut huge = b"\x89PNG\r\n\x1a\n".to_vec();
        huge.resize(MAX_BYTES + 1, 0);
        assert_eq!(Icon::new(huge), None);
        let svg = Icon::new(b"<svg/>".to_vec()).unwrap();
        assert_eq!(svg.data_url(), "data:image/svg+xml;base64,PHN2Zy8+");
    }

    #[test]
    fn appimages_use_the_icon_forager_copied() {
        let installed = Installed {
            tag: "v1".into(),
            format: Format::AppImage,
            asset: "tool.AppImage".into(),
            package: None,
            path: None,
            links: Vec::new(),
            desktop: vec![
                PathBuf::from("/home/me/.local/share/icons/hicolor/256x256/apps/forager-tool.png"),
                PathBuf::from("/home/me/.local/share/applications/forager-tool.desktop"),
            ],
        };
        assert_eq!(
            locate(&installed),
            Some(PathBuf::from(
                "/home/me/.local/share/icons/hicolor/256x256/apps/forager-tool.png"
            ))
        );
    }
}
