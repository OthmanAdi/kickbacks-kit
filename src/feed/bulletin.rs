//! Parser for the kickbacks.ai PUBLIC status bulletin at `/api/bulletin`.
//!
//! This is an unauthenticated, read-only, informational JSON document: the same
//! one the kickbacks.ai homepage fetches client-side to render its service
//! bulletin. kb reads it to surface the maintainer's only real status channel.
//! It is NOT a billing, auth, portfolio, or earnings route, so it stays within
//! the honesty invariant (which exists to block exactly those routes).
//!
//! Pure and fixture-tested. The schema is treated as best-effort: every field
//! defaults, and both the older (`icon`) and current (`ts`) per-entry shapes
//! are accepted, because a status feed must degrade quietly, never crash.

use serde::Deserialize;

use super::{FeedItem, FeedKind};

/// The public bulletin endpoint. No query string, no auth, no cookies.
pub const BULLETIN_URL: &str = "https://kickbacks.ai/api/bulletin";

/// Where a reader can see the bulletin in context (the homepage renders it).
const BULLETIN_LINK: &str = "https://kickbacks.ai/";

#[derive(Debug, Default, Deserialize)]
struct Bulletin {
    #[serde(default)]
    active: bool,
    #[serde(default)]
    id: String,
    #[serde(default)]
    tag: String,
    /// Human-published timestamp string, e.g. "Jun 12, 2026 · 11:49 AM PT".
    /// Not machine-parseable across locales, so it is shown as-is, never parsed
    /// into an ordering key.
    #[serde(default)]
    ts: String,
    #[serde(default)]
    headline: String,
    #[serde(default)]
    items: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
struct Entry {
    /// Current shape: a short human timestamp like "Fri 10:59 AM".
    #[serde(default)]
    ts: Option<String>,
    /// Older shape used a leading glyph here; accepted for resilience.
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    text: String,
}

/// A distilled bulletin: present only when one is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBulletin {
    pub id: String,
    pub tag: String,
    pub published: String,
    pub headline: String,
    pub entries: Vec<String>,
}

/// Parse the bulletin JSON. Returns `None` when no bulletin is active or the
/// payload has no headline, which the UI treats as "nothing to show".
pub fn parse(json: &str) -> Option<ParsedBulletin> {
    let b: Bulletin = serde_json::from_str(json).ok()?;
    if !b.active {
        return None;
    }
    let headline = strip_md(&b.headline);
    if headline.trim().is_empty() {
        return None;
    }
    let entries = b
        .items
        .into_iter()
        .filter(|e| !e.text.trim().is_empty())
        .map(|e| {
            let when = e.ts.or(e.icon).unwrap_or_default();
            let when = when.trim();
            let text = strip_md(&e.text);
            if when.is_empty() {
                text
            } else {
                format!("{when} — {text}")
            }
        })
        .collect();
    Some(ParsedBulletin {
        id: b.id,
        tag: if b.tag.is_empty() {
            "service bulletin".to_string()
        } else {
            b.tag
        },
        published: b.ts,
        headline,
        entries,
    })
}

/// Build the feed item for an active bulletin. The entries are joined into the
/// body with a visible separator; the TUI can re-split on it to show a list,
/// and the one-line surfaces show the joined preview.
pub fn to_item(b: &ParsedBulletin) -> FeedItem {
    let mut body = b.entries.join("  ·  ");
    if body.is_empty() {
        body = b.published.clone();
    }
    FeedItem::new(
        FeedKind::Bulletin,
        &b.headline,
        body,
        Some(BULLETIN_LINK.to_string()),
        None,
        "bulletin",
    )
}

/// Strip the lightweight `**bold**` markdown the bulletin uses, leaving plain
/// text. (Sanitization of control characters happens in `FeedItem::new`.)
fn strip_md(s: &str) -> String {
    s.replace("**", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = r#"{
        "active": true, "id": "2026-06-12-bot-army", "tag": "PSA · Service bulletin",
        "ts": "Jun 12, 2026 · 11:49 AM PT",
        "headline": "A bot army is attacking us. We're winning.",
        "items": [
            {"ts":"Fri 10:59 AM","text":"Stripe payouts are coming, possibly within 48 hours for big accounts."},
            {"ts":"Fri 9:59 AM","text":"Your earnings are safe. Bot traffic never gets paid out."}
        ]
    }"#;

    #[test]
    fn parses_live_bulletin() {
        let b = parse(LIVE).unwrap();
        assert_eq!(b.id, "2026-06-12-bot-army");
        assert_eq!(b.headline, "A bot army is attacking us. We're winning.");
        assert_eq!(b.entries.len(), 2);
        assert!(b.entries[0].starts_with("Fri 10:59 AM — Stripe"));
        let item = to_item(&b);
        assert_eq!(item.kind, FeedKind::Bulletin);
        assert!(item.body.contains("Stripe payouts"));
        assert!(item.body.contains("earnings are safe"));
    }

    #[test]
    fn inactive_bulletin_is_none() {
        assert!(parse(r#"{"active": false, "headline": "old news"}"#).is_none());
        assert!(parse(r#"{"active": true, "headline": ""}"#).is_none());
    }

    #[test]
    fn accepts_legacy_icon_shape() {
        let json =
            r#"{"active":true,"headline":"Heads up","items":[{"icon":"·","text":"something"}]}"#;
        let b = parse(json).unwrap();
        assert_eq!(b.entries, vec!["· — something"]);
    }

    #[test]
    fn strips_bold_markdown() {
        let json = r#"{"active":true,"headline":"We are **winning**","items":[]}"#;
        let b = parse(json).unwrap();
        assert_eq!(b.headline, "We are winning");
    }

    #[test]
    fn garbage_is_none_not_panic() {
        assert!(parse("not json").is_none());
        assert!(parse("{}").is_none());
    }
}
