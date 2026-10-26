//! Things worth knowing about an app's project that are not updates: the
//! repository was archived or moved, or nothing was released for a long
//! time. Checks report them; nothing acts on them without the user.

use std::fmt;

use serde::Serialize;

use crate::model::{App, Release};
use crate::source::{self, Source};

/// A project without a release for this long counts as possibly abandoned.
pub const STALE_DAYS: u64 = 365;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Notice {
    /// The repository is read-only: no more releases are coming.
    Archived,
    /// The repository has a new address. The old one still answers, but
    /// may stop to, or be taken by someone else.
    Moved {
        /// The new source, e.g. `github.com/new-org/tool`.
        to: String,
        /// Its web address, for `forager edit --url`.
        url: String,
    },
    /// The latest release is older than `STALE_DAYS`.
    Stale {
        /// The month of the latest release, `YYYY-MM`.
        since: String,
    },
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Notice::Archived => f.write_str("archived"),
            Notice::Moved { to, .. } => write!(f, "moved to {to}"),
            Notice::Stale { since } => write!(f, "no release since {since}"),
        }
    }
}

/// The notices for `app`, whose latest release is `latest`. Asks the forge
/// about the repository, at most every few days (see `Source::repo_info`).
pub fn for_app(app: &App, latest: &Release, now: u64) -> Vec<Notice> {
    let mut notices = Vec::new();
    if let Some(info) = app.source.repo_info() {
        if info.archived {
            notices.push(Notice::Archived);
        }
        if let Some(moved) = app.source.moved_to(&info) {
            notices.push(moved_notice(&moved));
        }
    }
    notices.extend(stale(app, latest, now));
    notices
}

fn moved_notice(to: &Source) -> Notice {
    Notice::Moved {
        to: to.to_string(),
        url: to.web_url(),
    }
}

/// `Stale` when `latest` came out more than `STALE_DAYS` before `now`. Not
/// for apps kept to older versions on purpose, whose latest eligible
/// release may well be old, or for releases without a date.
fn stale(app: &App, latest: &Release, now: u64) -> Option<Notice> {
    if app.versions.is_some() {
        return None;
    }
    let published = latest.published_at.as_deref()?;
    let age = now.checked_sub(source::unix_from_iso(published)?)?;
    if age <= STALE_DAYS * 86400 {
        return None;
    }
    Some(Notice::Stale {
        since: published.get(..7)?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(published_at: Option<&str>) -> Release {
        Release {
            tag: "v1".into(),
            name: None,
            prerelease: false,
            published_at: published_at.map(str::to_string),
            notes: None,
            assets: Vec::new(),
        }
    }

    #[test]
    fn old_releases_are_stale() {
        let app = App::new(
            Source::Direct {
                url: "https://x".into(),
            },
            "tool".into(),
        );
        let now = source::unix_from_iso("2026-10-27T00:00:00Z").unwrap();
        assert_eq!(
            stale(&app, &release(Some("2025-03-01T10:00:00Z")), now),
            Some(Notice::Stale {
                since: "2025-03".into()
            })
        );
        assert_eq!(stale(&app, &release(Some("2026-01-01T10:00:00Z")), now), None);
        assert_eq!(stale(&app, &release(None), now), None);
        let mut limited = app.clone();
        limited.versions = Some("<2".into());
        assert_eq!(stale(&limited, &release(Some("2020-01-01T00:00:00Z")), now), None);
    }

    #[test]
    fn describes_each_notice() {
        assert_eq!(Notice::Archived.to_string(), "archived");
        let moved = moved_notice(&Source::GitHub {
            owner: "new".into(),
            repo: "tool".into(),
        });
        assert_eq!(moved.to_string(), "moved to github.com/new/tool");
        assert_eq!(
            moved,
            Notice::Moved {
                to: "github.com/new/tool".into(),
                url: "https://github.com/new/tool".into()
            }
        );
    }
}
