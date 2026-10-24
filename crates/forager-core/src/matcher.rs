use regex::Regex;

use crate::error::{Error, Result};
use crate::model::{App, Asset, Format, Release};
use crate::platform::{KNOWN_ARCHES, Platform, arch_aliases};

/// Picks the release asset that best fits the app's settings and the platform.
///
/// Preference order: format (as ranked by the platform), then an asset naming
/// this architecture over one naming none. Assets naming another architecture,
/// debug packages and source packages are never picked.
pub fn select_asset<'a>(app: &App, release: &'a Release, platform: &Platform) -> Result<(&'a Asset, Format)> {
    let filter = app.asset_filter.as_deref().map(Regex::new).transpose()?;
    let formats = match app.format {
        Some(format) => vec![format],
        None => platform.formats(),
    };

    release
        .assets
        .iter()
        .filter(|a| filter.as_ref().is_none_or(|re| re.is_match(&a.name)))
        .filter(|a| !is_debug(&a.name))
        .filter_map(|a| {
            let format = Format::from_file_name(&a.name)?;
            let rank = formats.iter().position(|f| *f == format)?;
            let arch = arch_match(&a.name, platform.arch)?;
            Some((a, format, (rank, !arch)))
        })
        .min_by_key(|(_, _, score)| *score)
        .map(|(a, format, _)| (a, format))
        .ok_or_else(|| Error::NoMatchingAsset {
            tag: release.tag.clone(),
        })
}

/// `Some(true)` if the name mentions `arch`, `Some(false)` if it mentions no
/// architecture, `None` if it mentions another one.
fn arch_match(name: &str, arch: &str) -> Option<bool> {
    let name = name.to_ascii_lowercase();
    let mentions = |a: &str| arch_aliases(a).iter().any(|alias| contains_word(&name, alias));
    if mentions(arch) {
        Some(true)
    } else if KNOWN_ARCHES.iter().any(|a| *a != arch && mentions(a)) {
        None
    } else {
        Some(false)
    }
}

/// True if `word` occurs in `haystack` without letters or digits on either side.
fn contains_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

fn is_debug(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["debuginfo", "debugsource", "dbgsym", "-dbg_", "-dbg."]
        .iter()
        .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::PackageManager;
    use crate::source::Source;

    fn release(names: &[&str]) -> Release {
        Release {
            tag: "v1.0.0".into(),
            name: None,
            prerelease: false,
            published_at: None,
            assets: names
                .iter()
                .map(|n| Asset {
                    name: n.to_string(),
                    url: String::new(),
                    size: 0,
                    digest: None,
                })
                .collect(),
        }
    }

    fn app() -> App {
        App {
            id: "tool".into(),
            source: Source::GitHub {
                owner: "o".into(),
                repo: "tool".into(),
            },
            asset_filter: None,
            format: None,
            prerelease: false,
            installed: None,
        }
    }

    fn platform(pm: Option<PackageManager>) -> Platform {
        Platform {
            arch: "x86_64",
            package_manager: pm,
        }
    }

    const ASSETS: &[&str] = &[
        "tool-1.0.0-1.aarch64.rpm",
        "tool-1.0.0-1.x86_64.rpm",
        "tool-debuginfo-1.0.0-1.x86_64.rpm",
        "tool_1.0.0_amd64.deb",
        "tool_1.0.0_arm64.deb",
        "Tool-1.0.0-x86_64.AppImage",
        "tool-1.0.0.src.rpm",
        "SHA256SUMS",
    ];

    fn pick(app: &App, pm: Option<PackageManager>) -> String {
        let release = release(ASSETS);
        select_asset(app, &release, &platform(pm)).unwrap().0.name.clone()
    }

    #[test]
    fn prefers_native_format_and_arch() {
        assert_eq!(pick(&app(), Some(PackageManager::Dnf)), "tool-1.0.0-1.x86_64.rpm");
        assert_eq!(pick(&app(), Some(PackageManager::Apt)), "tool_1.0.0_amd64.deb");
        assert_eq!(pick(&app(), None), "Tool-1.0.0-x86_64.AppImage");
    }

    #[test]
    fn honours_format_override_and_filter() {
        let mut app = app();
        app.format = Some(Format::AppImage);
        assert_eq!(pick(&app, Some(PackageManager::Dnf)), "Tool-1.0.0-x86_64.AppImage");

        let mut app = self::app();
        app.asset_filter = Some("^nothing$".into());
        let release = release(ASSETS);
        assert!(select_asset(&app, &release, &platform(None)).is_err());
    }

    #[test]
    fn accepts_arch_neutral_but_rejects_foreign_arch() {
        let release = release(&["tool-arm64.AppImage", "tool.AppImage"]);
        let (asset, _) = select_asset(&app(), &release, &platform(None)).unwrap();
        assert_eq!(asset.name, "tool.AppImage");
    }
}
