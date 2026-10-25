//! A direct link to a file, such as `https://example.org/tool/latest.AppImage`.
//! It has one release: the file it leads to now. Its version is the one in
//! the name of the file the link redirects to or the folder it is in
//! (`/prod/6.2.0/tool.rpm`), else the file's date, else its ETag, so that a
//! changed file shows as an update.

use ureq::ResponseExt;

use crate::error::Result;
use crate::http;
use crate::model::{Asset, Release};

pub fn releases(url: &str) -> Result<Vec<Release>> {
    let mut response = http::head(url).call()?;
    // Some servers refuse HEAD; a GET whose body is never read is as good.
    if matches!(response.status().as_u16(), 403 | 405 | 501) {
        response = http::get(url).call()?;
    }
    let response = http::check(response)?;
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let final_url = response.get_uri().to_string();
    Ok(vec![release(
        url,
        &final_url,
        header("last-modified").as_deref(),
        header("etag").as_deref(),
        header("content-length").and_then(|l| l.parse().ok()),
    )])
}

/// The release a link at `url`, ending up at `final_url`, stands for.
pub(super) fn release(
    url: &str,
    final_url: &str,
    last_modified: Option<&str>,
    etag: Option<&str>,
    size: Option<u64>,
) -> Release {
    let name = file_name(final_url);
    let tag = super::version_in(&name)
        .or_else(|| folder_version(final_url))
        .or_else(|| {
            last_modified
                .and_then(super::parse_http_date)
                .map(|(y, mo, d, h, mi, _)| format!("build-{y:04}{mo:02}{d:02}{h:02}{mi:02}"))
        })
        .or_else(|| {
            etag.map(|e| {
                let clean: String = e.chars().filter(char::is_ascii_alphanumeric).take(16).collect();
                format!("etag-{clean}")
            })
        })
        .unwrap_or_else(|| "unknown".into());
    Release {
        prerelease: false,
        name: None,
        published_at: last_modified.and_then(super::iso_date),
        notes: None,
        tag,
        // The link itself, not where it leads now: that may be a signed
        // address that expires.
        assets: vec![Asset {
            name,
            url: url.to_string(),
            size: size.unwrap_or(0),
            digest: None,
        }],
    }
}

/// A version naming a folder of `url`'s path, the innermost first.
fn folder_version(url: &str) -> Option<String> {
    let path = url::Url::parse(url).ok()?.path().to_string();
    let mut folders: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    folders.pop();
    folders.iter().rev().find_map(|f| super::version_in(f))
}

/// The last path segment of `url`, without its query.
pub(super) fn file_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .replace("%20", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_release_after_the_file_it_leads_to() {
        let r = release(
            "https://example.org/latest/download",
            "https://cdn.example.org/tool/1.4.2/tool-1.4.2-x86_64.AppImage?sig=abc",
            Some("Mon, 13 May 2024 12:28:15 GMT"),
            None,
            Some(10),
        );
        assert_eq!(r.tag, "1.4.2");
        assert_eq!(r.assets[0].name, "tool-1.4.2-x86_64.AppImage");
        assert_eq!(r.assets[0].url, "https://example.org/latest/download");
        assert_eq!(r.assets[0].size, 10);
        assert_eq!(r.published_at.as_deref(), Some("2024-05-13T12:28:15Z"));
    }

    #[test]
    fn takes_the_version_from_the_folder() {
        let r = release(
            "https://zoom.us/client/latest/zoom_x86_64.rpm",
            "https://cdn.zoom.us/prod/6.2.11.5069/zoom_x86_64.rpm",
            Some("Mon, 13 May 2024 12:28:15 GMT"),
            None,
            None,
        );
        assert_eq!(r.tag, "6.2.11.5069");
    }

    #[test]
    fn falls_back_to_the_date_then_the_etag() {
        let dated = release(
            "https://example.org/tool.AppImage",
            "https://example.org/tool.AppImage",
            Some("Mon, 13 May 2024 12:28:15 GMT"),
            Some("\"abc\""),
            None,
        );
        assert_eq!(dated.tag, "build-202405131228");
        assert!(crate::version::is_newer(&dated.tag, "build-202405131229"));
        let tagged = release(
            "https://e.org/t.rpm",
            "https://e.org/t.rpm",
            None,
            Some("W/\"5f-a1b2\""),
            None,
        );
        assert_eq!(tagged.tag, "etag-W5fa1b2");
        assert_eq!(
            release("https://e.org/t.rpm", "https://e.org/t.rpm", None, None, None).tag,
            "unknown"
        );
    }
}
