use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::{Asset, Release};
use crate::{http, store};

const API: &str = "https://api.github.com";

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    name: Option<String>,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// Fetches recent releases, newest first. Uses `GITHUB_TOKEN` when set to
/// lift the unauthenticated limit of 60 requests per hour.
///
/// The last response is cached with its ETag, and GitHub answers an
/// unchanged list with an empty 304. With a token, 304s do not count against
/// the rate limit; without one they still do.
pub fn releases(owner: &str, repo: &str) -> Result<Vec<Release>> {
    let url = format!("{API}/repos/{owner}/{repo}/releases?per_page=20");
    let cache_path = store::cache_dir().join("github").join(format!("{owner}__{repo}.json"));
    let cached = load_cache(&cache_path);

    let mut request = http::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    if let Some(cached) = &cached {
        request = request.header("If-None-Match", &cached.etag);
    }
    let mut response = http::check(request.call()?)?;

    let body = match cached {
        Some(cached) if response.status() == 304 => cached.body,
        _ => {
            let etag = response
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let body = response.body_mut().with_config().limit(BODY_LIMIT).read_to_string()?;
            if let Some(etag) = etag {
                // Best effort: without the cache the next check just costs a request.
                save_cache(
                    &cache_path,
                    &Cached {
                        etag,
                        body: body.clone(),
                    },
                )
                .ok();
            }
            body
        }
    };
    parse_releases(&body)
}

/// Twenty releases with many assets can pass ureq's default 10 MB limit.
const BODY_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Cached {
    etag: String,
    body: String,
}

fn load_cache(path: &Path) -> Option<Cached> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn save_cache(path: &Path, cached: &Cached) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string(cached)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn parse_releases(body: &str) -> Result<Vec<Release>> {
    let releases: Vec<ApiRelease> = serde_json::from_str(body)?;
    Ok(releases
        .into_iter()
        .filter(|r| !r.draft)
        .map(|r| Release {
            tag: r.tag_name,
            name: r.name,
            prerelease: r.prerelease,
            published_at: r.published_at,
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
        let body = r#"[
            {"tag_name": "v2", "name": null, "draft": true, "prerelease": false, "published_at": null, "assets": []},
            {"tag_name": "v1", "name": "One", "draft": false, "prerelease": false, "published_at": null,
             "assets": [{"name": "tool.rpm", "browser_download_url": "https://x/tool.rpm", "size": 3,
                         "digest": "sha256:ab"}]}
        ]"#;
        let releases = parse_releases(body).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "v1");
        assert_eq!(releases[0].assets[0].digest.as_deref(), Some("sha256:ab"));
    }

    #[test]
    fn cache_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("github/o__r.json");
        assert!(load_cache(&path).is_none());
        save_cache(
            &path,
            &Cached {
                etag: "W/\"abc\"".into(),
                body: "[]".into(),
            },
        )
        .unwrap();
        let cached = load_cache(&path).unwrap();
        assert_eq!(cached.etag, "W/\"abc\"");
        assert_eq!(cached.body, "[]");
    }
}
