//! SourceHut git repositories: tags and their notes from the refs feed, and
//! the files attached to a tag ("artifacts") from its page. Neither needs a
//! token.

use std::sync::LazyLock;

use regex::Regex;

use crate::error::Result;
use crate::http;
use crate::model::{Asset, Release};

static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<item>(.*?)</item>").unwrap());
static ARTIFACT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="(/~[^"]+/refs/download/[^"]+)""#).unwrap());

/// Tags whose pages are fetched for their files: older ones are not
/// installed, so their files need not be known.
const WITH_FILES: usize = 5;

pub fn releases(owner: &str, repo: &str) -> Result<Vec<Release>> {
    let base = format!("https://git.sr.ht/~{owner}/{repo}");
    let feed = http::get_cached(
        http::get(&format!("{base}/refs/rss.xml")),
        &format!("sourcehut/{owner}__{repo}.xml"),
    )?;
    let mut releases = parse_feed(&feed);
    for release in releases.iter_mut().take(WITH_FILES) {
        let tag = url::form_urlencoded::byte_serialize(release.tag.as_bytes()).collect::<String>();
        let key = format!("sourcehut/{owner}__{repo}__{tag}.html");
        let page = http::get_cached(http::get(&format!("{base}/refs/{tag}")), &key)?;
        release.assets = parse_artifacts(&page);
    }
    Ok(releases)
}

/// Tags, newest first as the feed lists them.
pub(super) fn parse_feed(feed: &str) -> Vec<Release> {
    ITEM.captures_iter(feed)
        .filter_map(|item| {
            let item = &item[1];
            let tag = super::element(item, "title")?;
            Some(Release {
                prerelease: super::looks_prerelease(&tag),
                name: None,
                published_at: super::element(item, "pubDate").and_then(|d| super::iso_date(&d)),
                notes: super::element(item, "description").filter(|n| !n.trim().is_empty()),
                tag,
                assets: Vec::new(),
            })
        })
        .collect()
}

pub(super) fn parse_artifacts(page: &str) -> Vec<Asset> {
    let mut assets: Vec<Asset> = Vec::new();
    for caps in ARTIFACT.captures_iter(page) {
        let url = format!("https://git.sr.ht{}", caps[1].replace("&amp;", "&"));
        let name = url.rsplit('/').next().unwrap_or_default().replace("%20", " ");
        if !assets.iter().any(|a| a.url == url) {
            assets.push(Asset {
                name,
                url,
                size: 0,
                digest: None,
            });
        }
    }
    assets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tags_and_their_files() {
        let feed = "<rss><channel><title>~u/tool refs</title>\
            <item><title>v1.7.0</title><description>tool 1.7.0\n\nFixes &amp; more</description>\
            <pubDate>Tue, 07 Jul 2026 09:07:05 +0200</pubDate></item>\
            <item><title>v1.6.0</title><description></description></item></channel></rss>";
        let releases = parse_feed(feed);
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].tag, "v1.7.0");
        assert_eq!(releases[0].notes.as_deref(), Some("tool 1.7.0\n\nFixes & more"));
        assert_eq!(releases[0].published_at.as_deref(), Some("2026-07-07T07:07:05Z"));
        assert_eq!(releases[1].notes, None);

        let page = r#"<a href="/~u/tool/refs/download/v1.7.0/tool-1.7.0.tar.gz">tool-1.7.0.tar.gz</a>
            <a href="/~u/tool/refs/download/v1.7.0/tool-1.7.0.tar.gz.sig">sig</a>
            <a href="/~u/tool/refs/download/v1.7.0/tool-1.7.0.tar.gz">again</a>"#;
        let assets = parse_artifacts(page);
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].name, "tool-1.7.0.tar.gz");
        assert_eq!(
            assets[0].url,
            "https://git.sr.ht/~u/tool/refs/download/v1.7.0/tool-1.7.0.tar.gz"
        );
    }
}
