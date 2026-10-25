//! What an AppImage downloaded by hand says about itself: its name and
//! version, from the menu entry inside it, and where updates come from,
//! from the update information in its `.upd_info` ELF section (written by
//! appimagetool, e.g. `gh-releases-zsync|owner|repo|latest|App-*x86_64.AppImage.zsync`).
//! Enough to track it from its project without asking the user where it
//! came from. The AppImage is never run.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Serialize;

use crate::desktop;
use crate::error::Result;
use crate::share::AddRequest;
use crate::source::{self, Source};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AppImageInfo {
    /// The app's name, from its menu entry.
    pub name: Option<String>,
    /// Its version, from `X-AppImage-Version` or else the file name.
    pub version: Option<String>,
    /// The raw update information, if any.
    pub update_info: Option<String>,
    /// The project and settings the update information points to, as the
    /// Add page takes them; `None` when it names none Forager can follow.
    pub source: Option<AddRequest>,
}

/// Reads what `path` says about itself. Fails only when it is not an
/// AppImage at all; missing pieces are `None`.
pub fn inspect(path: &Path) -> Result<AppImageInfo> {
    let entry = desktop::appimage_entry(path)?;
    let field = |key: &str| entry.as_deref().and_then(|e| desktop::entry_field(e, key));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let update_info = read_update_info(path)?;
    Ok(AppImageInfo {
        name: field("Name"),
        version: field("X-AppImage-Version").or_else(|| source::version_in(&file_name)),
        source: update_info.as_deref().and_then(request_for),
        update_info,
    })
}

/// The `.upd_info` section of a 64-bit little-endian ELF file, up to its
/// first NUL; `None` if it has none or it is empty.
pub fn read_update_info(path: &Path) -> Result<Option<String>> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header)?;
    if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 {
        return Ok(None);
    }
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    let u64_at = |b: &[u8], i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    let (table, entry_size, count, names_index) = (
        u64_at(&header, 0x28),
        usize::from(u16_at(&header, 0x3a)),
        usize::from(u16_at(&header, 0x3c)),
        usize::from(u16_at(&header, 0x3e)),
    );
    if entry_size < 64 || count == 0 || names_index >= count || count > 4096 {
        return Ok(None);
    }
    let mut sections = vec![0u8; entry_size * count];
    file.seek(SeekFrom::Start(table))?;
    file.read_exact(&mut sections)?;
    let section = |i: usize| &sections[i * entry_size..(i + 1) * entry_size];
    // (name offset, file offset, size) of each section.
    let fields = |s: &[u8]| (u32_at(s, 0) as usize, u64_at(s, 0x18), u64_at(s, 0x20));

    let (_, names_offset, names_size) = fields(section(names_index));
    let names = read_at(&mut file, names_offset, names_size.min(64 * 1024))?;
    for i in 0..count {
        let (name, offset, size) = fields(section(i));
        if names.get(name..).is_some_and(|rest| rest.starts_with(b".upd_info\0")) {
            let data = read_at(&mut file, offset, size.min(4096))?;
            let text = String::from_utf8_lossy(data.split(|b| *b == 0).next().unwrap_or_default())
                .trim()
                .to_string();
            return Ok((!text.is_empty()).then_some(text));
        }
    }
    Ok(None)
}

fn read_at(file: &mut File, offset: u64, size: u64) -> Result<Vec<u8>> {
    let mut data = vec![0u8; size as usize];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut data)?;
    Ok(data)
}

/// The project an AppImage's update information names, with the settings
/// that pick its file among the release's: `gh-releases-zsync` names a
/// GitHub repository and a file pattern, `zsync` the `.zsync` file next to
/// the AppImage. Other kinds (`pling-v1-zsync`, `bintray-zsync`) name no
/// project Forager reads.
pub fn request_for(info: &str) -> Option<AddRequest> {
    let parts: Vec<&str> = info.trim().split('|').collect();
    match parts.as_slice() {
        ["gh-releases-zsync", owner, repo, tag, pattern, ..] => {
            let source = Source::GitHub {
                owner: owner.to_string(),
                repo: repo.to_string(),
            };
            let file = pattern.strip_suffix(".zsync").unwrap_or(pattern);
            Some(AddRequest {
                url: source.web_url(),
                asset_filter: Some(glob_to_regex(file)),
                // `latest` follows releases; `latest-pre` and `latest-all`
                // prereleases too; anything else is one tag, kept to.
                tag_filter: match *tag {
                    "latest" | "latest-pre" | "latest-all" => None,
                    tag => Some(format!("^{}$", regex::escape(tag))),
                },
                prerelease: matches!(*tag, "latest-pre" | "latest-all"),
                ..AddRequest::default()
            })
        }
        ["zsync", url, ..] => {
            let file = url.strip_suffix(".zsync").unwrap_or(url);
            match Source::from_url(file, None) {
                Ok(source @ Source::GitHub { .. }) => Some(AddRequest {
                    url: source.web_url(),
                    ..AddRequest::default()
                }),
                _ => Some(AddRequest {
                    url: file.to_string(),
                    forge: Some("direct".into()),
                    ..AddRequest::default()
                }),
            }
        }
        _ => None,
    }
}

/// An appimagetool file pattern (`*` for any text) as an anchored regex.
fn glob_to_regex(glob: &str) -> String {
    let parts: Vec<String> = glob.split('*').map(regex::escape).collect();
    format!("^{}$", parts.join(".*"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A minimal ELF64 file with a section table: a null section, the
    /// section names, and `.upd_info` holding `info`.
    fn elf_with(info: &[u8]) -> tempfile::NamedTempFile {
        let names = b"\0.shstrtab\0.upd_info\0";
        let names_at = 64u64;
        let info_at = names_at + names.len() as u64;
        let table_at = info_at + 1024;
        let mut data = vec![0u8; table_at as usize + 3 * 64];
        data[..6].copy_from_slice(b"\x7fELF\x02\x01");
        data[0x28..0x30].copy_from_slice(&table_at.to_le_bytes());
        data[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
        data[0x3c..0x3e].copy_from_slice(&3u16.to_le_bytes());
        data[0x3e..0x40].copy_from_slice(&1u16.to_le_bytes());
        data[names_at as usize..info_at as usize].copy_from_slice(names);
        data[info_at as usize..info_at as usize + info.len()].copy_from_slice(info);
        let mut section = |i: usize, name: u32, offset: u64, size: u64| {
            let s = table_at as usize + i * 64;
            data[s..s + 4].copy_from_slice(&name.to_le_bytes());
            data[s + 0x18..s + 0x20].copy_from_slice(&offset.to_le_bytes());
            data[s + 0x20..s + 0x28].copy_from_slice(&size.to_le_bytes());
        };
        section(1, 1, names_at, names.len() as u64);
        section(2, 11, info_at, 1024);
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&data).unwrap();
        file
    }

    #[test]
    fn reads_the_update_information() {
        let info = b"gh-releases-zsync|AalmanSadath|forager|latest|Forager-*-x86_64.AppImage.zsync";
        let file = elf_with(info);
        assert_eq!(
            read_update_info(file.path()).unwrap().as_deref(),
            Some(std::str::from_utf8(info).unwrap())
        );
        assert_eq!(read_update_info(elf_with(b"").path()).unwrap(), None);
        let mut script = tempfile::NamedTempFile::new().unwrap();
        script.write_all(&[b'#'; 64]).unwrap();
        assert_eq!(read_update_info(script.path()).unwrap(), None);
    }

    #[test]
    fn follows_github_and_zsync_update_information() {
        let github = request_for("gh-releases-zsync|o|tool|latest|Tool-*-x86_64.AppImage.zsync").unwrap();
        assert_eq!(github.url, "https://github.com/o/tool");
        let filter = github.asset_filter.unwrap();
        assert_eq!(filter, r"^Tool\-.*\-x86_64\.AppImage$");
        assert!(
            regex::Regex::new(&filter)
                .unwrap()
                .is_match("Tool-1.2.3-x86_64.AppImage")
        );
        assert_eq!(github.tag_filter, None);
        assert!(!github.prerelease);

        let pinned = request_for("gh-releases-zsync|o|tool|continuous|tool.AppImage.zsync").unwrap();
        assert_eq!(pinned.tag_filter.as_deref(), Some("^continuous$"));
        assert!(
            request_for("gh-releases-zsync|o|tool|latest-pre|t.zsync")
                .unwrap()
                .prerelease
        );

        let on_github =
            request_for("zsync|https://github.com/o/tool/releases/download/v1/tool.AppImage.zsync").unwrap();
        assert_eq!(on_github.url, "https://github.com/o/tool");
        let elsewhere = request_for("zsync|https://example.org/dl/tool-latest.AppImage.zsync").unwrap();
        assert_eq!(elsewhere.url, "https://example.org/dl/tool-latest.AppImage");
        assert_eq!(elsewhere.forge.as_deref(), Some("direct"));

        assert_eq!(request_for("pling-v1-zsync|12345"), None);
        assert_eq!(request_for(""), None);
    }
}
