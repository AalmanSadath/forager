//! HTTP agent shared by sources and downloads.

use std::sync::LazyLock;
use std::time::Duration;

use ureq::http::Response;
use ureq::typestate::WithoutBody;
use ureq::{Agent, Body, RequestBuilder};

use crate::error::{Error, Result};

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
