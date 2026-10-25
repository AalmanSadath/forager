//! SourceForge projects, through the RSS feed of their file area: each file
//! is an item, grouped into one release per folder (`/1.2.3/tool.AppImage`).

use std::sync::LazyLock;

use regex::Regex;

use crate::error::Result;
use crate::http;
use crate::model::{Asset, Release};

static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<item>(.*?)</item>").unwrap());
static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"filesize="(\d+)""#).unwrap());

pub fn releases(project: &str) -> Result<Vec<Release>> {
    let url = format!("https://sourceforge.net/projects/{project}/rss?path=/&limit=100");
    let request = http::get(&url).header("Accept", "application/rss+xml");
    let body = http::get_cached(request, &format!("sourceforge/{project}.xml"))?;
    Ok(parse(&body))
}

pub(super) fn parse(feed: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    for item in ITEM.captures_iter(feed) {
        let item = &item[1];
        let (Some(path), Some(link)) = (super::element(item, "title"), super::element(item, "link")) else {
            continue;
        };
        let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let Some(name) = parts.pop() else { continue };
        // The folder names the version (`/SMPlayer/23.5.0/`); a flat file
        // area leaves the file name.
        let Some(tag) = parts
            .iter()
            .rev()
            .find(|p| p.chars().any(|c| c.is_ascii_digit()))
            .map(|p| p.to_string())
            .or_else(|| super::version_in(name))
        else {
            continue;
        };
        let asset = Asset {
            name: name.to_string(),
            url: link,
            size: SIZE.captures(item).and_then(|c| c[1].parse().ok()).unwrap_or(0),
            // The feed has MD5 sums only, which Forager does not trust.
            digest: None,
        };
        match releases.iter_mut().find(|r| r.tag == tag) {
            Some(release) => release.assets.push(asset),
            None => releases.push(Release {
                prerelease: super::looks_prerelease(&tag),
                name: None,
                published_at: super::element(item, "pubDate").and_then(|d| super::iso_date(&d)),
                notes: None,
                tag,
                assets: vec![asset],
            }),
        }
    }
    super::sort_newest_first(&mut releases);
    releases
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<rss><channel><title>SMPlayer</title>
        <item><title><![CDATA[/SMPlayer/23.5.0/SMPlayer-23.5.0-x86_64.AppImage]]></title>
          <link>https://sourceforge.net/projects/smplayer/files/SMPlayer/23.5.0/SMPlayer-23.5.0-x86_64.AppImage/download</link>
          <pubDate>Mon, 13 May 2024 12:28:15 UT</pubDate>
          <media:content url="x" filesize="5221961"><media:hash algo="md5">d4</media:hash></media:content></item>
        <item><title><![CDATA[/SMPlayer/23.5.0/README.md]]></title>
          <link>https://sourceforge.net/projects/smplayer/files/SMPlayer/23.5.0/README.md/download</link>
          <pubDate>Mon, 13 May 2024 12:28:10 UT</pubDate></item>
        <item><title><![CDATA[/SMPlayer/24.0.0-rc1/SMPlayer-24.0.0-rc1-x86_64.AppImage]]></title>
          <link>https://sourceforge.net/x/download</link></item>
        <item><title><![CDATA[/SMPlayer/22.2.0/SMPlayer-22.2.0-x86_64.AppImage]]></title>
          <link>https://sourceforge.net/y/download</link></item>
        <item><title><![CDATA[/tool-1.0.tar.gz]]></title><link>https://sourceforge.net/z/download</link></item>
    </channel></rss>"#;

    #[test]
    fn groups_files_by_folder() {
        let releases = parse(FEED);
        let tags: Vec<&str> = releases.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(tags, ["24.0.0-rc1", "23.5.0", "22.2.0", "1.0"]);
        assert!(releases[0].prerelease);
        let current = &releases[1];
        assert_eq!(current.assets.len(), 2);
        assert_eq!(current.assets[0].name, "SMPlayer-23.5.0-x86_64.AppImage");
        assert_eq!(current.assets[0].size, 5221961);
        assert!(
            current.assets[0]
                .url
                .ends_with("/SMPlayer-23.5.0-x86_64.AppImage/download")
        );
        assert_eq!(current.published_at.as_deref(), Some("2024-05-13T12:28:15Z"));
    }
}
