//! The `kb feed` command: fetch the live status feed once (unless offline) and
//! render it to stdout, as readable text or as JSON. This is the headless,
//! scriptable twin of the Feed tab in `kb top`.

use anyhow::Result;
use crossterm::style::Stylize;

use crate::archive::Archive;
use crate::config;
use crate::feed::sync::{self, OfflineReason};
use crate::feed::FeedSnapshot;
use crate::{paths, util};

/// Run `kb feed`. Fetches and refreshes the cache unless the network is off, in
/// which case it renders whatever is cached. `as_json` dumps the items as JSON.
pub fn run(offline: bool, as_json: bool) -> Result<()> {
    let cfg = config::load();
    let reason = sync::offline_reason(offline, &cfg);
    let mut archive = Archive::open(&paths::db_path()?)?;
    let snapshot = sync::sync(&mut archive, &cfg, offline);

    if as_json {
        println!("{}", serde_json::to_string_pretty(&snapshot.ordered())?);
        return Ok(());
    }

    let color = std::io::IsTerminal::is_terminal(&std::io::stdout());
    print!("{}", render_text(&snapshot, util::now_ms(), color, reason));
    Ok(())
}

/// Render the feed as a block of text. Pure, so it is unit tested. `color`
/// gates ANSI styling (off when piped); `offline` carries the precise reason
/// the network was skipped, when it was.
pub fn render_text(
    snapshot: &FeedSnapshot,
    now_ms: i64,
    color: bool,
    offline: Option<OfflineReason>,
) -> String {
    let mut out = String::new();
    let dim = |s: &str| {
        if color {
            s.dim().to_string()
        } else {
            s.to_string()
        }
    };
    let bold = |s: &str| {
        if color {
            s.bold().to_string()
        } else {
            s.to_string()
        }
    };

    let status = network_line(snapshot, now_ms, offline);
    out.push_str(&format!(
        "{}   {}\n\n",
        bold("kickbacks-kit · live feed"),
        dim(&status)
    ));

    for item in snapshot.ordered() {
        let age = item
            .ts_ms
            .map(|ts| util::human_age_short(now_ms - ts))
            .unwrap_or_default();
        let glyph = item.kind.glyph();
        let title = util::truncate(&item.title, 64);
        let head = if age.is_empty() {
            format!("  {glyph} {}", bold(&title))
        } else {
            format!("  {glyph} {}  {}", bold(&title), dim(&age))
        };
        out.push_str(&head);
        out.push('\n');

        if !item.body.is_empty() {
            // The bulletin packs its entries into the body with a separator;
            // show them, wrapped softly to a readable width.
            for line in wrap(&item.body, 68) {
                out.push_str(&format!("     {}\n", dim(&line)));
            }
        }
        if let Some(url) = &item.url {
            out.push_str(&format!("     {}\n", dim(url)));
        }
        out.push('\n');
    }

    let footer = sources_line(snapshot);
    if !footer.is_empty() {
        out.push_str(&format!("{}\n", dim(&footer)));
    }
    out
}

/// A compact per-source health footer, so the user can always see exactly which
/// endpoints were reached and their state. Skips the static (never-fetched)
/// pseudo-source.
fn sources_line(snapshot: &FeedSnapshot) -> String {
    let parts: Vec<String> = snapshot
        .sources
        .iter()
        .filter(|s| s.source != "static")
        .map(|s| {
            let status = if s.last_status.is_empty() {
                "pending"
            } else {
                &s.last_status
            };
            format!("{} {}", s.source, status)
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("  sources · {}", parts.join(" · "))
    }
}

/// The one-line network status shown under the title: the precise offline
/// reason when the network was skipped, otherwise how long ago the feed last
/// reached any source. This is the transparency contract in one line.
fn network_line(snapshot: &FeedSnapshot, now_ms: i64, offline: Option<OfflineReason>) -> String {
    if let Some(reason) = offline {
        return format!("{} · showing cached feed", describe_offline(reason));
    }
    if snapshot.offline {
        return "offline · showing cached feed".to_string();
    }
    match snapshot.last_sync_ms {
        Some(ms) => format!("synced {}", util::human_age(now_ms - ms)),
        None => "not synced yet".to_string(),
    }
}

/// Describe an offline reason in words.
pub fn describe_offline(reason: OfflineReason) -> &'static str {
    match reason {
        OfflineReason::Flag => "offline (--offline)",
        OfflineReason::Config => "offline (feed disabled in config)",
        OfflineReason::Env => "offline (KICKBACKS_KIT_OFFLINE set)",
    }
}

/// Soft word-wrap to `width` columns, splitting on the bulletin separator and
/// on spaces. Never splits mid-word; long words are passed through whole.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for chunk in text.split("  ·  ") {
        let mut line = String::new();
        for word in chunk.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::{FeedItem, FeedKind};

    fn snap(items: Vec<FeedItem>, offline: bool, last_sync: Option<i64>) -> FeedSnapshot {
        FeedSnapshot {
            items,
            sources: Vec::new(),
            last_sync_ms: last_sync,
            offline,
        }
    }

    #[test]
    fn renders_bulletin_and_issue_plainly() {
        let now = 1_000_000_000;
        let items = vec![
            FeedItem::new(
                FeedKind::Bulletin,
                "A bot army is attacking us",
                "Stripe payouts coming  ·  earnings are safe",
                Some("https://kickbacks.ai/".into()),
                None,
                "bulletin",
            ),
            FeedItem::new(
                FeedKind::Issue,
                "#52 no kickback earned",
                "open issue",
                Some("https://github.com/x/issues/52".into()),
                Some(now - 3_600_000),
                "github",
            ),
        ];
        let text = render_text(&snap(items, false, Some(now - 120_000)), now, false, None);
        assert!(text.contains("A bot army is attacking us"));
        assert!(text.contains("Stripe payouts coming"));
        assert!(text.contains("earnings are safe"));
        assert!(text.contains("#52 no kickback earned"));
        assert!(text.contains("1h")); // issue age
        assert!(text.contains("synced 2m ago"));
        // Bulletin is pinned first.
        let bull = text.find("bot army").unwrap();
        let issue = text.find("#52").unwrap();
        assert!(bull < issue);
    }

    #[test]
    fn offline_is_labeled() {
        let text = render_text(
            &snap(Vec::new(), true, None),
            10,
            false,
            Some(OfflineReason::Flag),
        );
        assert!(text.contains("offline"));
    }

    #[test]
    fn wrap_breaks_on_separator_and_width() {
        let lines = wrap("alpha beta  ·  gamma", 100);
        assert_eq!(lines, vec!["alpha beta", "gamma"]);
        let narrow = wrap("one two three four five", 8);
        assert!(narrow.len() > 1);
        assert!(narrow.iter().all(|l| l.chars().count() <= 9));
    }
}
