use serde::Deserialize;

use crate::error::Result;
use crate::http;
use crate::model::{Asset, Release};

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    name: Option<String>,
    /// Release whose date lies in the future.
    #[serde(default)]
    upcoming_release: bool,
    released_at: Option<String>,
    assets: ApiAssets,
}

#[derive(Deserialize)]
struct ApiAssets {
    #[serde(default)]
    links: Vec<ApiLink>,
}

#[derive(Deserialize)]
struct ApiLink {
    name: String,
    url: String,
    direct_asset_url: Option<String>,
}

/// Fetches recent releases of project `path` (`group/subgroup/project`) on
/// `host`, newest first. A token comes from `FORAGER_TOKEN_<HOST>`, or
/// `GITLAB_TOKEN` for gitlab.com.
pub fn releases(host: &str, path: &str) -> Result<Vec<Release>> {
    let url = format!("https://{host}/api/v4/projects/{}/releases?per_page=20", encode(path));
    let mut request = http::get(&url);
    if let Some(token) = http::token_for(host, Some(("gitlab.com", "GITLAB_TOKEN"))) {
        request = request.header("PRIVATE-TOKEN", token);
    }
    let body = http::get_cached(request, &format!("gitlab/{host}__{}.json", path.replace('/', "__")))?;
    parse_releases(&body)
}

fn parse_releases(body: &str) -> Result<Vec<Release>> {
    let releases: Vec<ApiRelease> = serde_json::from_str(body)?;
    Ok(releases
        .into_iter()
        .map(|r| Release {
            tag: r.tag_name,
            name: r.name,
            prerelease: r.upcoming_release,
            published_at: r.released_at,
            assets: r
                .assets
                .links
                .into_iter()
                .map(|link| Asset {
                    name: link.name,
                    url: link.direct_asset_url.unwrap_or(link.url),
                    // GitLab does not report sizes or digests for release links.
                    size: 0,
                    digest: None,
                })
                .collect(),
        })
        .collect())
}

/// Percent-encodes a project path for use as a single URL segment.
fn encode(path: &str) -> String {
    path.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_links() {
        let body = r#"[{"name": "v1.2", "tag_name": "v1.2", "upcoming_release": false,
            "released_at": "2026-01-01T00:00:00Z",
            "assets": {"count": 3, "sources": [{"format": "zip", "url": "https://x/src.zip"}],
                       "links": [{"id": 1, "name": "tool_1.2_linux_amd64.deb", "url": "https://x/l/1",
                                  "direct_asset_url": "https://x/-/releases/v1.2/downloads/tool.deb",
                                  "link_type": "package"},
                                 {"id": 2, "name": "tool.AppImage", "url": "https://x/l/2"}]}}]"#;
        let releases = parse_releases(body).unwrap();
        assert_eq!(releases[0].tag, "v1.2");
        let assets = &releases[0].assets;
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].url, "https://x/-/releases/v1.2/downloads/tool.deb");
        assert_eq!(assets[1].url, "https://x/l/2");
    }

    #[test]
    fn encodes_subgroup_paths() {
        assert_eq!(encode("group/sub/my.project"), "group%2Fsub%2Fmy.project");
    }
}
