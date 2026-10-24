//! `~/.config/forager/env`: `KEY=value` lines holding API tokens. The timer's
//! service reads it as its EnvironmentFile; the CLI and the GUI read it too,
//! so a token saved once applies everywhere. Variables set in the
//! environment take precedence.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::error::Result;

pub fn path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("forager")
        .join("env")
}

/// `name` from the environment, else from the env file.
pub fn get(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| read_from(&path()).remove(name))
}

/// Every `KEY=value` in the file. Blank lines and `#` comments are skipped,
/// and one level of matching quotes is removed, as systemd does.
pub fn read_from(path: &Path) -> BTreeMap<String, String> {
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            let v = v.trim();
            let unquoted = ['"', '\'']
                .iter()
                .find_map(|q| v.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
                .unwrap_or(v);
            (k.trim().to_string(), unquoted.to_string())
        })
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

/// Rewrites the file with `values`, readable by its owner only, since it
/// holds secrets. Comments at the top are kept.
pub fn write_to(path: &Path, values: &BTreeMap<String, String>) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut text = String::from("# API tokens for Forager, read by the CLI, the app and the background check.\n");
    for (k, v) in values.iter().filter(|(_, v)| !v.is_empty()) {
        text.push_str(&format!("{k}={v}\n"));
    }
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn round_trips_private_values() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("forager/env");
        let values = BTreeMap::from([
            ("GITHUB_TOKEN".to_string(), "ghp_x".to_string()),
            ("GITLAB_TOKEN".to_string(), String::new()),
        ]);
        write_to(&path, &values).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let read = read_from(&path);
        assert_eq!(read.get("GITHUB_TOKEN").map(String::as_str), Some("ghp_x"));
        assert!(!read.contains_key("GITLAB_TOKEN"));
    }

    #[test]
    fn reads_systemd_style_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("env");
        fs::write(&path, "# comment\n\nA=1\nB = \"two words\"\nC='3'\nnot a pair\n").unwrap();
        let read = read_from(&path);
        assert_eq!(read.get("A").map(String::as_str), Some("1"));
        assert_eq!(read.get("B").map(String::as_str), Some("two words"));
        assert_eq!(read.get("C").map(String::as_str), Some("3"));
        assert_eq!(read.len(), 3);
    }
}
