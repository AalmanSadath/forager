use regex::Regex;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::{App, Asset, Format, Release};
use crate::platform::{KNOWN_ARCHES, Platform, arch_aliases};

/// Why an asset was not picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Not a package format Forager installs (checksums, signatures, zips...).
    NotPackage,
    /// Does not match the app's asset filter.
    Filter,
    Debug,
    OtherOs,
    /// A format this system cannot install, or not the one the app is pinned to.
    FormatNotWanted,
    OtherArch,
    /// A tarball naming neither Linux nor this architecture, usually source code.
    SourceArchive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum Verdict {
    Chosen,
    /// Installable, but a better match was chosen.
    Usable,
    Rejected(Reason),
}

/// One release asset and what the matcher made of it.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub asset: &'a Asset,
    pub format: Option<Format>,
    pub verdict: Verdict,
}

/// Every asset of `release` with its verdict, in release order. At most one
/// is `Chosen`.
///
/// Preference order: format (the installed one first, then as ranked by the
/// platform), then an asset naming this architecture over one naming none.
/// Assets naming another architecture or operating system, debug packages
/// and source packages are never picked.
/// A tarball must name Linux or this architecture, since one naming neither is
/// usually a source archive.
pub fn candidates<'a>(app: &App, release: &'a Release, platform: &Platform) -> Result<Vec<Candidate<'a>>> {
    let filter = app.asset_filter.as_deref().map(Regex::new).transpose()?;
    let formats = match app.format {
        Some(format) => vec![format],
        // Updates stay in the format already installed when it still suits
        // the system, so a Flatpak is not replaced by the distro package.
        None => {
            let mut formats = platform.formats();
            if let Some(installed) = &app.installed
                && let Some(index) = formats.iter().position(|f| *f == installed.format)
            {
                let format = formats.remove(index);
                formats.insert(0, format);
            }
            formats
        }
    };

    let mut best: Option<(usize, (usize, bool))> = None;
    let mut list: Vec<Candidate> = release
        .assets
        .iter()
        .enumerate()
        .map(|(index, asset)| {
            let format = Format::from_file_name(&asset.name);
            let verdict = match judge(asset, format, filter.as_ref(), &formats, platform) {
                Ok(score) => {
                    if best.is_none_or(|(_, best_score)| score < best_score) {
                        best = Some((index, score));
                    }
                    Verdict::Usable
                }
                Err(reason) => Verdict::Rejected(reason),
            };
            Candidate { asset, format, verdict }
        })
        .collect();
    if let Some((index, _)) = best {
        list[index].verdict = Verdict::Chosen;
    }
    Ok(list)
}

/// Score of an installable asset (lower is better), or why it is not one.
fn judge(
    asset: &Asset,
    format: Option<Format>,
    filter: Option<&Regex>,
    formats: &[Format],
    platform: &Platform,
) -> std::result::Result<(usize, bool), Reason> {
    let format = format.ok_or(Reason::NotPackage)?;
    if filter.is_some_and(|re| !re.is_match(&asset.name)) {
        return Err(Reason::Filter);
    }
    if is_debug(&asset.name) {
        return Err(Reason::Debug);
    }
    if names_other_os(&asset.name) {
        return Err(Reason::OtherOs);
    }
    let rank = formats
        .iter()
        .position(|f| *f == format)
        .ok_or(Reason::FormatNotWanted)?;
    let arch = arch_match(&asset.name, platform.arch).ok_or(Reason::OtherArch)?;
    if format == Format::Tarball && !arch && !asset.name.to_ascii_lowercase().contains("linux") {
        return Err(Reason::SourceArchive);
    }
    Ok((rank, !arch))
}

/// Picks the release asset that best fits the app's settings and the
/// platform; see `candidates` for the rules.
pub fn select_asset<'a>(app: &App, release: &'a Release, platform: &Platform) -> Result<(&'a Asset, Format)> {
    candidates(app, release, platform)?
        .into_iter()
        .find(|c| c.verdict == Verdict::Chosen)
        .and_then(|c| Some((c.asset, c.format?)))
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

fn names_other_os(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "darwin", "macos", "osx", "mac", "apple", "windows", "win", "win32", "win64", "freebsd", "openbsd", "netbsd",
        "android", "ios", "illumos", "solaris",
    ]
    .iter()
    .any(|os| contains_word(&lower, os))
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
            notes: None,
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
            tag_filter: None,
            format: None,
            prerelease: false,
            bins: Vec::new(),
            signature: None,
            installed: None,
            also_installed: Vec::new(),
        }
    }

    fn platform(pm: Option<PackageManager>) -> Platform {
        Platform {
            arch: "x86_64",
            package_manager: pm,
            flatpak: false,
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

    /// Forager's own release files, as `packaging/build-all.sh` names them:
    /// tracking itself must pick the package matching each system.
    #[test]
    fn picks_each_of_foragers_own_packages() {
        let release = release(&[
            "forager_0.1.0_amd64.deb",
            "forager-0.1.0-1.x86_64.rpm",
            "forager-0.1.0-1-x86_64.pkg.tar.zst",
            "forager-0.1.0-x86_64.flatpak",
            "Forager-0.1.0-x86_64.AppImage",
            "forager-0.1.0-x86_64-linux.tar.gz",
            "SHA256SUMS",
        ]);
        let pick = |pm, flatpak| {
            let platform = Platform {
                flatpak,
                ..platform(pm)
            };
            select_asset(&app(), &release, &platform).unwrap().0.name.clone()
        };
        assert_eq!(pick(Some(PackageManager::Dnf), true), "forager-0.1.0-1.x86_64.rpm");
        assert_eq!(pick(Some(PackageManager::Zypper), false), "forager-0.1.0-1.x86_64.rpm");
        assert_eq!(pick(Some(PackageManager::Apt), true), "forager_0.1.0_amd64.deb");
        assert_eq!(
            pick(Some(PackageManager::Pacman), true),
            "forager-0.1.0-1-x86_64.pkg.tar.zst"
        );
        assert_eq!(pick(None, true), "forager-0.1.0-x86_64.flatpak");
        assert_eq!(pick(None, false), "Forager-0.1.0-x86_64.AppImage");
    }

    #[test]
    fn automatic_format_keeps_the_installed_one() {
        let mut app = app();
        app.installed = Some(crate::model::Installed {
            tag: "v0.9.0".into(),
            format: Format::AppImage,
            asset: String::new(),
            package: None,
            path: None,
            links: Vec::new(),
        });
        assert_eq!(pick(&app, Some(PackageManager::Dnf)), "Tool-1.0.0-x86_64.AppImage");
        // Unless the installed format no longer suits the system.
        app.installed.as_mut().unwrap().format = Format::Pacman;
        assert_eq!(pick(&app, Some(PackageManager::Dnf)), "tool-1.0.0-1.x86_64.rpm");
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

    #[test]
    fn picks_linux_tarball_for_this_arch() {
        let release = release(&[
            "tool-1.0.0.tar.gz",
            "tool-1.0.0-x86_64-apple-darwin.tar.gz",
            "tool-1.0.0-x86_64-pc-windows-msvc.tar.gz",
            "tool-1.0.0-aarch64-unknown-linux-gnu.tar.gz",
            "tool-1.0.0-x86_64-unknown-linux-musl.tar.gz",
            "tool-1.0.0-x86_64-unknown-linux-musl.tar.gz.sha256",
        ]);
        let (asset, format) = select_asset(&app(), &release, &platform(Some(PackageManager::Dnf))).unwrap();
        assert_eq!(asset.name, "tool-1.0.0-x86_64-unknown-linux-musl.tar.gz");
        assert_eq!(format, Format::Tarball);
    }

    #[test]
    fn rejects_tarball_naming_neither_linux_nor_arch() {
        let release = release(&["tool-1.0.0.tar.gz", "tool-1.0.0-src.tar.xz"]);
        assert!(select_asset(&app(), &release, &platform(None)).is_err());

        let release = self::release(&["tool-linux.tar.xz"]);
        assert!(select_asset(&app(), &release, &platform(None)).is_ok());
    }

    #[test]
    fn prefers_appimage_over_tarball() {
        let release = release(&["tool-linux-x86_64.tar.gz", "Tool-x86_64.AppImage"]);
        let (asset, _) = select_asset(&app(), &release, &platform(None)).unwrap();
        assert_eq!(asset.name, "Tool-x86_64.AppImage");
    }

    #[test]
    fn prefers_flatpak_over_appimage_when_available() {
        let release = release(&["tool-x86_64.flatpak", "Tool-x86_64.AppImage"]);
        let mut platform = platform(Some(PackageManager::Dnf));
        assert_eq!(select_asset(&app(), &release, &platform).unwrap().1, Format::AppImage);
        platform.flatpak = true;
        assert_eq!(select_asset(&app(), &release, &platform).unwrap().1, Format::Flatpak);
    }

    #[test]
    fn rejects_32_bit_builds_named_386() {
        let release = release(&["glab_1.0_linux_386.tar.gz", "glab_1.0_linux_armv6.tar.gz"]);
        assert!(select_asset(&app(), &release, &platform(None)).is_err());
    }

    #[test]
    fn explains_each_verdict() {
        let release = release(&[
            "SHA256SUMS",
            "tool-debuginfo-1.0.0-1.x86_64.rpm",
            "tool-1.0.0-x86_64-apple-darwin.tar.gz",
            "tool_1.0.0_amd64.deb",
            "tool-1.0.0-1.aarch64.rpm",
            "tool-1.0.0.tar.gz",
            "tool-1.0.0-1.x86_64.rpm",
            "Tool-1.0.0-x86_64.AppImage",
        ]);
        let verdicts: Vec<Verdict> = candidates(&app(), &release, &platform(Some(PackageManager::Dnf)))
            .unwrap()
            .into_iter()
            .map(|c| c.verdict)
            .collect();
        use Reason::*;
        assert_eq!(
            verdicts,
            [
                Verdict::Rejected(NotPackage),
                Verdict::Rejected(Debug),
                Verdict::Rejected(OtherOs),
                Verdict::Rejected(FormatNotWanted),
                Verdict::Rejected(OtherArch),
                Verdict::Rejected(SourceArchive),
                Verdict::Chosen,
                Verdict::Usable,
            ]
        );

        let mut filtered = app();
        filtered.asset_filter = Some("AppImage".into());
        let first = candidates(&filtered, &release, &platform(None)).unwrap();
        assert_eq!(first[6].verdict, Verdict::Rejected(Filter));
        assert_eq!(first[7].verdict, Verdict::Chosen);
    }
}
