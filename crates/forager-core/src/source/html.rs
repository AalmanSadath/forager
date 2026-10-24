//! A web page that links to downloads, for projects without a forge: the
//! links whose address matches the app's pattern are its assets, grouped
//! into one release per version.

use std::sync::LazyLock;

use regex::Regex;
use url::Url;

use crate::error::{Error, Result};
use crate::model::{Asset, Release};
use crate::{http, verify, version};

static HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)href\s*=\s*["']([^"'#]+)"#).unwrap());
/// A version in a file name: `1.2`, `1.2.3`, `2024.10.1`.
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)+").unwrap());

/// Files next to an asset that verify it.
const SIDECARS: [&str; 5] = [".sha256", ".sha256sum", ".minisig", ".asc", ".sig"];

pub fn releases(page: &str, pattern: &str) -> Result<Vec<Release>> {
    let key: String = page
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let request = http::get(page).header("Accept", "text/html,application/xhtml+xml");
    let html = http::get_cached(request, &format!("html/{key}.json"))?;
    parse(page, &html, pattern)
}

/// Checks a page address and pattern as given for a new app.
pub fn validate(page: &str, pattern: &str) -> Result<()> {
    let url = Url::parse(page).map_err(|e| Error::UnsupportedSource(format!("{page}: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::UnsupportedSource(page.to_string()));
    }
    Regex::new(pattern).map_err(Error::InvalidPattern)?;
    Ok(())
}

pub(super) fn parse(page: &str, html: &str, pattern: &str) -> Result<Vec<Release>> {
    let base = Url::parse(page).map_err(|e| Error::UnsupportedSource(format!("{page}: {e}")))?;
    let pattern = Regex::new(pattern).map_err(Error::InvalidPattern)?;

    let mut links: Vec<Url> = Vec::new();
    for caps in HREF.captures_iter(html) {
        let href = caps[1].trim().replace("&amp;", "&");
        if let Ok(url) = base.join(&href)
            && matches!(url.scheme(), "http" | "https")
            && !links.contains(&url)
        {
            links.push(url);
        }
    }
    let name_of = |url: &Url| {
        url.path_segments()
            .and_then(|mut s| s.next_back())
            .map(|n| n.replace("%20", " "))
            .unwrap_or_default()
    };

    // Checksum lists on the page cover whatever they name, in any release.
    let lists: Vec<Asset> = links
        .iter()
        .filter(|u| verify::is_checksum_list(&name_of(u).to_ascii_lowercase()))
        .map(|u| asset(name_of(u), u))
        .collect();

    let mut releases: Vec<Release> = Vec::new();
    for url in &links {
        let full = url.as_str();
        let Some(caps) = pattern.captures(full) else { continue };
        let name = name_of(url);
        let Some(tag) = caps
            .name("version")
            .map(|m| m.as_str().to_string())
            .or_else(|| NUMBER.find(&name).map(|m| m.as_str().to_string()))
        else {
            continue;
        };
        let release = match releases.iter_mut().position(|r| r.tag == tag) {
            Some(i) => &mut releases[i],
            None => {
                let lower = tag.to_ascii_lowercase();
                releases.push(Release {
                    prerelease: ["alpha", "beta", "rc", "pre", "dev"].iter().any(|p| lower.contains(p)),
                    tag: tag.clone(),
                    name: None,
                    published_at: None,
                    assets: lists.clone(),
                });
                releases.last_mut().unwrap()
            }
        };
        for suffix in SIDECARS {
            let sidecar = format!("{full}{suffix}");
            if let Some(found) = links.iter().find(|u| u.as_str() == sidecar) {
                release.assets.push(asset(format!("{name}{suffix}"), found));
            }
        }
        release.assets.push(asset(name, url));
    }

    // Newest first, by version where the tags parse as one.
    releases.sort_by(|a, b| match (version::parse_tag(&a.tag), version::parse_tag(&b.tag)) {
        (Some(x), Some(y)) => y.cmp(&x),
        _ => b.tag.cmp(&a.tag),
    });
    Ok(releases)
}

fn asset(name: String, url: &Url) -> Asset {
    Asset {
        name,
        url: url.to_string(),
        // A page does not say how large its files are.
        size: 0,
        digest: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r##"<html><body>
        <a href="tool-2.1.0-x86_64.AppImage">2.1.0</a>
        <a href="tool-2.1.0-x86_64.AppImage.sha256">sha</a>
        <a href='/files/tool-2.0.9-x86_64.AppImage'>2.0.9</a>
        <a href="tool-2.2.0-rc1-x86_64.AppImage">rc</a>
        <a href="https://cdn.example.org/tool-1.9.0-x86_64.AppImage?x=1&amp;y=2">old</a>
        <a href="SHA256SUMS">sums</a>
        <a href="tool-2.1.0-x86_64.AppImage">duplicate</a>
        <a href="other-9.9.9.zip">unrelated</a>
        <a href="#top">anchor</a>
    </body></html>"##;

    #[test]
    fn groups_matching_links_into_releases() {
        let releases = parse("https://example.org/tool/download.html", PAGE, r"tool-.*\.AppImage$").unwrap();
        let tags: Vec<&str> = releases.iter().map(|r| r.tag.as_str()).collect();
        // 2.2.0-rc1 is read as 2.2.0 without a named group: the number alone.
        assert_eq!(tags, ["2.2.0", "2.1.0", "2.0.9"]);

        let current = &releases[1];
        let names: Vec<&str> = current.assets.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "SHA256SUMS",
                "tool-2.1.0-x86_64.AppImage.sha256",
                "tool-2.1.0-x86_64.AppImage"
            ]
        );
        assert_eq!(
            current.assets[2].url,
            "https://example.org/tool/tool-2.1.0-x86_64.AppImage"
        );
        assert_eq!(
            releases[2].assets[1].url,
            "https://example.org/files/tool-2.0.9-x86_64.AppImage"
        );
    }

    #[test]
    fn takes_the_version_from_a_named_group() {
        let releases = parse(
            "https://example.org/",
            PAGE,
            r"tool-(?P<version>[\d.]+(?:-rc\d+)?)-x86_64\.AppImage(\?.*)?$",
        )
        .unwrap();
        let rc = releases.iter().find(|r| r.tag == "2.2.0-rc1").unwrap();
        assert!(rc.prerelease);
        // The query string survives, with &amp; decoded.
        let old = releases.iter().find(|r| r.tag == "1.9.0").unwrap();
        assert_eq!(
            old.assets.last().unwrap().url,
            "https://cdn.example.org/tool-1.9.0-x86_64.AppImage?x=1&y=2"
        );
    }

    #[test]
    fn validates_page_and_pattern() {
        assert!(validate("https://example.org/dl", "tool").is_ok());
        assert!(validate("ftp://example.org/dl", "tool").is_err());
        assert!(validate("not a url", "tool").is_err());
        assert!(validate("https://example.org/dl", "(").is_err());
    }
}
