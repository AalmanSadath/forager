use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256, Sha512};

use crate::error::{Error, Result};
use crate::http;
use crate::model::{Asset, Release};

/// A hash algorithm releases publish checksums in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Sha256,
    Sha512,
}

impl Algorithm {
    /// The algorithm whose hex digest is `len` characters long.
    fn from_hex_len(len: usize) -> Option<Algorithm> {
        match len {
            64 => Some(Algorithm::Sha256),
            128 => Some(Algorithm::Sha512),
            _ => None,
        }
    }
}

impl fmt::Display for Algorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Algorithm::Sha256 => "sha256",
            Algorithm::Sha512 => "sha512",
        })
    }
}

/// A checksum a download must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    pub algorithm: Algorithm,
    /// Lowercase hex.
    pub hex: String,
    /// The release file it was read from; `None` for the forge's own digest.
    pub from: Option<String>,
}

/// Checksum of `asset`, from the forge's digest or from a checksum file
/// published in the same release: one for the asset alone
/// (`<asset>.sha256`), a list (`SHA256SUMS`, `checksums.txt`), or
/// electron-builder's `latest-linux.yml`. `None` if the release has none.
pub fn expected(asset: &Asset, release: &Release) -> Result<Option<Checksum>> {
    if let Some(digest) = &asset.digest {
        for (prefix, algorithm) in [("sha256:", Algorithm::Sha256), ("sha512:", Algorithm::Sha512)] {
            if let Some(hex) = digest.strip_prefix(prefix) {
                return Ok(Some(Checksum {
                    algorithm,
                    hex: hex.to_ascii_lowercase(),
                    from: None,
                }));
            }
        }
    }

    let lower_name = asset.name.to_ascii_lowercase();
    let own_files: Vec<String> = ["sha256", "sha256sum", "sha256.txt", "sha512", "sha512sum", "digests"]
        .iter()
        .map(|ext| format!("{lower_name}.{ext}"))
        .collect();
    let own = |a: &&Asset| own_files.contains(&a.name.to_ascii_lowercase());
    let list = |a: &&Asset| is_checksum_list(&a.name.to_ascii_lowercase());
    // The asset's own file first: it cannot be about another asset.
    for candidate in release
        .assets
        .iter()
        .filter(own)
        .chain(release.assets.iter().filter(list))
    {
        let text = http::check(http::get(&candidate.url).call()?)?
            .body_mut()
            .read_to_string()?;
        if let Some((algorithm, hex)) = find_in_checksums(&text, &asset.name, own(&candidate)) {
            return Ok(Some(Checksum {
                algorithm,
                hex,
                from: Some(candidate.name.clone()),
            }));
        }
    }
    for candidate in release.assets.iter().filter(|a| is_electron_builder_yml(&a.name)) {
        let text = http::check(http::get(&candidate.url).call()?)?
            .body_mut()
            .read_to_string()?;
        if let Some(hex) = find_in_electron_builder_yml(&text, &asset.name) {
            return Ok(Some(Checksum {
                algorithm: Algorithm::Sha512,
                hex,
                from: Some(candidate.name.clone()),
            }));
        }
    }
    Ok(None)
}

/// A file listing the checksums of several assets.
pub(crate) fn is_checksum_list(lower: &str) -> bool {
    let stem = lower.trim_end_matches(".txt");
    [
        "sha256sums",
        "sha256sum",
        "shasums256",
        "sha512sums",
        "sha512sum",
        "shasums512",
        "checksums",
    ]
    .iter()
    .any(|suffix| stem.ends_with(suffix))
        || matches!(
            lower,
            "checksums.sha256" | "checksums.sha512" | "sha256.txt" | "sha512.txt"
        )
}

/// electron-builder's update metadata, which carries a SHA-512 of each
/// file: `latest-linux.yml`, `latest-linux-arm64.yml`.
fn is_electron_builder_yml(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("latest-linux") && lower.ends_with(".yml")
}

/// Reads a checksum list for `name`'s hash: `sha256sum`/`sha512sum` output
/// (`<hex>  <name>` or `<hex> *<name>`) or BSD style
/// (`SHA256 (<name>) = <hex>`). A file made for a single asset may contain
/// only the hash. The algorithm follows from the hash's length.
pub(crate) fn find_in_checksums(text: &str, name: &str, single_file: bool) -> Option<(Algorithm, String)> {
    let is_hex = |hex: &str| hex.chars().all(|c| c.is_ascii_hexdigit());
    text.lines().find_map(|line| {
        let line = line.trim();
        let (hex, file) = match line.split_once(" (") {
            // BSD style: the hash comes last.
            Some((_, rest)) if rest.contains(") = ") => {
                let (file, hex) = rest.rsplit_once(") = ")?;
                (hex.trim(), Some(file))
            }
            _ => {
                let mut fields = line.split_whitespace();
                (fields.next()?, fields.next())
            }
        };
        let algorithm = Algorithm::from_hex_len(hex.len()).filter(|_| is_hex(hex))?;
        let matches = match file {
            Some(file) => file.trim_start_matches('*').rsplit('/').next() == Some(name),
            None => single_file,
        };
        matches.then(|| (algorithm, hex.to_ascii_lowercase()))
    })
}

/// The SHA-512 of `name` in electron-builder's `latest-linux.yml`, as hex.
/// The file lists `- url: <name>` entries, each followed by its base64
/// `sha512:`; a top-level `path:` with its own `sha512:` names the main one.
fn find_in_electron_builder_yml(text: &str, name: &str) -> Option<String> {
    use base64::Engine;
    let mut current: Option<&str> = None;
    for line in text.lines() {
        let line = line.trim().trim_start_matches("- ");
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches(['\'', '"']);
        match key.trim() {
            "url" | "path" => current = Some(value),
            "sha512" if current == Some(name) => {
                let bytes = base64::engine::general_purpose::STANDARD.decode(value).ok()?;
                return (bytes.len() == 64).then(|| hex::encode(bytes));
            }
            _ => {}
        }
    }
    None
}

/// Hashes the file at `path` with `algorithm`, as lowercase hex.
pub fn hash_file(path: &Path, algorithm: Algorithm) -> Result<String> {
    match algorithm {
        Algorithm::Sha256 => hash_with::<Sha256>(path),
        Algorithm::Sha512 => hash_with::<Sha512>(path),
    }
}

fn hash_with<D: Digest>(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = D::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Downloads `asset` into `dir`, returning the file path and its SHA-256.
/// Data goes to `<name>.part` first, so an interrupted download never
/// leaves a file that looks complete.
/// `progress` gets bytes received so far and the total, if known; returning
/// `Break` cancels the download with `Error::Cancelled`.
pub fn download(
    asset: &Asset,
    dir: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
) -> Result<(PathBuf, String)> {
    fs::create_dir_all(dir)?;
    let file_name = Path::new(&asset.name)
        .file_name()
        .ok_or_else(|| Error::CommandFailed(format!("bad asset name {}", asset.name)))?;
    let path = dir.join(file_name);
    let mut part = path.clone().into_os_string();
    part.push(".part");
    let part = PathBuf::from(part);

    let known_size = (asset.size > 0).then_some(asset.size);
    match fetch(&asset.url, &part, known_size, progress) {
        Ok(hash) => {
            fs::rename(&part, &path)?;
            Ok((path, hash))
        }
        Err(e) => {
            fs::remove_file(&part).ok();
            Err(e)
        }
    }
}

/// Streams `url` into `path`, returning its SHA-256.
fn fetch(
    url: &str,
    path: &Path,
    size: Option<u64>,
    progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
) -> Result<String> {
    let mut response = http::check(http::get(url).call()?)?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse().ok())
        .or(size);
    let mut done = 0;
    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(path)?;
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
        done += n as u64;
        if progress(done, total).is_break() {
            return Err(Error::Cancelled);
        }
    }
    file.sync_all()?;
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    #[test]
    fn finds_hash_in_sha256sums() {
        let text = format!("{HASH}  tool.rpm\n{}  other.deb\n", "a".repeat(64));
        assert_eq!(found(&text, "tool.rpm", false), Some(HASH.into()));
        assert_eq!(
            found(&format!("{HASH} *dist/tool.rpm"), "tool.rpm", false),
            Some(HASH.into())
        );
        assert_eq!(find_in_checksums(&text, "missing.rpm", false), None);
    }

    fn found(text: &str, name: &str, single_file: bool) -> Option<String> {
        find_in_checksums(text, name, single_file).map(|(_, hex)| hex)
    }

    #[test]
    fn reads_bsd_lines_and_sha512() {
        let bsd = format!("SHA256 (tool.rpm) = {HASH}\nSHA256 (other.rpm) = {}\n", "a".repeat(64));
        assert_eq!(found(&bsd, "tool.rpm", false), Some(HASH.into()));
        assert_eq!(found(&bsd, "other.rpm", false), Some("a".repeat(64)));
        let sha512 = "b".repeat(128);
        assert_eq!(
            find_in_checksums(&format!("{sha512}  tool.AppImage\n"), "tool.AppImage", false),
            Some((Algorithm::Sha512, sha512.clone()))
        );
        let bsd512 = format!("SHA512 (tool.AppImage) = {}", sha512.to_uppercase());
        assert_eq!(found(&bsd512, "tool.AppImage", false), Some(sha512));
        // Neither SHA-256 nor SHA-512.
        assert_eq!(
            find_in_checksums(&format!("{}  tool.rpm", "c".repeat(40)), "tool.rpm", false),
            None
        );
    }

    #[test]
    fn knows_checksum_lists_by_name() {
        for name in [
            "SHA256SUMS",
            "sha256sums.txt",
            "SHA512SUMS",
            "tool_1.0_checksums.txt",
            "checksums.sha256",
            "sha256.txt",
        ] {
            assert!(is_checksum_list(&name.to_ascii_lowercase()), "{name}");
        }
        for name in ["tool.tar.gz.sha256", "tool.tar.gz", "tool.sha512"] {
            assert!(!is_checksum_list(name), "{name}");
        }
        assert!(is_electron_builder_yml("latest-linux.yml"));
        assert!(is_electron_builder_yml("latest-linux-arm64.yml"));
        assert!(!is_electron_builder_yml("latest-mac.yml"));
    }

    #[test]
    fn reads_electron_builders_update_metadata() {
        use base64::Engine;
        let digest = |byte: u8| base64::engine::general_purpose::STANDARD.encode([byte; 64]);
        let yml = format!(
            "version: 1.14.5\nfiles:\n  - url: Obsidian-1.14.5.AppImage\n    sha512: {}\n    size: 139000000\n  \
             - url: obsidian_1.14.5_amd64.deb\n    sha512: {}\n    size: 90000000\npath: Obsidian-1.14.5.AppImage\n\
             sha512: {}\nreleaseDate: '2026-10-01T00:00:00.000Z'\n",
            digest(1),
            digest(2),
            digest(1)
        );
        assert_eq!(
            find_in_electron_builder_yml(&yml, "Obsidian-1.14.5.AppImage"),
            Some("01".repeat(64))
        );
        assert_eq!(
            find_in_electron_builder_yml(&yml, "obsidian_1.14.5_amd64.deb"),
            Some("02".repeat(64))
        );
        assert_eq!(find_in_electron_builder_yml(&yml, "missing.rpm"), None);
    }

    #[test]
    fn hashes_files_with_either_algorithm() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, "test").unwrap();
        assert_eq!(hash_file(&path, Algorithm::Sha256).unwrap(), HASH);
        assert_eq!(
            hash_file(&path, Algorithm::Sha512).unwrap(),
            hex::encode(Sha512::digest(b"test"))
        );
    }

    /// Serves `body` once over HTTP on a local port and returns its URL.
    fn serve_once(body: Vec<u8>) -> String {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/tool.tar.gz", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            let mut stream = stream;
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(&body).ok();
        });
        url
    }

    fn asset(url: String) -> Asset {
        Asset {
            name: "tool.tar.gz".into(),
            url,
            size: 0,
            digest: None,
        }
    }

    #[test]
    fn downloads_with_progress_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut seen = None;
        let (path, hash) = download(&asset(serve_once(b"test".to_vec())), dir.path(), &mut |done, total| {
            seen = Some((done, total));
            ControlFlow::Continue(())
        })
        .unwrap();
        assert_eq!(fs::read(path).unwrap(), b"test");
        assert_eq!(hash, HASH);
        assert_eq!(seen, Some((4, Some(4))));
    }

    #[test]
    fn cancelled_download_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let url = serve_once(vec![0; 1 << 20]);
        let result = download(&asset(url), dir.path(), &mut |_, _| ControlFlow::Break(()));
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn accepts_bare_hash_only_for_single_file() {
        assert_eq!(found(HASH, "tool.rpm", true), Some(HASH.into()));
        assert_eq!(find_in_checksums(HASH, "tool.rpm", false), None);
    }
}
