mod direct;
mod forgejo;
mod github;
mod gitlab;
mod html;
mod jenkins;
mod sourceforge;
mod sourcehut;

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{Format, Release};
use crate::version;

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
    /// A SourceForge project's files.
    SourceForge {
        project: String,
    },
    /// A git.sr.ht repository; `owner` without its `~`.
    SourceHut {
        owner: String,
        repo: String,
    },
    /// A Jenkins job, its address ending in `/`.
    Jenkins {
        url: String,
    },
    /// A link straight to the file.
    Direct {
        url: String,
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
    SourceForge,
    SourceHut,
    Jenkins,
    /// A link straight to the file.
    Direct,
}

impl std::str::FromStr for Forge {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "github" => Ok(Forge::GitHub),
            "gitlab" => Ok(Forge::GitLab),
            "forgejo" | "gitea" => Ok(Forge::Forgejo),
            "html" | "page" | "web" => Ok(Forge::Html),
            "sourceforge" => Ok(Forge::SourceForge),
            "sourcehut" | "srht" => Ok(Forge::SourceHut),
            "jenkins" => Ok(Forge::Jenkins),
            "direct" | "file" | "link" => Ok(Forge::Direct),
            _ => Err(format!(
                "unknown forge '{s}' (expected gitlab, forgejo, html, sourceforge, sourcehut, jenkins or direct)"
            )),
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
        // Jenkins and direct links keep the address as given, scheme and all.
        let full = || {
            let with_scheme = match url.trim().contains("://") {
                true => url.trim().to_string(),
                false => format!("https://{}", url.trim()),
            };
            url::Url::parse(&with_scheme)
                .ok()
                .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
                .ok_or_else(unsupported)
        };
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
            None if host == "sourceforge.net" => Forge::SourceForge,
            None if host == "git.sr.ht" => Forge::SourceHut,
            None if segments.contains(&"job") => Forge::Jenkins,
            None if segments.last().is_some_and(|s| Format::from_file_name(s).is_some()) => Forge::Direct,
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
            Forge::SourceForge => match segments.as_slice() {
                ["projects" | "p", project, ..] if host == "sourceforge.net" => Ok(Source::SourceForge {
                    project: project.to_string(),
                }),
                _ => Err(unsupported()),
            },
            Forge::SourceHut => match segments.as_slice() {
                [owner, repo, ..] if host == "git.sr.ht" && owner.starts_with('~') => Ok(Source::SourceHut {
                    owner: owner.trim_start_matches('~').to_string(),
                    repo: repo.to_string(),
                }),
                _ => Err(unsupported()),
            },
            Forge::Jenkins => {
                let mut job = full()?;
                let path: Vec<String> = job
                    .path_segments()
                    .map(|s| s.filter(|s| !s.is_empty()).map(str::to_string).collect())
                    .unwrap_or_default();
                // Up to the last `job/<name>`: a build or page inside the job is the job.
                let end = path
                    .iter()
                    .rposition(|s| s == "job")
                    .filter(|i| i + 1 < path.len())
                    .ok_or_else(unsupported)?;
                job.set_path(&format!("/{}/", path[..end + 2].join("/")));
                job.set_query(None);
                job.set_fragment(None);
                Ok(Source::Jenkins { url: job.to_string() })
            }
            Forge::Direct => {
                let mut link = full()?;
                link.set_fragment(None);
                Ok(Source::Direct { url: link.to_string() })
            }
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
            Source::Html { url, .. } | Source::Jenkins { url } | Source::Direct { url } => url.clone(),
            Source::SourceHut { owner, repo } => format!("https://git.sr.ht/~{owner}/{repo}"),
            _ => format!("https://{self}"),
        }
    }

    /// The page of the release tagged `tag`, for apps that are tracked only.
    pub fn release_url(&self, tag: &str) -> String {
        let tag = url::form_urlencoded::byte_serialize(tag.as_bytes()).collect::<String>();
        match self {
            Source::GitHub { .. } | Source::Forgejo { .. } => format!("{}/releases/tag/{tag}", self.web_url()),
            Source::GitLab { .. } => format!("{}/-/releases/{tag}", self.web_url()),
            Source::SourceForge { .. } => format!("{}/files/", self.web_url()),
            Source::SourceHut { .. } => format!("{}/refs/{tag}", self.web_url()),
            Source::Jenkins { url } => match tag.strip_prefix("build-") {
                Some(number) => format!("{url}{number}/"),
                None => url.clone(),
            },
            Source::Html { url, .. } | Source::Direct { url } => url.clone(),
        }
    }

    /// Short name of the forge kind: `github`, `gitlab` or `forgejo`.
    pub fn forge(&self) -> &'static str {
        match self {
            Source::GitHub { .. } => "github",
            Source::GitLab { .. } => "gitlab",
            Source::Forgejo { .. } => "forgejo",
            Source::Html { .. } => "html",
            Source::SourceForge { .. } => "sourceforge",
            Source::SourceHut { .. } => "sourcehut",
            Source::Jenkins { .. } => "jenkins",
            Source::Direct { .. } => "direct",
        }
    }

    /// Suggested app id derived from the source.
    pub fn default_id(&self) -> String {
        match self {
            Source::GitHub { repo, .. } | Source::Forgejo { repo, .. } => repo.to_ascii_lowercase(),
            Source::GitLab { path, .. } => path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase(),
            Source::Html { url, .. } => html_default_id(url),
            Source::SourceForge { project } => project.to_ascii_lowercase(),
            Source::SourceHut { repo, .. } => repo.to_ascii_lowercase(),
            Source::Jenkins { url } => {
                let name = url.trim_end_matches('/').rsplit('/').next().unwrap_or("app");
                sanitize_id(
                    &url::form_urlencoded::parse(name.as_bytes())
                        .map(|(k, _)| k)
                        .collect::<String>(),
                )
            }
            Source::Direct { url } => direct_default_id(url),
        }
    }

    /// Recent releases of many GitHub repositories at once, by `(owner,
    /// repo)`; empty without a GitHub token. See `github::releases_batch`.
    pub fn github_batch(repos: &[(String, String)]) -> std::collections::HashMap<(String, String), Vec<Release>> {
        github::releases_batch(repos)
    }

    /// Recent releases, newest first: the latest 20 from a forge.
    pub fn releases(&self) -> Result<Vec<Release>> {
        self.releases_deep(false)
    }

    /// Recent releases, newest first; `deep` asks a forge for as many as one
    /// request returns (100, or Forgejo's 50), for apps that keep to older
    /// versions. Other sources list what they have either way.
    pub fn releases_deep(&self, deep: bool) -> Result<Vec<Release>> {
        match self {
            Source::GitHub { owner, repo } => github::releases(owner, repo, deep),
            Source::GitLab { host, path } => gitlab::releases(host, path, deep),
            Source::Forgejo { host, owner, repo } => forgejo::releases(host, owner, repo, deep),
            Source::Html { url, pattern } => html::releases(url, pattern),
            Source::SourceForge { project } => sourceforge::releases(project),
            Source::SourceHut { owner, repo } => sourcehut::releases(owner, repo),
            Source::Jenkins { url } => jenkins::releases(url),
            Source::Direct { url } => direct::releases(url),
        }
        .map_err(|e| match e {
            Error::Http(ureq::Error::StatusCode(404)) => Error::ProjectNotFound(self.to_string()),
            e => e,
        })
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
    sanitize_id(&segment.or(host).unwrap_or_else(|| "app".into()))
}

/// Lower case, with anything an id may not hold turned into `-`.
fn sanitize_id(name: &str) -> String {
    let id: String = name
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
    match id.trim_start_matches('.') {
        "" => "app".into(),
        id => id.to_string(),
    }
}

/// Where the name of a file ends and its version or platform begins.
static NAME_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[-_. ](v?\d|x86|x64|amd64|aarch64|arm64|i[36]86|linux|setup|latest)").unwrap());

/// The app a direct link's file is: `zoom` for `zoom_x86_64.rpm`.
fn direct_default_id(url: &str) -> String {
    let name = direct::file_name(url);
    let name = NAME_END.find(&name).map_or(name.as_str(), |m| &name[..m.start()]);
    let name = name.split('.').next().unwrap_or(name);
    sanitize_id(if name.is_empty() { "app" } else { name })
}

/// A version in a file name: `1.2`, `1.2.3`, `2024.10.1`.
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)+").unwrap());

/// The first dotted number in `name`, as a version.
pub(crate) fn version_in(name: &str) -> Option<String> {
    NUMBER.find(name).map(|m| m.as_str().to_string())
}

/// Whether a tag found in a feed or file name reads like a prerelease.
fn looks_prerelease(tag: &str) -> bool {
    let lower = tag.to_ascii_lowercase();
    ["alpha", "beta", "rc", "pre", "dev"].iter().any(|p| lower.contains(p))
}

/// Newest first, by version where the tags parse as one.
fn sort_newest_first(releases: &mut [Release]) {
    releases.sort_by(|a, b| match (version::parse_tag(&a.tag), version::parse_tag(&b.tag)) {
        (Some(x), Some(y)) => y.cmp(&x),
        _ => b.tag.cmp(&a.tag),
    });
}

/// The text of the first `<name>` element in a piece of RSS, CDATA
/// unwrapped and the common entities decoded.
fn element(xml: &str, name: &str) -> Option<String> {
    let start = xml.find(&format!("<{name}>"))? + name.len() + 2;
    let end = start + xml[start..].find(&format!("</{name}>"))?;
    let text = xml[start..end].trim();
    let text = text
        .strip_prefix("<![CDATA[")
        .and_then(|t| t.strip_suffix("]]>"))
        .map(str::to_string)
        .unwrap_or_else(|| {
            text.replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&#39;", "'")
                .replace("&amp;", "&")
        });
    Some(text)
}

/// Date and time (UTC) of an RFC 2822 date as feeds and HTTP headers write
/// them, e.g. `Mon, 13 May 2024 12:28:15 GMT` or `... +0200`.
fn parse_http_date(text: &str) -> Option<(i64, u32, u32, u32, u32, u32)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let words = match words.first() {
        Some(w) if w.ends_with(',') => &words[1..],
        _ => &words[..],
    };
    let [day, month, year, time, zone @ ..] = words else {
        return None;
    };
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let month = MONTHS.iter().position(|m| month.to_ascii_lowercase().starts_with(m))? as u32 + 1;
    let mut clock = time.split(':').map(|p| p.parse::<u32>().ok());
    let (h, mi, s) = (clock.next()??, clock.next()??, clock.next().flatten().unwrap_or(0));
    let offset = match zone.first() {
        Some(z) if z.starts_with(['+', '-']) && z.len() == 5 => {
            let minutes = z[1..3].parse::<i64>().ok()? * 60 + z[3..5].parse::<i64>().ok()?;
            if z.starts_with('-') { -minutes } else { minutes }
        }
        _ => 0,
    };
    let days = days_from_civil(year.parse().ok()?, month, day.parse().ok()?);
    let secs = days * 86400 + i64::from(h * 3600 + mi * 60 + s) - offset * 60;
    Some(civil_from_unix(secs))
}

/// An RFC 2822 date as ISO 8601 in UTC, as the forges' APIs give dates.
fn iso_date(text: &str) -> Option<String> {
    let (y, mo, d, h, mi, s) = parse_http_date(text)?;
    Some(format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"))
}

/// ISO 8601 as the forges write it (`2026-10-25T08:50:19Z`, with optional
/// fractions of a second or a `+01:00` offset) as seconds since the Unix
/// epoch.
pub fn unix_from_iso(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, time) = text.split_once(['T', ' ']).unwrap_or((text, "00:00:00"));
    let mut parts = date.split('-').map(|p| p.parse::<i64>().ok());
    let (year, month, day) = (parts.next()??, parts.next()??, parts.next()??);
    let zone_at = time.find(['Z', 'z', '+', '-']).unwrap_or(time.len());
    let (clock, zone) = time.split_at(zone_at);
    let mut clock = clock.split('.').next()?.split(':').map(|p| p.parse::<i64>().ok());
    let (h, mi, s) = (clock.next()??, clock.next()??, clock.next().flatten().unwrap_or(0));
    let offset = match zone.strip_prefix(['+', '-']) {
        Some(rest) => {
            let digits: String = rest.chars().filter(char::is_ascii_digit).collect();
            let minutes =
                digits.get(..2)?.parse::<i64>().ok()? * 60 + digits.get(2..4).map_or(Some(0), |m| m.parse().ok())?;
            if zone.starts_with('-') { -minutes } else { minutes }
        }
        None => 0,
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let secs = days_from_civil(year, month as u32, day as u32) * 86400 + h * 3600 + mi * 60 + s - offset * 60;
    u64::try_from(secs).ok()
}

/// Seconds since the Unix epoch as ISO 8601 in UTC.
pub fn iso_from_unix(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = civil_from_unix(secs as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Howard Hinnant's days_from_civil.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86400);
    let rest = secs.rem_euclid(86400) as u32;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rest / 3600, rest % 3600 / 60, rest % 60)
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::GitHub { owner, repo } => write!(f, "github.com/{owner}/{repo}"),
            Source::GitLab { host, path } => write!(f, "{host}/{path}"),
            Source::Forgejo { host, owner, repo } => write!(f, "{host}/{owner}/{repo}"),
            Source::SourceForge { project } => write!(f, "sourceforge.net/projects/{project}"),
            Source::SourceHut { owner, repo } => write!(f, "git.sr.ht/~{owner}/{repo}"),
            Source::Html { url, .. } | Source::Jenkins { url } | Source::Direct { url } => {
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
    fn links_release_pages() {
        let github = Source::from_url("https://github.com/o/r", None).unwrap();
        assert_eq!(github.release_url("v1.0"), "https://github.com/o/r/releases/tag/v1.0");
        let gitlab = Source::from_url("https://gitlab.com/g/r", None).unwrap();
        assert_eq!(gitlab.release_url("v1/x"), "https://gitlab.com/g/r/-/releases/v1%2Fx");
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
    fn recognises_sourceforge_sourcehut_jenkins_and_direct_links() {
        let sf = Source::from_url("https://sourceforge.net/projects/smplayer/files/SMPlayer/", None).unwrap();
        assert_eq!(
            sf,
            Source::SourceForge {
                project: "smplayer".into()
            }
        );
        assert_eq!(sf.to_string(), "sourceforge.net/projects/smplayer");
        assert_eq!(
            sf.release_url("23.5.0"),
            "https://sourceforge.net/projects/smplayer/files/"
        );

        let srht = Source::from_url("https://git.sr.ht/~emersion/kanshi/refs", None).unwrap();
        assert_eq!(
            srht,
            Source::SourceHut {
                owner: "emersion".into(),
                repo: "kanshi".into()
            }
        );
        assert_eq!(srht.web_url(), "https://git.sr.ht/~emersion/kanshi");
        assert_eq!(
            srht.release_url("v1.7.0"),
            "https://git.sr.ht/~emersion/kanshi/refs/v1.7.0"
        );
        assert_eq!(srht.default_id(), "kanshi");

        let jenkins = Source::from_url("http://ci.example.org/job/team/job/My%20Tool/42/console?x=1", None).unwrap();
        assert_eq!(
            jenkins,
            Source::Jenkins {
                url: "http://ci.example.org/job/team/job/My%20Tool/".into()
            }
        );
        assert_eq!(jenkins.default_id(), "my-tool");
        assert_eq!(
            jenkins.release_url("build-42"),
            "http://ci.example.org/job/team/job/My%20Tool/42/"
        );

        let direct = Source::from_url("https://zoom.us/client/latest/zoom_x86_64.rpm", None).unwrap();
        assert_eq!(
            direct,
            Source::Direct {
                url: "https://zoom.us/client/latest/zoom_x86_64.rpm".into()
            }
        );
        assert_eq!(direct.default_id(), "zoom");
        assert_eq!(direct.to_string(), "zoom.us/client/latest/zoom_x86_64.rpm");
        // A link without a file name needs saying so.
        let url = "https://discord.com/api/download?platform=linux&format=tar.gz";
        assert!(matches!(Source::from_url(url, None), Err(Error::UnknownForge(_))));
        assert_eq!(
            Source::from_url(url, Some(Forge::Direct)).unwrap().default_id(),
            "download"
        );
        assert_eq!(
            Source::from_url("https://example.org/Tool-2.1-linux.AppImage", None)
                .unwrap()
                .default_id(),
            "tool"
        );
    }

    #[test]
    fn reads_feed_dates() {
        assert_eq!(
            iso_date("Mon, 13 May 2024 12:28:15 UT").as_deref(),
            Some("2024-05-13T12:28:15Z")
        );
        assert_eq!(
            iso_date("Tue, 01 Jan 2030 01:30:00 +0200").as_deref(),
            Some("2029-12-31T23:30:00Z")
        );
        assert_eq!(iso_date("yesterday"), None);
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        for secs in [0, 1_792_918_219, 4_102_444_800] {
            assert_eq!(unix_from_iso(&iso_from_unix(secs)), Some(secs));
        }
        assert_eq!(unix_from_iso("2026-10-25T08:50:19.123Z"), Some(1_792_918_219));
        assert_eq!(unix_from_iso("2026-10-25T10:50:19+02:00"), Some(1_792_918_219));
        assert_eq!(unix_from_iso("2026-10-25"), Some(1_792_886_400));
        assert_eq!(unix_from_iso("soon"), None);
        assert_eq!(
            element("<a><title><![CDATA[/x & y]]></title></a>", "title").as_deref(),
            Some("/x & y")
        );
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
