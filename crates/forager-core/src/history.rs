//! A log of what Forager did, shared by the CLI, the GUI and the timer:
//! one JSON object per line in `$XDG_STATE_HOME/forager/history.jsonl`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::Format;

/// Lines kept when the file is trimmed.
const KEEP: usize = 5000;
/// The file is trimmed once it grows past this size.
const TRIM_AT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Installed,
    Updated,
    Removed,
    Failed,
    /// A check found a newer release.
    UpdatesFound,
}

/// What did it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Cli,
    Gui,
    Timer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Seconds since the Unix epoch.
    pub time: u64,
    pub app: String,
    pub kind: Kind,
    pub via: Via,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    /// Error text for `Failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Event {
    pub fn new(app: &str, kind: Kind, via: Via) -> Event {
        Event {
            time: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            app: app.to_string(),
            kind,
            via,
            from: None,
            to: None,
            format: None,
            message: None,
        }
    }
}

pub fn path() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("forager")
        .join("history.jsonl")
}

/// Appends `event`. Each event is one `write` of one line to a file opened
/// for appending, so concurrent writers do not interleave.
pub fn record(event: &Event) -> Result<()> {
    record_to(&path(), event)
}

/// Newest events first, at most `limit`.
pub fn read(limit: usize) -> Result<Vec<Event>> {
    read_from(&path(), limit)
}

pub fn record_to(path: &Path, event: &Event) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())?;
    if fs::metadata(path)?.len() > TRIM_AT {
        trim(path, KEEP)?;
    }
    Ok(())
}

pub fn read_from(path: &Path, limit: usize) -> Result<Vec<Event>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    // A line cut short by a crash, or written by a newer version, is skipped.
    Ok(text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str(line).ok())
        .take(limit)
        .collect())
}

fn trim(path: &Path, keep: usize) -> Result<()> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(keep);
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, lines[start..].join("\n") + "\n")?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_reads_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("forager/history.jsonl");
        assert!(read_from(&path, 10).unwrap().is_empty());

        let mut updated = Event::new("ripgrep", Kind::Updated, Via::Timer);
        updated.from = Some("14.0.0".into());
        updated.to = Some("15.2.0".into());
        updated.format = Some(Format::Tarball);
        record_to(&path, &Event::new("ripgrep", Kind::Installed, Via::Cli)).unwrap();
        record_to(&path, &updated).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"truncated")
            .unwrap();

        let events = read_from(&path, 10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], updated);
        assert_eq!(read_from(&path, 1).unwrap().len(), 1);
    }

    #[test]
    fn trims_to_the_newest_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");
        for i in 0..10 {
            record_to(&path, &Event::new(&format!("app{i}"), Kind::Removed, Via::Gui)).unwrap();
        }
        trim(&path, 3).unwrap();
        let apps: Vec<String> = read_from(&path, 10).unwrap().into_iter().map(|e| e.app).collect();
        assert_eq!(apps, ["app9", "app8", "app7"]);
    }
}
