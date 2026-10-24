use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::source::Source;

/// Package formats Forager knows how to install.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Deb,
    Rpm,
    AppImage,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Deb, Format::Rpm, Format::AppImage];

    pub fn from_file_name(name: &str) -> Option<Format> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".deb") {
            Some(Format::Deb)
        } else if lower.ends_with(".rpm") && !lower.ends_with(".src.rpm") {
            Some(Format::Rpm)
        } else if lower.ends_with(".appimage") {
            Some(Format::AppImage)
        } else {
            None
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::Deb => "deb",
            Format::Rpm => "rpm",
            Format::AppImage => "appimage",
        })
    }
}

impl std::str::FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Format::ALL
            .into_iter()
            .find(|f| f.to_string().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown format '{s}' (expected deb, rpm or appimage)"))
    }
}

#[derive(Debug, Clone)]
pub struct Release {
    pub tag: String,
    pub name: Option<String>,
    pub prerelease: bool,
    pub published_at: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Digest published by the forge, e.g. `sha256:<hex>`.
    pub digest: Option<String>,
}

/// An app the user tracks, as persisted in `apps.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub source: Source,
    /// Regex an asset name must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_filter: Option<String>,
    /// Force a format instead of picking the best one for the system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<Installed>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installed {
    pub tag: String,
    pub format: Format,
    pub asset: String,
    /// Package name as known to the package manager (deb/rpm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Install location for formats Forager places itself (AppImage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}
