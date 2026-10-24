use serde::Deserialize;

use crate::error::Result;
use crate::model::{Asset, Release};

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
pub fn releases(owner: &str, repo: &str) -> Result<Vec<Release>> {
    let url = format!("{API}/repos/{owner}/{repo}/releases?per_page=20");
    let mut request = ureq::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", concat!("forager/", env!("CARGO_PKG_VERSION")));
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let releases: Vec<ApiRelease> = request.call()?.body_mut().read_json()?;

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
