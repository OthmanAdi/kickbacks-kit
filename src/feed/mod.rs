//! The live status feed: the one place kb reaches the network, and only ever
//! with read-only GETs of PUBLIC endpoints.
//!
//! Honesty invariant (unchanged): the feed NEVER posts an impression, view, or
//! click, NEVER calls `/metrics` or any loopback `/vibe-ads/<token>/*` billing
//! route, and NEVER touches an auth, portfolio, or earnings endpoint. It reads:
//!   - the GitHub REST API for the public upstream repo (stars, issues, the
//!     synced extension version), and
//!   - the kickbacks.ai PUBLIC status bulletin (`/api/bulletin`), an
//!     unauthenticated informational JSON document the homepage itself fetches.
//!
//! That is the entire network scope. Everything is cached in the local archive,
//! so the status line and snapshots render the feed without any network at all.
//!
//! The X/Twitter maintainer channel is surfaced as a deep link, not scraped:
//! keyless X reads are no longer viable, and a fragile scrape has no place in a
//! tool whose whole pitch is being trustworthy.

pub mod bulletin;
pub mod cli;
pub mod github;
pub mod http;
pub mod sync;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What a feed item is about. Drives the glyph and grouping in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedKind {
    /// A kickbacks.ai service bulletin / PSA (the official status channel).
    Bulletin,
    /// The upstream extension version was synced to a new release.
    Version,
    /// An issue on the upstream repo.
    Issue,
    /// A repository statistic snapshot (stars, open issues).
    Stat,
    /// A static deep link (the maintainer's X profile, the earnings portfolio).
    Link,
}

impl FeedKind {
    /// Persisted tag.
    pub fn as_str(self) -> &'static str {
        match self {
            FeedKind::Bulletin => "bulletin",
            FeedKind::Version => "version",
            FeedKind::Issue => "issue",
            FeedKind::Stat => "stat",
            FeedKind::Link => "link",
        }
    }

    /// Parse a persisted tag, defaulting to `Link` for forward compatibility.
    pub fn from_str(s: &str) -> Self {
        match s {
            "bulletin" => FeedKind::Bulletin,
            "version" => FeedKind::Version,
            "issue" => FeedKind::Issue,
            "stat" => FeedKind::Stat,
            _ => FeedKind::Link,
        }
    }

    /// A one-cell, width-stable glyph (no emoji: terminal width is reliable for
    /// these). The bulletin marker is filled to read as the loud one.
    pub fn glyph(self) -> &'static str {
        match self {
            FeedKind::Bulletin => "●",
            FeedKind::Version => "◆",
            FeedKind::Issue => "○",
            FeedKind::Stat => "★",
            FeedKind::Link => "→",
        }
    }

    /// Display priority (lower sorts first). This is a status feed, so the
    /// reader wants the official bulletin, then which version they are on, then
    /// repo health, before the issue stream and the static links. Recency
    /// orders items within a single kind.
    fn priority(self) -> u8 {
        match self {
            FeedKind::Bulletin => 0,
            FeedKind::Version => 1,
            FeedKind::Stat => 2,
            FeedKind::Issue => 3,
            FeedKind::Link => 4,
        }
    }
}

/// One entry in the feed. `title` and `body` are always sanitized of control
/// characters before they are stored, because they may carry text authored
/// outside this process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedItem {
    pub id: String,
    pub kind: FeedKind,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    /// Event time in epoch millis when known; `None` when the source gives only
    /// an unparseable human string (then the UI orders by fetch time).
    pub ts_ms: Option<i64>,
    pub source: String,
}

impl FeedItem {
    /// Build an item, sanitizing the title and body. A stable id is derived
    /// from kind + url + title so a re-fetch updates the same row.
    pub fn new(
        kind: FeedKind,
        title: impl AsRef<str>,
        body: impl AsRef<str>,
        url: Option<String>,
        ts_ms: Option<i64>,
        source: impl Into<String>,
    ) -> Self {
        let title = crate::util::sanitize_text(title.as_ref());
        let body = crate::util::sanitize_text(body.as_ref());
        let mut h = Sha256::new();
        h.update(kind.as_str().as_bytes());
        h.update(b"\n");
        h.update(url.as_deref().unwrap_or("").as_bytes());
        h.update(b"\n");
        h.update(title.as_bytes());
        let digest = h.finalize();
        let mut id = String::with_capacity(16);
        for b in digest.iter().take(8) {
            id.push_str(&format!("{b:02x}"));
        }
        FeedItem {
            id,
            kind,
            title,
            body,
            url,
            ts_ms,
            source: source.into(),
        }
    }
}

/// Health of a single feed source, persisted so the UI can always say when it
/// last reached a source and whether it is currently backing off. This is the
/// transparency contract: the user can see at any moment what the network is
/// doing and why.
#[derive(Debug, Clone, Default)]
pub struct SourceHealth {
    pub source: String,
    /// Short human status, e.g. `ok`, `up to date`, `offline`, `cooling down`,
    /// `rate limited`, `error: timed out`.
    pub last_status: String,
    pub last_sync_ms: Option<i64>,
    /// Stored ETag for conditional requests (bandwidth saver).
    pub etag: Option<String>,
    pub failures: u32,
    /// While `now < circuit_until_ms` the source is skipped (circuit open).
    pub circuit_until_ms: Option<i64>,
}

/// Everything a read surface needs to render the feed, all from the cache.
#[derive(Debug, Clone, Default)]
pub struct FeedSnapshot {
    pub items: Vec<FeedItem>,
    pub sources: Vec<SourceHealth>,
    /// Most recent successful sync across all network sources.
    pub last_sync_ms: Option<i64>,
    /// True when the network is disabled for this process (flag, config, env).
    pub offline: bool,
}

impl FeedSnapshot {
    /// Items ordered for display: by kind priority (bulletin, version, repo
    /// stats, issues, links), then newest first within a kind. A missing
    /// `ts_ms` sorts oldest.
    pub fn ordered(&self) -> Vec<&FeedItem> {
        let mut out: Vec<&FeedItem> = self.items.iter().collect();
        out.sort_by(|a, b| {
            a.kind.priority().cmp(&b.kind.priority()).then(
                b.ts_ms
                    .unwrap_or(i64::MIN)
                    .cmp(&a.ts_ms.unwrap_or(i64::MIN)),
            )
        });
        out
    }
}

/// The static, never-fetched feed rows: the maintainer's X channel surfaced as
/// a link rather than scraped. Always present so the feed is never empty.
pub fn static_items() -> Vec<FeedItem> {
    vec![FeedItem::new(
        FeedKind::Link,
        "@andrewmccalip on X",
        "Maintainer status, outages, and back-online notes",
        Some("https://x.com/andrewmccalip".to_string()),
        None,
        "static",
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_tag_roundtrips() {
        for k in [
            FeedKind::Bulletin,
            FeedKind::Version,
            FeedKind::Issue,
            FeedKind::Stat,
            FeedKind::Link,
        ] {
            assert_eq!(FeedKind::from_str(k.as_str()), k);
        }
        // Unknown tags decay to Link rather than failing.
        assert_eq!(FeedKind::from_str("who-knows"), FeedKind::Link);
    }

    #[test]
    fn item_sanitizes_and_ids_stably() {
        let a = FeedItem::new(
            FeedKind::Issue,
            "title\x1bwith ctrl",
            "body",
            Some("https://example.com/1".into()),
            Some(10),
            "github",
        );
        let b = FeedItem::new(
            FeedKind::Issue,
            "title\x1bwith ctrl",
            "different body",
            Some("https://example.com/1".into()),
            Some(20),
            "github",
        );
        assert!(!a.title.contains('\x1b'));
        // id keys on kind+url+title, so body/ts changes keep the same row.
        assert_eq!(a.id, b.id);
        assert_eq!(a.id.len(), 16);
    }

    #[test]
    fn ordered_pins_bulletin_then_newest() {
        let snap = FeedSnapshot {
            items: vec![
                FeedItem::new(FeedKind::Issue, "old issue", "", None, Some(100), "github"),
                FeedItem::new(FeedKind::Issue, "new issue", "", None, Some(900), "github"),
                FeedItem::new(FeedKind::Bulletin, "PSA", "", None, None, "bulletin"),
            ],
            ..Default::default()
        };
        let ordered = snap.ordered();
        assert_eq!(ordered[0].kind, FeedKind::Bulletin);
        assert_eq!(ordered[1].title, "new issue");
        assert_eq!(ordered[2].title, "old issue");
    }
}
