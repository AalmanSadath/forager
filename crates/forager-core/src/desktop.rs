//! Menu entries for the apps Forager places itself: AppImages and tarballs
//! would otherwise be files no launcher knows about.
//!
//! An AppImage carries its `.desktop` file and icon in the squashfs image
//! appended to it, which is read in place: the AppImage is never run to
//! extract them. A tarball gets an entry only when it ships one
//! (`share/applications`), so command-line tools stay out of the menu.
//!
//! Entries are written as `forager-<id>.desktop` with the icon renamed to
//! `forager-<id>`, so they never clash with a packaged copy of the same app,
//! and every file written is returned so it can be removed with the app.

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use backhand::{FilesystemReader, InnerNode};

use crate::error::{Error, Result};
use crate::host;

/// Icon sizes looked for, largest first; `scalable` holds SVGs.
const SIZES: [&str; 8] = [
    "scalable", "512x512", "256x256", "192x192", "128x128", "96x96", "64x64", "48x48",
];

/// The `.desktop` file inside the AppImage at `path`, if it has one. Fails
/// when `path` is not a type 2 AppImage.
pub fn appimage_entry(path: &Path) -> Result<Option<String>> {
    Ok(AppImage::open(path)?.desktop_entry())
}

/// A key's value in a menu entry's `[Desktop Entry]` group.
pub fn entry_field(entry: &str, key: &str) -> Option<String> {
    field(entry, key)
}

/// The menu entry and icon of an AppImage at `appimage`.
pub fn for_appimage(id: &str, appimage: &Path) -> Result<Vec<PathBuf>> {
    let image = AppImage::open(appimage)?;
    let entry = image.desktop_entry();
    let icon_name = entry.as_deref().and_then(|e| field(e, "Icon"));
    let icon = image.icon(icon_name.as_deref());
    write(id, entry.as_deref(), &quote(appimage), icon)
}

/// The menu entry and icon a tarball unpacked at `dir` ships, if it ships
/// one. `links` are the executables Forager linked, used to point `Exec` at
/// them.
pub fn for_tarball(id: &str, dir: &Path, links: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let applications = dir.join("share/applications");
    let Some(desktop) = fs::read_dir(&applications).ok().and_then(|entries| {
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "desktop"))
            .collect();
        files.sort();
        files.into_iter().next()
    }) else {
        return Ok(Vec::new());
    };
    let entry = fs::read_to_string(&desktop)?;
    let program = field(&entry, "Exec")
        .and_then(|exec| first_word(&exec))
        .map(|p| Path::new(&p).file_name().map(|n| n.to_owned()))
        .unwrap_or_default();
    // The linked executable of the same name, else one in the tree's bin/.
    let exec = program
        .and_then(|name| {
            links
                .iter()
                .find(|l| l.file_name() == Some(name.as_os_str()))
                .cloned()
                .or_else(|| Some(dir.join("bin").join(&name)).filter(|p| p.is_file()))
        })
        .ok_or_else(|| Error::Archive("the menu entry names no executable in the archive".into()))?;
    let icon = field(&entry, "Icon").and_then(|name| tarball_icon(dir, &name));
    write(id, Some(&entry), &quote(&exec), icon)
}

/// Whether a tarball unpacked at `dir` ships a menu entry.
pub fn tarball_has_entry(dir: &Path) -> bool {
    fs::read_dir(dir.join("share/applications")).is_ok_and(|entries| {
        entries
            .filter_map(|e| e.ok())
            .any(|e| e.path().extension().is_some_and(|x| x == "desktop"))
    })
}

/// Removes files written for a menu entry: only ones in the user's
/// applications and icons directories, whatever the config says.
pub fn remove(files: &[PathBuf]) {
    let (Some(apps), Some(icons)) = (applications_dir(), icons_dir()) else {
        return;
    };
    for file in files.iter().filter(|f| f.starts_with(&apps) || f.starts_with(&icons)) {
        fs::remove_file(file).ok();
    }
    refresh(&apps);
}

fn applications_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("applications"))
}

fn icons_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("icons/hicolor"))
}

/// Lets launchers see the change at once rather than on their next scan.
fn refresh(apps: &Path) {
    host::command("update-desktop-database").arg(apps).output().ok();
}

/// An icon's bytes and the hicolor size directory it belongs in.
struct Icon {
    data: Vec<u8>,
    size: String,
    svg: bool,
}

impl Icon {
    fn new(data: Vec<u8>) -> Option<Icon> {
        if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.len() >= 24 {
            let width = u32::from_be_bytes(data[16..20].try_into().ok()?);
            let height = u32::from_be_bytes(data[20..24].try_into().ok()?);
            let size = match width == height && SIZES.contains(&format!("{width}x{height}").as_str()) {
                true => format!("{width}x{height}"),
                // Odd sizes still show; launchers scale them.
                false => "256x256".into(),
            };
            return Some(Icon { data, size, svg: false });
        }
        let head = String::from_utf8_lossy(&data[..data.len().min(1024)]).to_ascii_lowercase();
        head.contains("<svg").then(|| Icon {
            data,
            size: "scalable".into(),
            svg: true,
        })
    }
}

fn write(id: &str, entry: Option<&str>, exec: &str, icon: Option<Icon>) -> Result<Vec<PathBuf>> {
    let apps = applications_dir().ok_or(Error::NoHome)?;
    let icons = icons_dir().ok_or(Error::NoHome)?;
    let icon_name = format!("forager-{id}");
    let mut written = Vec::new();
    if let Some(icon) = &icon {
        let dir = icons.join(&icon.size).join("apps");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{icon_name}.{}", if icon.svg { "svg" } else { "png" }));
        fs::write(&path, &icon.data)?;
        written.push(path);
    }
    let icon_field = icon.is_some().then_some(icon_name.as_str());
    let text = match entry {
        Some(entry) => rewrite(entry, id, exec, icon_field),
        None => minimal(id, exec, icon_field),
    };
    fs::create_dir_all(&apps)?;
    let path = apps.join(format!("forager-{id}.desktop"));
    fs::write(&path, text)?;
    written.push(path);
    refresh(&apps);
    Ok(written)
}

/// The app's own entry, launching `exec` (quoted) with its own arguments and
/// field codes, showing `icon` when one was copied. Lines that are not
/// plain single-line text are dropped, as are keys that would make the
/// entry start something other than `exec`.
fn rewrite(entry: &str, id: &str, exec: &str, icon: Option<&str>) -> String {
    let mut out = String::new();
    for line in entry.lines() {
        if line.contains(['\0', '\r']) {
            continue;
        }
        let key = line.split_once('=').map(|(k, _)| k.trim());
        match key {
            Some("TryExec" | "DBusActivatable") => continue,
            Some("Exec") => {
                let args = line.split_once('=').map_or("", |(_, v)| v.trim());
                let rest = rest_after_first_word(args);
                out.push_str(&format!("Exec={exec}{rest}\n"));
            }
            Some("Icon") if icon.is_some() => {
                out.push_str(&format!("Icon={}\n", icon.unwrap_or_default()));
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
        if line.trim() == "[Desktop Entry]" {
            out.push_str(&format!("X-Forager-App={id}\n"));
        }
    }
    out
}

fn minimal(id: &str, exec: &str, icon: Option<&str>) -> String {
    let icon = icon.map_or(String::new(), |i| format!("Icon={i}\n"));
    format!(
        "[Desktop Entry]\nX-Forager-App={id}\nType=Application\nName={id}\nExec={exec}\n{icon}Terminal=false\nCategories=Utility;\n"
    )
}

/// `path` as one `Exec` argument: quoted, with the characters the
/// specification reserves inside quotes escaped.
fn quote(path: &Path) -> String {
    let mut quoted = String::from('"');
    for c in path.to_string_lossy().chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// The value of `key` in the `[Desktop Entry]` group.
fn field(entry: &str, key: &str) -> Option<String> {
    let mut in_main = false;
    for line in entry.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
        } else if in_main
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// The program in an `Exec` value, without quotes.
fn first_word(exec: &str) -> Option<String> {
    let exec = exec.trim();
    match exec.strip_prefix('"') {
        Some(rest) => rest.split('"').next().map(str::to_string),
        None => exec.split_whitespace().next().map(str::to_string),
    }
}

/// Everything after the program in an `Exec` value, with its leading space.
fn rest_after_first_word(exec: &str) -> &str {
    let exec = exec.trim();
    let end = match exec.strip_prefix('"') {
        Some(rest) => rest.find('"').map_or(exec.len(), |i| i + 2),
        None => exec.find(char::is_whitespace).unwrap_or(exec.len()),
    };
    &exec[end.min(exec.len())..]
}

fn tarball_icon(dir: &Path, name: &str) -> Option<Icon> {
    let hicolor = dir.join("share/icons/hicolor");
    let from_theme = SIZES.iter().find_map(|size| {
        ["svg", "png"]
            .iter()
            .find_map(|ext| fs::read(hicolor.join(size).join("apps").join(format!("{name}.{ext}"))).ok())
    });
    from_theme
        .or_else(|| fs::read(dir.join("share/pixmaps").join(format!("{name}.png"))).ok())
        .and_then(Icon::new)
}

/// Where the squashfs image starts in an AppImage: right after the ELF
/// runtime, at the end of its section header table.
fn squashfs_offset(header: &[u8]) -> Option<u64> {
    if header.len() < 64 || &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 {
        return None;
    }
    let u64_at = |i: usize| header[i..i + 8].try_into().ok().map(u64::from_le_bytes);
    let u16_at = |i: usize| header[i..i + 2].try_into().ok().map(u16::from_le_bytes);
    let section_headers = u64_at(0x28)?;
    let entry_size = u64::from(u16_at(0x3a)?);
    let entries = u64::from(u16_at(0x3c)?);
    section_headers.checked_add(entry_size.checked_mul(entries)?)
}

struct AppImage {
    files: Vec<(PathBuf, Vec<u8>)>,
    links: Vec<(PathBuf, PathBuf)>,
}

impl AppImage {
    /// Reads the top-level files and the hicolor icons of the image: all
    /// that a menu entry needs, and small.
    fn open(path: &Path) -> Result<AppImage> {
        let mut header = [0u8; 64];
        File::open(path)?.read_exact(&mut header)?;
        let offset = squashfs_offset(&header).ok_or_else(|| Error::Archive("not a type 2 AppImage".into()))?;
        let reader = BufReader::new(File::open(path)?);
        let fs = FilesystemReader::from_reader_with_offset(reader, offset)
            .map_err(|e| Error::Archive(format!("cannot read the AppImage: {e}")))?;
        let mut files = Vec::new();
        let mut links = Vec::new();
        for node in fs.files() {
            let path = node.fullpath.clone();
            let wanted = path.parent() == Some(Path::new("/")) || path.starts_with("/usr/share/icons/hicolor");
            if !wanted {
                continue;
            }
            match &node.inner {
                InnerNode::File(file) => {
                    let mut data = Vec::new();
                    // Menu entries and icons are small; anything big is not one.
                    fs.file(file).reader().take(4 << 20).read_to_end(&mut data)?;
                    files.push((path, data));
                }
                InnerNode::Symlink(link) => links.push((path, link.link.clone())),
                _ => {}
            }
        }
        Ok(AppImage { files, links })
    }

    fn get(&self, path: &Path) -> Option<&[u8]> {
        let mut path = path.to_path_buf();
        // Follow a few symlinks (`.DirIcon` usually is one), relative to the root.
        for _ in 0..4 {
            if let Some((_, data)) = self.files.iter().find(|(p, _)| *p == path) {
                return Some(data);
            }
            let (_, target) = self.links.iter().find(|(p, _)| *p == path)?;
            path = Path::new("/").join(target.strip_prefix("/").unwrap_or(target));
        }
        None
    }

    fn desktop_entry(&self) -> Option<String> {
        let (_, data) = self
            .files
            .iter()
            .find(|(p, _)| p.parent() == Some(Path::new("/")) && p.extension().is_some_and(|x| x == "desktop"))?;
        String::from_utf8(data.clone()).ok()
    }

    fn icon(&self, name: Option<&str>) -> Option<Icon> {
        let named = name.into_iter().flat_map(|name| {
            let top = ["svg", "png"].map(|ext| PathBuf::from(format!("/{name}.{ext}")));
            let themed = SIZES.iter().flat_map(move |size| {
                ["svg", "png"].map(|ext| PathBuf::from(format!("/usr/share/icons/hicolor/{size}/apps/{name}.{ext}")))
            });
            top.into_iter().chain(themed).collect::<Vec<_>>()
        });
        named
            .chain([PathBuf::from("/.DirIcon")])
            .find_map(|path| self.get(&path).map(<[u8]>::to_vec))
            .and_then(Icon::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_squashfs_after_the_elf_runtime() {
        let mut header = vec![0u8; 64];
        header[..6].copy_from_slice(b"\x7fELF\x02\x01");
        header[0x28..0x30].copy_from_slice(&1000u64.to_le_bytes());
        header[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
        header[0x3c..0x3e].copy_from_slice(&10u16.to_le_bytes());
        assert_eq!(squashfs_offset(&header), Some(1640));
        header[4] = 1; // 32-bit
        assert_eq!(squashfs_offset(&header), None);
        assert_eq!(squashfs_offset(b"#!/bin/sh"), None);
    }

    #[test]
    fn rewrites_the_apps_own_entry() {
        let entry = "[Desktop Entry]\nName=Tool\nExec=tool --flag %U\nTryExec=tool\nIcon=tool\n\
                     DBusActivatable=true\nBad=li\rne\n\n[Desktop Action new]\nName=New window\nExec=\"tool\" --new\n";
        let out = rewrite(
            entry,
            "tool",
            "\"/home/me/Applications/tool.AppImage\"",
            Some("forager-tool"),
        );
        assert!(out.starts_with("[Desktop Entry]\nX-Forager-App=tool\nName=Tool\n"));
        assert!(out.contains("Exec=\"/home/me/Applications/tool.AppImage\" --flag %U\n"));
        assert!(out.contains("Exec=\"/home/me/Applications/tool.AppImage\" --new\n"));
        assert!(out.contains("Icon=forager-tool\n"));
        assert!(!out.contains("TryExec") && !out.contains("DBusActivatable") && !out.contains("Bad="));
        // Without a copied icon the app's own icon name stays.
        assert!(rewrite(entry, "tool", "x", None).contains("Icon=tool\n"));
    }

    #[test]
    fn quotes_exec_paths() {
        assert_eq!(quote(Path::new("/a b/$x\"y")), "\"/a b/\\$x\\\"y\"");
        assert_eq!(first_word("\"/opt/my tool\" %F").as_deref(), Some("/opt/my tool"));
        assert_eq!(rest_after_first_word("\"/opt/my tool\" %F"), " %F");
        assert_eq!(rest_after_first_word("tool"), "");
    }

    #[test]
    fn reads_fields_of_the_main_group_only() {
        let entry = "[Desktop Action x]\nIcon=other\n[Desktop Entry]\nIcon = tool\n";
        assert_eq!(field(entry, "Icon").as_deref(), Some("tool"));
    }

    #[test]
    fn sizes_icons_by_their_header() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(128u32.to_be_bytes());
        png.extend(128u32.to_be_bytes());
        assert_eq!(Icon::new(png).unwrap().size, "128x128");
        let svg = Icon::new(b"<?xml version=\"1.0\"?><svg xmlns=\"\"/>".to_vec()).unwrap();
        assert!(svg.svg);
        assert!(Icon::new(b"GIF89a".to_vec()).is_none());
    }

    #[test]
    fn tarballs_without_an_entry_get_none() {
        let dir = std::env::temp_dir().join(format!("forager-desktop-{}", std::process::id()));
        fs::create_dir_all(dir.join("bin")).unwrap();
        assert!(for_tarball("tool", &dir, &[]).unwrap().is_empty());
        fs::remove_dir_all(&dir).ok();
    }
}
