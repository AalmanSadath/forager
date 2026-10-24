use serde::Deserialize;

use crate::error::Result;
use crate::http;
use crate::model::{Asset, Release};

const API: &str = "https://api.github.com";

/// Release as returned by GitHub, and by Forgejo and Gitea, which copy its
/// API (without `digest`).
#[derive(Deserialize)]
pub(super) struct ApiRelease {
    tag_name: String,
    name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<String>,
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    digest: Option<String>,
}

/// Fetches recent releases, newest first. Uses `GITHUB_TOKEN` when set to
/// lift the unauthenticated limit of 60 requests per hour.
///
/// GitHub answers an unchanged list with a 304. With a token, 304s do not
/// count against the rate limit; without one they still do.
pub fn releases(owner: &str, repo: &str) -> Result<Vec<Release>> {
    let url = format!("{API}/repos/{owner}/{repo}/releases?per_page=20");
    let mut request = http::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = http::token_for("github.com", Some(("github.com", "GITHUB_TOKEN"))) {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let body = http::get_cached(request, &format!("github/{owner}__{repo}.json"))?;
    parse_releases(&body)
}

pub(super) fn parse_releases(body: &str) -> Result<Vec<Release>> {
    let releases: Vec<ApiRelease> = serde_json::from_str(body)?;
    Ok(releases
        .into_iter()
        .filter(|r| !r.draft)
        .map(|r| Release {
            tag: r.tag_name,
            name: r.name,
            prerelease: r.prerelease,
            published_at: r.published_at,
            notes: r.body.filter(|b| !b.trim().is_empty()),
            assets: r
                .assets
                .into_iter()
                .map(|a| Asset {
                    name: a.name,
                    url: a.browser_download_url,
                    size: a.size,
                    digest: a.digest,
                })
                .collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_releases_and_skips_drafts() {
        let body = r###"[
            {"tag_name": "v2", "name": null, "draft": true, "prerelease": false, "published_at": null, "assets": []},
            {"tag_name": "v1", "name": "One", "draft": false, "prerelease": false, "published_at": null,
             "body": "## Fixed\n- a bug",
             "assets": [{"name": "tool.rpm", "browser_download_url": "https://x/tool.rpm", "size": 3,
                         "digest": "sha256:ab"}]}
        ]"###;
        let releases = parse_releases(body).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "v1");
        assert_eq!(releases[0].assets[0].digest.as_deref(), Some("sha256:ab"));
        assert_eq!(releases[0].notes.as_deref(), Some("## Fixed\n- a bug"));
    }
}
