use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::model::{Asset, Release};

/// Expected SHA-256 of `asset`, from the forge's digest or from a checksum
/// file published in the same release. `None` if the release offers neither.
pub fn expected_sha256(asset: &Asset, release: &Release) -> Result<Option<String>> {
    if let Some(hex) = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        return Ok(Some(hex.to_ascii_lowercase()));
    }

    let own_file = format!("{}.sha256", asset.name).to_ascii_lowercase();
    for candidate in release.assets.iter().filter(|a| {
        let lower = a.name.to_ascii_lowercase();
        lower == own_file || is_checksum_list(&lower)
    }) {
        let text = ureq::get(&candidate.url).call()?.body_mut().read_to_string()?;
        let single_file = candidate.name.eq_ignore_ascii_case(&own_file);
        if let Some(hex) = find_in_checksums(&text, &asset.name, single_file) {
            return Ok(Some(hex));
        }
    }
    Ok(None)
}

fn is_checksum_list(lower: &str) -> bool {
    let stem = lower.trim_end_matches(".txt");
    stem.ends_with("sha256sums") || stem.ends_with("sha256sum") || stem.ends_with("checksums")
}

/// Reads `sha256sum` output (`<hex>  <name>` or `<hex> *<name>`). A file made
/// for a single asset may contain only the hash.
fn find_in_checksums(text: &str, name: &str, single_file: bool) -> Option<String> {
    text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hex = fields.next()?;
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let matches = match fields.next() {
            Some(file) => file.trim_start_matches('*').rsplit('/').next() == Some(name),
            None => single_file,
        };
        matches.then(|| hex.to_ascii_lowercase())
    })
}

/// Downloads `asset` into `dir`, returning the file path and its SHA-256.
pub fn download(asset: &Asset, dir: &Path) -> Result<(PathBuf, String)> {
    fs::create_dir_all(dir)?;
    let file_name = Path::new(&asset.name)
        .file_name()
        .ok_or_else(|| Error::CommandFailed(format!("bad asset name {}", asset.name)))?;
    let path = dir.join(file_name);

    let mut response = ureq::get(&asset.url).call()?;
    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(&path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    file.sync_all()?;
    Ok((path, hex::encode(hasher.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    #[test]
    fn finds_hash_in_sha256sums() {
        let text = format!("{HASH}  tool.rpm\n{}  other.deb\n", "a".repeat(64));
        assert_eq!(find_in_checksums(&text, "tool.rpm", false).as_deref(), Some(HASH));
        assert_eq!(
            find_in_checksums(&format!("{HASH} *dist/tool.rpm"), "tool.rpm", false).as_deref(),
            Some(HASH)
        );
        assert_eq!(find_in_checksums(&text, "missing.rpm", false), None);
    }

    #[test]
    fn accepts_bare_hash_only_for_single_file() {
        assert_eq!(find_in_checksums(HASH, "tool.rpm", true).as_deref(), Some(HASH));
        assert_eq!(find_in_checksums(HASH, "tool.rpm", false), None);
    }
}
