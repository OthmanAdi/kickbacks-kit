//! Data shapes. The `CliAd` mirrors exactly what the kickbacks.ai extension
//! writes to `~/.vibe-ads/cli-ad.json`. We only ever READ it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Raw shape of `~/.vibe-ads/cli-ad.json` (the current ad served to the CLI
/// status line). Field names match the extension's JSON.
#[derive(Debug, Clone, Deserialize)]
pub struct CliAd {
    #[serde(rename = "adText")]
    pub ad_text: String,
    #[serde(rename = "clickUrl", default)]
    pub click_url: Option<String>,
    #[serde(rename = "iconUrl", default)]
    pub icon_url: Option<String>,
    /// Kept for schema fidelity with the extension's JSON; not used yet.
    #[serde(rename = "iconRef", default)]
    #[allow(dead_code)]
    pub icon_ref: Option<String>,
    /// Rotation timestamp in epoch milliseconds (set by the extension).
    pub ts: i64,
}

impl CliAd {
    /// Stable id for an ad creative: first 16 hex chars of
    /// sha256(click_url \n ad_text). Two rotations of the same creative
    /// collapse to one row in the archive.
    pub fn id(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.click_url.as_deref().unwrap_or("").as_bytes());
        h.update(b"\n");
        h.update(self.ad_text.as_bytes());
        let digest = h.finalize();
        let mut s = String::with_capacity(16);
        for b in digest.iter().take(8) {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    /// Best-effort advertiser name. The extension's creatives follow the
    /// pattern "Advertiser · tagline" (middot separator), so we take the part
    /// before the first middot; otherwise fall back to the click_url host.
    pub fn advertiser(&self) -> String {
        if let Some((head, _)) = self.ad_text.split_once(" · ") {
            let head = head.trim();
            if !head.is_empty() {
                return head.to_string();
            }
        }
        if let Some(url) = &self.click_url {
            if let Some(host) = host_of(url) {
                return host;
            }
        }
        "unknown".to_string()
    }
}

/// One row of the `ads` table: a distinct creative we have observed.
#[derive(Debug, Clone, Serialize)]
pub struct AdRow {
    pub id: String,
    pub advertiser: String,
    pub ad_text: String,
    pub click_url: Option<String>,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub times_seen: i64,
}

/// Extract the host portion of a URL without pulling in a URL crate.
pub fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split('@').next_back().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}
