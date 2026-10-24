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
    /// A tar archive, optionally compressed (gzip, bzip2, xz or zstd).
    Tarball,
    /// A single-file Flatpak bundle.
    Flatpak,
    /// An Arch Linux package (`.pkg.tar.*`).
    Pacman,
}

impl Format {
    pub const ALL: [Format; 6] = [
        Format::Deb,
        Format::Rpm,
        Format::AppImage,
        Format::Tarball,
        Format::Flatpak,
        Format::Pacman,
    ];

    pub fn from_file_name(name: &str) -> Option<Format> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".deb") {
            Some(Format::Deb)
        } else if lower.ends_with(".rpm") && !lower.ends_with(".src.rpm") {
            Some(Format::Rpm)
        } else if lower.ends_with(".appimage") {
            Some(Format::AppImage)
        } else if lower.ends_with(".flatpak") {
            Some(Format::Flatpak)
        } else if PACMAN_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix)) {
            // Checked before tarballs, which these also look like.
            Some(Format::Pacman)
        } else if TARBALL_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix)) {
            Some(Format::Tarball)
        } else {
            None
        }
    }
}

/// `.pacman` is what electron-builder names Arch packages.
const PACMAN_SUFFIXES: [&str; 4] = [".pkg.tar.zst", ".pkg.tar.xz", ".pkg.tar.gz", ".pacman"];

const TARBALL_SUFFIXES: [&str; 9] = [
    ".tar", ".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tar.xz", ".txz", ".tar.zst", ".tzst",
];

impl Format {
    /// Installed through the system package manager, as root.
    pub fn needs_root(self) -> bool {
        matches!(self, Format::Deb | Format::Rpm | Format::Pacman)
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::Deb => "deb",
            Format::Rpm => "rpm",
            Format::AppImage => "appimage",
            Format::Tarball => "tarball",
            Format::Flatpak => "flatpak",
            Format::Pacman => "pacman",
        })
    }
}

impl std::str::FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Format::ALL
            .into_iter()
            .find(|f| f.to_string().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown format '{s}' (expected deb, rpm, appimage, tarball, flatpak or pacman)"))
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
    /// Executables to link from a tarball, relative to its top directory.
    /// Found automatically when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bins: Vec<String>,
    /// Key the app's assets must be signed with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<Installed>,
}

impl App {
    /// An app with default settings, not yet installed.
    pub fn new(source: Source, id: String) -> App {
        App {
            id,
            source,
            asset_filter: None,
            format: None,
            prerelease: false,
            bins: Vec::new(),
            signature: None,
            installed: None,
        }
    }

    /// Checks settings that come from the user: the id, which becomes part
    /// of file paths, and the asset filter regex.
    pub fn validate(&self) -> crate::Result<()> {
        validate_id(&self.id)?;
        if let Some(filter) = &self.asset_filter {
            regex::Regex::new(filter)?;
        }
        Ok(())
    }
}

/// App ids name files and directories (`~/.local/opt/<id>`,
/// `~/Applications/<id>.AppImage`), so they are limited to letters, digits,
/// `.`, `_` and `-`, and may not start with a dot.
pub fn validate_id(id: &str) -> crate::Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if ok {
        Ok(())
    } else {
        Err(crate::Error::InvalidId(id.to_string()))
    }
}

/// How an app's assets are signed, with the key the user trusts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Signature {
    /// A minisign public key, base64.
    Minisign { public_key: String },
    /// A keyring holding the trusted OpenPGP key, as stored by Forager.
    Gpg { keyring: PathBuf },
    /// A GitHub artifact attestation from the app's own repository.
    Attestation,
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Signature::Minisign { .. } => "minisign",
            Signature::Gpg { .. } => "gpg",
            Signature::Attestation => "attestation",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    pub tag: String,
    pub format: Format,
    pub asset: String,
    /// Package name as known to the package manager (deb/rpm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Install location for formats Forager places itself (AppImage, tarball).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Symlinks Forager created for a tarball's executables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ids() {
        for id in ["ripgrep", "gear-lever", "org.example.Tool_2"] {
            assert!(validate_id(id).is_ok(), "{id}");
        }
        for id in ["", "../x", "a/b", ".hidden", "with space", &"x".repeat(65)] {
            assert!(validate_id(id).is_err(), "{id}");
        }
    }

    #[test]
    fn classifies_file_names() {
        for (name, format) in [
            ("tool_1.0_amd64.deb", Some(Format::Deb)),
            ("tool-1.0-1.x86_64.rpm", Some(Format::Rpm)),
            ("tool-1.0-1.src.rpm", None),
            ("Tool.AppImage", Some(Format::AppImage)),
            ("tool-x86_64.flatpak", Some(Format::Flatpak)),
            ("tool-1.0-1-x86_64.pkg.tar.zst", Some(Format::Pacman)),
            ("tool-1.0-1-any.pkg.tar.xz", Some(Format::Pacman)),
            ("Tool-1.0-linux-x64.pacman", Some(Format::Pacman)),
            ("tool-1.0-1-x86_64.pkg.tar.zst.sig", None),
            ("tool-linux-x86_64.tar.zst", Some(Format::Tarball)),
            ("tool.tgz", Some(Format::Tarball)),
            ("SHA256SUMS", None),
        ] {
            assert_eq!(Format::from_file_name(name), format, "{name}");
        }
    }
}
