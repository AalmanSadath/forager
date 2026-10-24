mod forgejo;
mod github;
mod gitlab;
mod html;

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::Release;

/// Where an app's releases come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    GitHub {
        owner: String,
        repo: String,
    },
    /// gitlab.com or a self-hosted GitLab. `path` may include subgroups.
    GitLab {
        host: String,
        path: String,
    },
    /// Codeberg or another Forgejo or Gitea instance.
    Forgejo {
        host: String,
        owner: String,
        repo: String,
    },
    /// A web page linking to downloads; links whose address matches
    /// `pattern` are the assets.
    Html {
        url: String,
        pattern: String,
    },
}

/// Kind of forge, for hosts Forager does not recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
    Forgejo,
    /// A plain web page, which needs a pattern: see `Source::html`.
    Html,
}

impl std::str::FromStr for Forge {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "github" => Ok(Forge::GitHub),
            "gitlab" => Ok(Forge::GitLab),
            "forgejo" | "gitea" => Ok(Forge::Forgejo),
            "html" | "page" | "web" => Ok(Forge::Html),
            _ => Err(format!("unknown forge '{s}' (expected gitlab, forgejo or html)")),
        }
    }
}

const GITLAB_HOSTS: [&str; 6] = [
    "gitlab.com",
    "gitlab.gnome.org",
    "gitlab.freedesktop.org",
    "invent.kde.org",
    "salsa.debian.org",
    "framagit.org",
];
const FORGEJO_HOSTS: [&str; 1] = ["codeberg.org"];

impl Source {
    /// Parses a project URL such as `https://github.com/owner/repo`. `forge`
    /// says which API a host speaks; it is needed only for hosts other than
    /// github.com, codeberg.org and a few well-known GitLab instances.
    pub fn from_url(url: &str, forge: Option<Forge>) -> Result<Source> {
        let unsupported = || Error::UnsupportedSource(url.to_string());
        let trimmed = url
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.")
            .trim_end_matches('/')
            .trim_end_matches(".git");
        let (host, path) = trimmed.split_once('/').ok_or_else(unsupported)?;
        let host = host.to_ascii_lowercase();
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        let forge = match forge {
            Some(forge) => forge,
            None if host == "github.com" => Forge::GitHub,
            None if GITLAB_HOSTS.contains(&host.as_str()) => Forge::GitLab,
            None if FORGEJO_HOSTS.contains(&host.as_str()) => Forge::Forgejo,
            None => return Err(Error::UnknownForge(host)),
        };
        match forge {
            Forge::GitHub if host == "github.com" && segments.len() >= 2 => Ok(Source::GitHub {
                owner: segments[0].to_string(),
                repo: segments[1].to_string(),
            }),
            Forge::GitHub => Err(unsupported()),
            Forge::GitLab => {
                // Everything before `/-/` (e.g. `/-/releases`) is the project path.
                let end = segments.iter().position(|s| *s == "-").unwrap_or(segments.len());
                if end < 2 {
                    return Err(unsupported());
                }
                Ok(Source::GitLab {
                    host,
                    path: segments[..end].join("/"),
                })
            }
            Forge::Forgejo if segments.len() >= 2 => Ok(Source::Forgejo {
                host,
                owner: segments[0].to_string(),
                repo: segments[1].to_string(),
            }),
            Forge::Forgejo => Err(unsupported()),
            Forge::Html => Err(Error::PatternNeeded),
        }
    }

    /// A web page source: `url` lists downloads, and links matching
    /// `pattern` (a regex over the full address, optionally with a
    /// `(?P<version>…)` group) are the app's assets.
    pub fn html(url: &str, pattern: &str) -> Result<Source> {
        let url = url.trim();
        html::validate(url, pattern)?;
        Ok(Source::Html {
            url: url.to_string(),
            pattern: pattern.to_string(),
        })
    }

    /// The project's page in a browser.
    pub fn web_url(&self) -> String {
        match self {
            Source::Html { url, .. } => url.clone(),
            _ => format!("https://{self}"),
        }
    }

    /// Short name of the forge kind: `github`, `gitlab` or `forgejo`.
    pub fn forge(&self) -> &'static str {
        match self {
            Source::GitHub { .. } => "github",
            Source::GitLab { .. } => "gitlab",
            Source::Forgejo { .. } => "forgejo",
            Source::Html { .. } => "html",
        }
    }

    /// Suggested app id derived from the source.
    pub fn default_id(&self) -> String {
        match self {
            Source::GitHub { repo, .. } | Source::Forgejo { repo, .. } => repo.to_ascii_lowercase(),
            Source::GitLab { path, .. } => path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase(),
            Source::Html { url, .. } => html_default_id(url),
        }
    }

    /// Newest release, skipping prereleases unless `prerelease` is set.
    pub fn latest_release(&self, prerelease: bool) -> Result<Release> {
        let releases = match self {
            Source::GitHub { owner, repo } => github::releases(owner, repo),
            Source::GitLab { host, path } => gitlab::releases(host, path),
            Source::Forgejo { host, owner, repo } => forgejo::releases(host, owner, repo),
            Source::Html { url, pattern } => html::releases(url, pattern),
        }
        .map_err(|e| match e {
            Error::Http(ureq::Error::StatusCode(404)) => Error::ProjectNotFound(self.to_string()),
            e => e,
        })?;
        releases
            .into_iter()
            .find(|r| prerelease || !r.prerelease)
            .ok_or(Error::NoRelease)
    }
}

/// The last meaningful part of a page address, as an app id: `node` for
/// `https://nodejs.org/dist/latest/` would be `latest`, so the host's first
/// label is used when the path says nothing better.
fn html_default_id(url: &str) -> String {
    let parsed = url::Url::parse(url).ok();
    let segment = parsed
        .as_ref()
        .and_then(|u| u.path_segments())
        .and_then(|mut s| s.rfind(|s| !s.is_empty()))
        .map(|s| s.split('.').next().unwrap_or(s).to_string())
        .filter(|s| !["latest", "download", "downloads", "releases", "dist", "index"].contains(&s.as_str()));
    let host = parsed
        .as_ref()
        .and_then(|u| u.host_str())
        .map(|h| h.trim_start_matches("www.").split('.').next().unwrap_or(h).to_string());
    let id: String = segment
        .or(host)
        .unwrap_or_else(|| "app".into())
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    id
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::GitHub { owner, repo } => write!(f, "github.com/{owner}/{repo}"),
            Source::GitLab { host, path } => write!(f, "{host}/{path}"),
            Source::Forgejo { host, owner, repo } => write!(f, "{host}/{owner}/{repo}"),
            Source::Html { url, .. } => {
                let bare = url.trim_start_matches("https://").trim_start_matches("http://");
                write!(f, "{}", bare.trim_end_matches('/'))
            }
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
            assert_eq!(Source::from_url(url, None).unwrap(), expected, "{url}");
        }
        assert!(Source::from_url("https://github.com/owner", None).is_err());
        assert!(Source::from_url("https://github.com", None).is_err());
    }

    #[test]
    fn parses_gitlab_urls() {
        let source = Source::from_url("https://gitlab.com/group/sub/tool/-/releases", None).unwrap();
        assert_eq!(
            source,
            Source::GitLab {
                host: "gitlab.com".into(),
                path: "group/sub/tool".into()
            }
        );
        assert_eq!(source.default_id(), "tool");
        assert_eq!(source.to_string(), "gitlab.com/group/sub/tool");
        assert!(Source::from_url("https://gitlab.com/group", None).is_err());
    }

    #[test]
    fn parses_forgejo_urls() {
        let expected = Source::Forgejo {
            host: "codeberg.org".into(),
            owner: "owner".into(),
            repo: "repo".into(),
        };
        assert_eq!(
            Source::from_url("https://codeberg.org/owner/repo/releases", None).unwrap(),
            expected
        );
    }

    #[test]
    fn makes_web_page_sources() {
        let source = Source::html("https://nodejs.org/dist/latest/", r"linux-x64\.tar\.xz$").unwrap();
        assert_eq!(source.forge(), "html");
        assert_eq!(source.default_id(), "nodejs");
        assert_eq!(source.to_string(), "nodejs.org/dist/latest");
        assert_eq!(source.web_url(), "https://nodejs.org/dist/latest/");
        assert_eq!(
            Source::html("https://example.org/tools/Krita.html", "x")
                .unwrap()
                .default_id(),
            "krita"
        );
        assert!(matches!(
            Source::from_url("https://example.org/dl", Some(Forge::Html)),
            Err(Error::PatternNeeded)
        ));
    }

    #[test]
    fn unknown_hosts_need_a_forge() {
        let url = "https://git.example.org/owner/repo";
        assert!(matches!(Source::from_url(url, None), Err(Error::UnknownForge(_))));
        assert!(matches!(
            Source::from_url(url, Some(Forge::Forgejo)).unwrap(),
            Source::Forgejo { .. }
        ));
        assert!(matches!(
            Source::from_url(url, Some(Forge::GitLab)).unwrap(),
            Source::GitLab { .. }
        ));
    }
}
