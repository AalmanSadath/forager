use std::collections::HashMap;

use serde::Deserialize;

use crate::error::Result;
use crate::http;
use crate::model::{Asset, Release};

const API: &str = "https://api.github.com";
const GRAPHQL: &str = "https://api.github.com/graphql";
/// Repositories asked about in one GraphQL request: 25 × 20 releases × 100
/// assets stays well under GitHub's limit of 500,000 nodes per query.
const BATCH: usize = 25;

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
pub fn releases(owner: &str, repo: &str, deep: bool) -> Result<Vec<Release>> {
    let (count, suffix) = if deep { (100, "-deep") } else { (20, "") };
    let url = format!("{API}/repos/{owner}/{repo}/releases?per_page={count}");
    let mut request = http::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = http::token_for("github.com", Some(("github.com", "GITHUB_TOKEN"))) {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let body = http::get_cached(request, &format!("github/{owner}__{repo}{suffix}.json"))?;
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

/// Recent releases of many repositories, `BATCH` per GraphQL request, by
/// `(owner, repo)`. GraphQL needs a token, so without one this is empty.
/// A repository the answer leaves out (not found, or a request that
/// failed) is missing from the result, for the caller to fetch alone; that
/// request then reports the error. Asset digests are not asked for: this
/// is for checking, and updates fetch the release again with them.
pub fn releases_batch(repos: &[(String, String)]) -> HashMap<(String, String), Vec<Release>> {
    let Some(token) = http::token_for("github.com", Some(("github.com", "GITHUB_TOKEN"))) else {
        return HashMap::new();
    };
    let mut found = HashMap::new();
    for chunk in repos.chunks(BATCH) {
        let body = serde_json::json!({ "query": batch_query(chunk) });
        if let Ok(text) = http::post_json(GRAPHQL, &body, &token) {
            found.extend(parse_batch(chunk, &text));
            // Saves asking about each repository's state on its own.
            for ((owner, repo), info) in parse_repo_infos(chunk, &text) {
                super::repo::save(&super::Source::GitHub { owner, repo }, &info);
            }
        }
    }
    found
}

fn batch_query(repos: &[(String, String)]) -> String {
    let fields = "isArchived nameWithOwner releases(first: 20, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { \
                  tagName name isPrerelease isDraft publishedAt description \
                  releaseAssets(first: 100) { nodes { name downloadUrl size } } } }";
    let repos: Vec<String> = repos
        .iter()
        .enumerate()
        .map(|(i, (owner, repo))| {
            // JSON string literals are valid GraphQL strings.
            let (owner, repo) = (serde_json::json!(owner), serde_json::json!(repo));
            format!("r{i}: repository(owner: {owner}, name: {repo}) {{ {fields} }}")
        })
        .collect();
    format!("query {{ {} }}", repos.join(" "))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRelease {
    tag_name: String,
    name: Option<String>,
    #[serde(default)]
    is_prerelease: bool,
    #[serde(default)]
    is_draft: bool,
    published_at: Option<String>,
    description: Option<String>,
    release_assets: GqlNodes<GqlAsset>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlAsset {
    name: String,
    download_url: String,
    #[serde(default)]
    size: u64,
}

#[derive(Deserialize)]
struct GqlNodes<T> {
    nodes: Vec<T>,
}

/// The releases in a GraphQL answer to `batch_query(repos)`, by repository.
fn parse_batch(repos: &[(String, String)], body: &str) -> HashMap<(String, String), Vec<Release>> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return HashMap::new();
    };
    repos
        .iter()
        .enumerate()
        .filter_map(|(i, repo)| {
            let nodes = value["data"][format!("r{i}")]["releases"].get("nodes")?.clone();
            let releases: Vec<GqlRelease> = serde_json::from_value(nodes).ok()?;
            let releases = releases
                .into_iter()
                .filter(|r| !r.is_draft)
                .map(|r| {
                    let mut assets: Vec<Asset> = r
                        .release_assets
                        .nodes
                        .into_iter()
                        .map(|a| Asset {
                            name: a.name,
                            url: a.download_url,
                            size: a.size,
                            digest: None,
                        })
                        .collect();
                    // In REST's order, by name, so ties between equally good
                    // assets go the same way as when updating.
                    assets.sort_by(|a, b| a.name.cmp(&b.name));
                    Release {
                        tag: r.tag_name,
                        name: r.name,
                        prerelease: r.is_prerelease,
                        published_at: r.published_at,
                        notes: r.description.filter(|d| !d.trim().is_empty()),
                        assets,
                    }
                })
                .collect();
            Some((repo.clone(), releases))
        })
        .collect()
}

/// Whether each repository in a GraphQL answer is archived, and its name.
fn parse_repo_infos(repos: &[(String, String)], body: &str) -> Vec<((String, String), super::RepoInfo)> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    repos
        .iter()
        .enumerate()
        .filter_map(|(i, repo)| {
            let node = &value["data"][format!("r{i}")];
            let info = super::RepoInfo {
                archived: node["isArchived"].as_bool()?,
                name: node["nameWithOwner"].as_str()?.to_string(),
            };
            Some((repo.clone(), info))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_each_repository_under_an_alias() {
        let query = batch_query(&[("o".into(), "tool".into()), ("x\"y".into(), "z".into())]);
        assert!(query.starts_with("query { r0: repository(owner: \"o\", name: \"tool\") {"));
        assert!(query.contains("r1: repository(owner: \"x\\\"y\", name: \"z\")"));
        assert!(query.contains("releaseAssets(first: 100)"));
        assert!(query.contains("isArchived nameWithOwner"));
    }

    #[test]
    fn reads_a_batch_answer() {
        let repos = [
            ("o".to_string(), "tool".to_string()),
            ("o".to_string(), "gone".to_string()),
        ];
        let body = r###"{"data": {
            "r0": {"isArchived": true, "nameWithOwner": "new/tool", "releases": {"nodes": [
                {"tagName": "v2", "name": null, "isPrerelease": false, "isDraft": true, "publishedAt": null,
                 "description": "", "releaseAssets": {"nodes": []}},
                {"tagName": "v1", "name": "One", "isPrerelease": true, "isDraft": false,
                 "publishedAt": "2026-10-25T08:50:19Z", "description": "## Fixed",
                 "releaseAssets": {"nodes": [{"name": "tool.rpm", "downloadUrl": "https://x/tool.rpm", "size": 3},
                                             {"name": "tool.deb", "downloadUrl": "https://x/tool.deb", "size": 4}]}}
            ]}},
            "r1": null},
            "errors": [{"type": "NOT_FOUND", "path": ["r1"]}]}"###;
        let found = parse_batch(&repos, body);
        assert_eq!(found.len(), 1);
        let releases = &found[&repos[0]];
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "v1");
        assert!(releases[0].prerelease);
        assert_eq!(releases[0].notes.as_deref(), Some("## Fixed"));
        // Sorted by name, as REST lists them.
        assert_eq!(releases[0].assets[0].url, "https://x/tool.deb");
        assert_eq!(releases[0].assets[1].size, 3);
        assert!(parse_batch(&repos, "not json").is_empty());
        let infos = parse_repo_infos(&repos, body);
        assert_eq!(infos.len(), 1);
        assert_eq!(
            infos[0].1,
            super::super::RepoInfo {
                archived: true,
                name: "new/tool".into()
            }
        );
    }

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
