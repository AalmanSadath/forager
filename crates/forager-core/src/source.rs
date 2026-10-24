mod github;

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::Release;

/// Where an app's releases come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    GitHub { owner: String, repo: String },
}

impl Source {
    /// Parses a project URL such as `https://github.com/owner/repo`.
    pub fn from_url(url: &str) -> Result<Source> {
        let trimmed = url
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.")
            .trim_end_matches('/')
            .trim_end_matches(".git");
        let mut parts = trimmed.split('/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("github.com"), Some(owner), Some(repo)) if !owner.is_empty() && !repo.is_empty() => {
                Ok(Source::GitHub {
                    owner: owner.to_string(),
                    repo: repo.to_string(),
                })
            }
            _ => Err(Error::UnsupportedSource(url.to_string())),
        }
    }

    /// Suggested app id derived from the source.
    pub fn default_id(&self) -> String {
        match self {
            Source::GitHub { repo, .. } => repo.to_ascii_lowercase(),
        }
    }

    /// Newest release, skipping prereleases unless `prerelease` is set.
    pub fn latest_release(&self, prerelease: bool) -> Result<Release> {
        let releases = match self {
            Source::GitHub { owner, repo } => github::releases(owner, repo)?,
        };
        releases
            .into_iter()
            .find(|r| prerelease || !r.prerelease)
            .ok_or(Error::NoRelease)
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::GitHub { owner, repo } => write!(f, "github.com/{owner}/{repo}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_urls() {
        let expected = Source::GitHub {
            owner: "owner".into(),
            repo: "repo".into(),
        };
        for url in [
            "https://github.com/owner/repo",
            "github.com/owner/repo/",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo/releases",
        ] {
            assert_eq!(Source::from_url(url).unwrap(), expected, "{url}");
        }
        assert!(Source::from_url("https://gitlab.com/owner/repo").is_err());
        assert!(Source::from_url("https://github.com/owner").is_err());
    }
}
