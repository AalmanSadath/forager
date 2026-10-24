//! HTTP agent shared by sources and downloads.

use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use ureq::http::Response;
use ureq::typestate::WithoutBody;
use ureq::{Agent, Body, RequestBuilder};

use crate::error::{Error, Result};
use crate::store;

static AGENT: LazyLock<Agent> = LazyLock::new(|| {
    Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        // Per read rather than overall, so large downloads only fail when stalled.
        .timeout_per_read(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .user_agent(concat!("forager/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
});

pub fn get(url: &str) -> RequestBuilder<WithoutBody> {
    AGENT.get(url)
}

/// Turns error statuses into errors. 304 passes through for conditional
/// requests; a GitHub rate-limit response becomes `Error::RateLimited`.
pub fn check(response: Response<Body>) -> Result<Response<Body>> {
    let status = response.status();
    if status.is_success() || status == 304 {
        return Ok(response);
    }
    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok());
    if matches!(status.as_u16(), 403 | 429) && header("x-ratelimit-remaining") == Some("0") {
        return Err(Error::RateLimited {
            reset: header("x-ratelimit-reset").and_then(|v| v.parse().ok()),
        });
    }
    Err(ureq::Error::StatusCode(status.as_u16()).into())
}

/// Release lists with many assets can pass ureq's default 10 MB limit.
const BODY_LIMIT: u64 = 64 * 1024 * 1024;

/// Sends `request` conditionally, using the response cached under
/// `$XDG_CACHE_HOME/forager/<cache_key>` with its ETag. An unchanged
/// resource comes back as an empty 304 and the cached body is returned.
pub fn get_cached(mut request: RequestBuilder<WithoutBody>, cache_key: &str) -> Result<String> {
    let cache_path = store::cache_dir().join(cache_key);
    let cached = load_cache(&cache_path);
    if let Some(cached) = &cached {
        request = request.header("If-None-Match", &cached.etag);
    }
    let mut response = check(request.call()?)?;

    if let Some(cached) = cached
        && response.status() == 304
    {
        return Ok(cached.body);
    }
    let etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = response.body_mut().with_config().limit(BODY_LIMIT).read_to_string()?;
    if let Some(etag) = etag {
        // Best effort: without the cache the next check just costs a full request.
        save_cache(
            &cache_path,
            &Cached {
                etag,
                body: body.clone(),
            },
        )
        .ok();
    }
    Ok(body)
}

/// Token for `host` from `FORAGER_TOKEN_<HOST>` (e.g. `FORAGER_TOKEN_GITLAB_EXAMPLE_ORG`),
/// or from `fallback` when `host` is the forge that variable belongs to. A
/// token is never sent to another host.
pub fn token_for(host: &str, fallback: Option<(&str, &str)>) -> Option<String> {
    let name: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    std::env::var(format!("FORAGER_TOKEN_{name}"))
        .ok()
        .or_else(|| match fallback {
            Some((fallback_host, var)) if host.eq_ignore_ascii_case(fallback_host) => std::env::var(var).ok(),
            _ => None,
        })
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
