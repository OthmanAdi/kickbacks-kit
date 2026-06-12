//! A small hardened HTTP client for read-only GETs of public JSON endpoints.
//!
//! Defenses, because this is the only outward-facing surface in kb:
//!   - bounded timeouts (connect and total), so a hung socket cannot wedge a
//!     background fetch thread;
//!   - HTTPS-only, with a low redirect cap, so a request cannot be bounced to a
//!     plaintext or unexpected host;
//!   - a hard body-size cap, so a hostile or broken server cannot exhaust
//!     memory;
//!   - conditional requests via `If-None-Match`, returning `NotModified`
//!     without re-downloading;
//!   - rate-limit headers surfaced to the caller so it can back off.
//!
//! Status codes are NOT turned into transport errors here: the caller inspects
//! the code so it can tell `304 Not Modified` and `403 rate limited` apart from
//! a real failure.

use std::time::Duration;

/// Maximum response body kb will read from any feed endpoint. The real payloads
/// are a few kilobytes (the bulletin) to tens of kilobytes (an issue page); a
/// half-megabyte ceiling is generous while still bounding memory.
const BODY_LIMIT: u64 = 512 * 1024;

/// Outcome of a conditional GET.
#[derive(Debug)]
pub enum Fetched {
    /// The server returned `304 Not Modified`; the cached copy is still current.
    NotModified,
    /// A fresh `2xx` body, with any caching and rate-limit metadata.
    Body(FetchedBody),
}

/// A fresh response body plus the headers kb acts on.
#[derive(Debug, Default)]
pub struct FetchedBody {
    pub text: String,
    pub etag: Option<String>,
    /// `x-ratelimit-remaining`, when the host sends it (GitHub does).
    pub rate_remaining: Option<u32>,
    /// `x-ratelimit-reset` converted to epoch millis, when present.
    pub rate_reset_ms: Option<i64>,
}

/// Why a fetch did not yield a usable body.
#[derive(Debug)]
pub enum FetchError {
    /// A non-2xx, non-304 status (e.g. 403, 404, 500).
    Status(u16),
    /// A connect/timeout/TLS/DNS failure.
    Transport(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Status(code) => write!(f, "HTTP {code}"),
            FetchError::Transport(msg) => write!(f, "{msg}"),
        }
    }
}

/// A reusable client wrapping a configured `ureq::Agent`.
pub struct FetchClient {
    agent: ureq::Agent,
    user_agent: String,
}

impl FetchClient {
    /// Build a client with conservative timeouts. `user_agent` is sent on every
    /// request (GitHub rejects requests without one).
    pub fn new(user_agent: impl Into<String>) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // We read status codes ourselves (304/403 are not failures here).
            .http_status_as_error(false)
            .https_only(true)
            .max_redirects(3)
            .timeout_global(Some(Duration::from_secs(12)))
            .timeout_connect(Some(Duration::from_secs(6)))
            .build()
            .into();
        FetchClient {
            agent,
            user_agent: user_agent.into(),
        }
    }

    /// Conditional GET of a JSON endpoint. Sends `If-None-Match` when `etag` is
    /// provided; caps the body; never panics on a hostile response.
    pub fn get_json(&self, url: &str, etag: Option<&str>) -> Result<Fetched, FetchError> {
        let mut req = self
            .agent
            .get(url)
            .header("User-Agent", self.user_agent.as_str())
            .header("Accept", "application/json");
        if let Some(tag) = etag {
            if !tag.is_empty() {
                req = req.header("If-None-Match", tag);
            }
        }

        let mut resp = req
            .call()
            .map_err(|e| FetchError::Transport(e.to_string()))?;

        let status = resp.status().as_u16();
        let etag = header(&resp, "etag");
        let rate_remaining = header(&resp, "x-ratelimit-remaining").and_then(|s| s.parse().ok());
        let rate_reset_ms = header(&resp, "x-ratelimit-reset")
            .and_then(|s| s.parse::<i64>().ok())
            .map(|secs| secs.saturating_mul(1000));

        if status == 304 {
            return Ok(Fetched::NotModified);
        }
        if !(200..300).contains(&status) {
            return Err(FetchError::Status(status));
        }

        let text = resp
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_string()
            .map_err(|e| FetchError::Transport(e.to_string()))?;

        Ok(Fetched::Body(FetchedBody {
            text,
            etag,
            rate_remaining,
            rate_reset_ms,
        }))
    }
}

/// Read a response header as an owned `String`, if present and valid UTF-8.
fn header(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_error_displays_cleanly() {
        assert_eq!(FetchError::Status(404).to_string(), "HTTP 404");
        assert_eq!(
            FetchError::Transport("timed out".into()).to_string(),
            "timed out"
        );
    }
}
