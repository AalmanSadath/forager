use crate::error::Result;
use crate::http;
use crate::model::Release;

/// Fetches recent releases from a Forgejo or Gitea instance (Codeberg runs
/// Forgejo), newest first. Their API has the same shape as GitHub's. A token
/// comes from `FORAGER_TOKEN_<HOST>`, or `CODEBERG_TOKEN` for codeberg.org.
pub fn releases(host: &str, owner: &str, repo: &str) -> Result<Vec<Release>> {
    let url = format!("https://{host}/api/v1/repos/{owner}/{repo}/releases?limit=20");
    let mut request = http::get(&url).header("Accept", "application/json");
    if let Some(token) = http::token_for(host, Some(("codeberg.org", "CODEBERG_TOKEN"))) {
        request = request.header("Authorization", format!("token {token}"));
    }
    let body = http::get_cached(request, &format!("forgejo/{host}__{owner}__{repo}.json"))?;
    super::github::parse_releases(&body)
}

#[cfg(test)]
mod tests {
    use super::super::github::parse_releases;

    #[test]
    fn parses_forgejo_releases() {
        let body = r#"[{"id": 1, "tag_name": "v3.0", "name": "3.0", "draft": false, "prerelease": true,
            "published_at": "2026-01-01T00:00:00Z",
            "assets": [{"id": 9, "name": "tool-x86_64.AppImage", "size": 1024, "download_count": 5,
                        "browser_download_url": "https://codeberg.org/o/r/releases/download/v3.0/tool-x86_64.AppImage"}]}]"#;
        let releases = parse_releases(body).unwrap();
        assert!(releases[0].prerelease);
        assert_eq!(releases[0].assets[0].size, 1024);
        assert_eq!(releases[0].assets[0].digest, None);
    }
}
