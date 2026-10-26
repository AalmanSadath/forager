//! What a forge says about a repository itself: whether it is archived and
//! its current name, which differs from the one Forager tracks when the
//! project moved. The forges answer requests for the old name, so a moved
//! project keeps working, but the address goes stale.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{Source, gitlab};
use crate::http;
use crate::store;

/// How long an answer is kept. Archiving and moving are rare, and a check
/// without a token should not cost a second request per app every time.
const FRESH: Duration = Duration::from_secs(3 * 24 * 3600);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    pub archived: bool,
    /// The full name the forge gives the repository now: `owner/repo`, or
    /// GitLab's path with its groups.
    pub name: String,
}

impl Source {
    /// The repository's state, from the cache when fresh, else from the
    /// forge. `None` for sources without repositories, and on errors: this
    /// is advice, and a failed request is retried at the next check.
    pub fn repo_info(&self) -> Option<RepoInfo> {
        let path = cache_path(self)?;
        if let Some(info) = load(&path) {
            return Some(info);
        }
        let info = fetch(self)?;
        save(self, &info);
        Some(info)
    }

    /// Where the repository went, when `info` names another one than this
    /// source. Names are compared without case, as the forges do.
    pub fn moved_to(&self, info: &RepoInfo) -> Option<Source> {
        let current = match self {
            Source::GitHub { owner, repo } | Source::Forgejo { owner, repo, .. } => format!("{owner}/{repo}"),
            Source::GitLab { path, .. } => path.clone(),
            _ => return None,
        };
        if info.name.is_empty() || current.eq_ignore_ascii_case(&info.name) {
            return None;
        }
        Some(match self {
            Source::GitHub { .. } => {
                let (owner, repo) = info.name.split_once('/')?;
                Source::GitHub {
                    owner: owner.into(),
                    repo: repo.into(),
                }
            }
            Source::Forgejo { host, .. } => {
                let (owner, repo) = info.name.split_once('/')?;
                Source::Forgejo {
                    host: host.clone(),
                    owner: owner.into(),
                    repo: repo.into(),
                }
            }
            Source::GitLab { host, .. } => Source::GitLab {
                host: host.clone(),
                path: info.name.clone(),
            },
            _ => return None,
        })
    }
}

/// Keeps `info` for `source`, e.g. from an answer about many repositories.
pub(super) fn save(source: &Source, info: &RepoInfo) {
    if let Some(path) = cache_path(source)
        && let Ok(text) = serde_json::to_string(info)
    {
        // Best effort: without it the next check asks again.
        fs::create_dir_all(path.parent().unwrap_or(&path)).ok();
        fs::write(path, text).ok();
    }
}

fn cache_path(source: &Source) -> Option<PathBuf> {
    let name = match source {
        Source::GitHub { owner, repo } => format!("github__{owner}__{repo}"),
        Source::GitLab { host, path } => format!("gitlab__{host}__{}", path.replace('/', "__")),
        Source::Forgejo { host, owner, repo } => format!("forgejo__{host}__{owner}__{repo}"),
        _ => return None,
    };
    Some(store::cache_dir().join("repo").join(format!("{name}.json")))
}

fn load(path: &Path) -> Option<RepoInfo> {
    let age = fs::metadata(path).ok()?.modified().ok()?.elapsed().ok()?;
    if age > FRESH {
        return None;
    }
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn fetch(source: &Source) -> Option<RepoInfo> {
    let body = match source {
        Source::GitHub { owner, repo } => {
            let mut request = http::get(&format!("https://api.github.com/repos/{owner}/{repo}"))
                .header("Accept", "application/vnd.github+json");
            if let Some(token) = http::token_for("github.com", Some(("github.com", "GITHUB_TOKEN"))) {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
            read(request)?
        }
        Source::Forgejo { host, owner, repo } => {
            let mut request = http::get(&format!("https://{host}/api/v1/repos/{owner}/{repo}"));
            if let Some(token) = http::token_for(host, Some(("codeberg.org", "CODEBERG_TOKEN"))) {
                request = request.header("Authorization", format!("token {token}"));
            }
            read(request)?
        }
        Source::GitLab { host, path } => {
            let mut request = http::get(&format!("https://{host}/api/v4/projects/{}", gitlab::encode(path)));
            if let Some(token) = http::token_for(host, Some(("gitlab.com", "GITLAB_TOKEN"))) {
                request = request.header("PRIVATE-TOKEN", token);
            }
            read(request)?
        }
        _ => return None,
    };
    parse(source, &body)
}

fn read(request: ureq::RequestBuilder<ureq::typestate::WithoutBody>) -> Option<String> {
    http::check(request.call().ok()?).ok()?.body_mut().read_to_string().ok()
}

/// Reads a forge's answer about a repository: `archived` and `full_name`
/// from GitHub and Forgejo, `archived` and `path_with_namespace` from GitLab.
fn parse(source: &Source, body: &str) -> Option<RepoInfo> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let name = match source {
        Source::GitLab { .. } => value["path_with_namespace"].as_str()?,
        _ => value["full_name"].as_str()?,
    };
    Some(RepoInfo {
        archived: value["archived"].as_bool().unwrap_or(false),
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    fn github(owner: &str, repo: &str) -> Source {
        Source::GitHub {
            owner: owner.into(),
            repo: repo.into(),
        }
    }

    #[test]
    fn reads_each_forges_answer() {
        let info = parse(&github("o", "r"), r#"{"full_name": "New/r", "archived": true}"#).unwrap();
        assert_eq!(
            info,
            RepoInfo {
                archived: true,
                name: "New/r".into()
            }
        );
        let gitlab = Source::GitLab {
            host: "gitlab.com".into(),
            path: "g/sub/p".into(),
        };
        let info = parse(&gitlab, r#"{"path_with_namespace": "g/sub/p", "archived": false}"#).unwrap();
        assert_eq!(info.name, "g/sub/p");
        assert!(!info.archived);
        assert!(parse(&gitlab, "{}").is_none());
    }

    #[test]
    fn a_new_name_is_a_move() {
        let source = github("old-owner", "Tool");
        let same = RepoInfo {
            archived: false,
            name: "Old-Owner/tool".into(),
        };
        assert_eq!(source.moved_to(&same), None);
        let moved = RepoInfo {
            archived: false,
            name: "new-org/tool".into(),
        };
        assert_eq!(source.moved_to(&moved), Some(github("new-org", "tool")));

        let gitlab = Source::GitLab {
            host: "gitlab.example.org".into(),
            path: "a/b".into(),
        };
        let moved = RepoInfo {
            archived: false,
            name: "c/d/b".into(),
        };
        assert_eq!(
            gitlab.moved_to(&moved),
            Some(Source::GitLab {
                host: "gitlab.example.org".into(),
                path: "c/d/b".into()
            })
        );
        let page = Source::Direct {
            url: "https://x".into(),
        };
        assert_eq!(page.moved_to(&moved), None);
    }

    #[test]
    fn keeps_answers_for_a_while() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo.json");
        fs::write(&path, r#"{"archived": true, "name": "o/r"}"#).unwrap();
        assert_eq!(load(&path).map(|i| i.archived), Some(true));
        let old = SystemTime::now() - FRESH - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert_eq!(load(&path), None);
    }
}
