//! `kb statusline` — one compact line for a CLI status line (Claude Code's
//! statusLine setting). It keeps the extension's ad front and center exactly
//! like the stock kickbacks script (prefix, hyperlink, control-char stripping),
//! appends kb's own stats, and, when there is room, a single status alert
//! drawn from the cached feed (a kickbacks service bulletin, or a new extension
//! version). The ad is never suppressed or shortened below readability:
//! showing it is what earns.
//!
//! Speed and honesty: the status line is on the host's hot path, so it NEVER
//! goes to the network. The feed alert is read from the local cache that
//! `kb top`, `kb watch`, and `kb feed` populate; if the cache is cold there is
//! simply no alert. Each invocation runs one capture pass, so a wired-up status
//! line quietly grows the archive while you code. If anything is unavailable
//! the line still prints; this must never break the host CLI.

use std::cmp::Ordering;
use std::io::{IsTerminal, Read};

use anyhow::Result;

use crate::archive::{Archive, Stats};
use crate::capture::capture_pass;
use crate::feed::{sync, FeedSnapshot};
use crate::model::CliAd;
use crate::sources::{self, AdStatus};
use crate::{paths, util};

/// `cli-ad.json` freshness window, mirroring the extension.
const FRESH_MS: i64 = 600_000;

/// Visible separator between segments.
const SEP: &str = "  ·  ";

/// The ad keeps at least this many columns even on narrow terminals.
const MIN_AD_COLS: usize = 16;

/// A feed alert is only shown when the ad still has at least this much room, so
/// the ad (which earns) always wins the width budget.
const AD_FLOOR_FOR_ALERT: usize = 24;

/// The longest a feed alert may grow before it is truncated.
const MAX_ALERT_COLS: usize = 38;

/// A one-line status alert distilled from the cached feed.
struct Alert {
    text: String,
    /// True for an urgent alert (a service bulletin), which is colored red.
    urgent: bool,
}

pub fn run(width: Option<u16>, plain: bool, no_feed: bool) -> Result<()> {
    // Claude Code pipes a JSON payload to statusline commands; consume it.
    if !std::io::stdin().is_terminal() {
        let mut sink = String::new();
        std::io::stdin().read_to_string(&mut sink).ok();
    }
    let width = width
        .map(usize::from)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
        })
        .unwrap_or(120);

    let now = util::now_ms();
    let ad = sources::read_cli_ad().ok().flatten();
    let state = sources::read_live_state().unwrap_or_default();
    let fresh = ad.as_ref().map(|a| now - a.ts <= FRESH_MS).unwrap_or(false);
    let status = sources::ad_status(&state, fresh);

    // Best-effort capture, stats, and a cache-only feed alert. Any failure
    // leaves that piece off the line rather than failing the host's status bar.
    // The feed read is strictly local (no network), keeping this fast.
    let mut alert = None;
    let stats = paths::db_path()
        .ok()
        .and_then(|p| Archive::open(&p).ok())
        .and_then(|mut a| {
            capture_pass(&mut a).ok();
            if !no_feed {
                alert = feed_alert(&sync::read_cached(&a, true));
            }
            a.stats(now).ok()
        });

    let line = format_line(
        ad.as_ref().filter(|_| fresh),
        stats.as_ref(),
        status,
        width,
        !plain,
        alert.as_ref(),
    );
    println!("{line}");
    Ok(())
}

/// Distill a one-line alert from the cached feed: an active service bulletin
/// first (urgent), otherwise a newer extension version than the one installed.
fn feed_alert(snapshot: &FeedSnapshot) -> Option<Alert> {
    if let Some(headline) = snapshot.bulletin_headline() {
        return Some(Alert {
            text: format!("▲ {headline}"),
            urgent: true,
        });
    }
    let installed = sources::installed_extension_version()?;
    let latest = snapshot.latest_version()?;
    if util::version_cmp(&installed, &latest) == Ordering::Less {
        return Some(Alert {
            text: format!("↑ ext v{latest}"),
            urgent: false,
        });
    }
    None
}

/// Build the line. Pure so it is unit tested. `color` adds the status dot
/// color, the OSC 8 hyperlink on the ad, and the alert color (escapes take no
/// columns). The width ladder is: keep the ad and the kb stats; add the alert
/// only when the ad still has comfortable room; drop the alert before
/// truncating the ad.
fn format_line(
    ad: Option<&CliAd>,
    stats: Option<&Stats>,
    status: AdStatus,
    width: usize,
    color: bool,
    alert: Option<&Alert>,
) -> String {
    let (dot, label) = match status {
        AdStatus::Live => ("●", "live"),
        AdStatus::Idle => ("○", "idle"),
        AdStatus::Paused => ("●", "paused"),
        AdStatus::InjectionOff => ("○", "ads off"),
        AdStatus::SignedOut => ("○", "signed out"),
    };
    let kb_plain = match stats {
        Some(s) => format!(
            "kb {} today · {} advs · {dot} {label}",
            s.sightings_today, s.advertisers
        ),
        None => format!("kb {dot} {label}"),
    };
    let kb_out = if color {
        let colored = match status {
            AdStatus::Live => format!("\x1b[32m{dot}\x1b[0m"),
            AdStatus::Paused => format!("\x1b[31m{dot}\x1b[0m"),
            _ => format!("\x1b[33m{dot}\x1b[0m"),
        };
        kb_plain.replacen(dot, &colored, 1)
    } else {
        kb_plain.clone()
    };
    let kb_cols = kb_plain.chars().count();
    let sep_cols = SEP.chars().count();

    // The alert, truncated to its cap, with its column cost.
    let alert_text = alert.map(|a| util::truncate(&a.text, MAX_ALERT_COLS));
    let alert_cols = alert_text
        .as_ref()
        .map(|t| sep_cols + t.chars().count())
        .unwrap_or(0);

    let Some(ad) = ad else {
        // No ad: stats, then the alert if it fits at all.
        let room = width.saturating_sub(kb_cols);
        if alert_cols > 0 && room >= alert_cols {
            let a = alert.unwrap();
            return format!(
                "{kb_out}{}",
                styled_alert(&alert_text.unwrap(), a.urgent, color)
            );
        }
        return kb_out;
    };

    // Decide whether the alert fits while leaving the ad a comfortable floor.
    let ad_room_with_alert = width.saturating_sub(kb_cols + sep_cols + alert_cols);
    let show_alert = alert_cols > 0 && ad_room_with_alert >= AD_FLOOR_FOR_ALERT;
    let ad_budget = if show_alert {
        ad_room_with_alert
    } else {
        width.saturating_sub(kb_cols + sep_cols)
    }
    .max(MIN_AD_COLS);

    let text = util::truncate(&format!("ad· {}", strip_controls(&ad.ad_text)), ad_budget);
    let ad_out = match ad.click_url.as_deref().filter(|_| color) {
        // OSC 8 hyperlink, exactly like the stock kickbacks status line.
        Some(url) => format!(
            "\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\",
            strip_controls(url),
            text
        ),
        None => text,
    };

    let mut line = format!("{ad_out}{SEP}{kb_out}");
    if show_alert {
        line.push_str(&styled_alert(
            &alert_text.unwrap(),
            alert.unwrap().urgent,
            color,
        ));
    }
    line
}

/// The alert segment, with its leading separator, colored when enabled.
fn styled_alert(text: &str, urgent: bool, color: bool) -> String {
    if color {
        let code = if urgent { "31" } else { "33" };
        format!("{SEP}\x1b[{code}m{text}\x1b[0m")
    } else {
        format!("{SEP}{text}")
    }
}

/// Remove C0, DEL, and C1 control characters so ad-supplied text can never
/// emit its own terminal escapes. Mirrors the extension's own sanitizer.
fn strip_controls(s: &str) -> String {
    s.chars()
        .filter(|&c| !c.is_control() && !('\u{80}'..='\u{9f}').contains(&c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(text: &str) -> CliAd {
        CliAd {
            ad_text: text.to_string(),
            click_url: Some("https://example.com/".to_string()),
            icon_url: None,
            icon_ref: None,
            ts: 0,
        }
    }

    fn stats() -> Stats {
        Stats {
            sightings_today: 12,
            advertisers: 17,
            ..Stats::default()
        }
    }

    fn psa() -> Alert {
        Alert {
            text: "▲ A bot army is attacking us".to_string(),
            urgent: true,
        }
    }

    #[test]
    fn ad_and_stats_share_the_line() {
        let a = ad("Ramp - business cards that close themselves");
        let line = format_line(Some(&a), Some(&stats()), AdStatus::Live, 120, false, None);
        assert_eq!(
            line,
            "ad· Ramp - business cards that close themselves  ·  kb 12 today · 17 advs · ● live"
        );
    }

    #[test]
    fn without_ad_only_stats_print() {
        let line = format_line(None, Some(&stats()), AdStatus::Idle, 120, false, None);
        assert_eq!(line, "kb 12 today · 17 advs · ○ idle");
    }

    #[test]
    fn long_ad_is_truncated_to_width_but_never_dropped() {
        let a = ad(&"x".repeat(300));
        let line = format_line(Some(&a), Some(&stats()), AdStatus::Live, 80, false, None);
        assert!(line.chars().count() <= 80);
        assert!(line.starts_with("ad· "));
        assert!(line.contains("kb 12 today"));
    }

    #[test]
    fn colored_ad_carries_an_osc8_hyperlink() {
        let a = ad("Solo - run your agents");
        let line = format_line(Some(&a), Some(&stats()), AdStatus::Live, 120, true, None);
        assert!(line.contains("\x1b]8;;https://example.com/"));
        assert!(line.contains("\x1b[32m●\x1b[0m"));
    }

    #[test]
    fn control_chars_in_ad_text_are_stripped() {
        let a = ad("Evil\x1b]0;pwned\x07 - ad");
        let line = format_line(Some(&a), None, AdStatus::Live, 120, false, None);
        assert!(!line.contains('\x1b'));
        assert!(!line.contains('\x07'));
        assert!(line.contains("Evil]0;pwned - ad"));
    }

    #[test]
    fn paused_status_is_visible_without_stats() {
        let line = format_line(None, None, AdStatus::Paused, 120, false, None);
        assert_eq!(line, "kb ● paused");
    }

    #[test]
    fn alert_appended_when_there_is_room() {
        let a = ad("Ramp - cards");
        let line = format_line(
            Some(&a),
            Some(&stats()),
            AdStatus::Live,
            120,
            false,
            Some(&psa()),
        );
        assert!(line.contains("▲ A bot army"));
        // The ad still leads the line.
        assert!(line.starts_with("ad· Ramp"));
    }

    #[test]
    fn alert_is_dropped_before_the_ad_is_squeezed() {
        // A long ad on a narrow line: the alert yields so the ad keeps its room.
        let a = ad(&"x".repeat(200));
        let line = format_line(
            Some(&a),
            Some(&stats()),
            AdStatus::Live,
            80,
            false,
            Some(&psa()),
        );
        assert!(line.chars().count() <= 80);
        assert!(!line.contains("bot army"));
        assert!(line.contains("kb 12 today"));
    }

    #[test]
    fn urgent_alert_is_red_when_colored() {
        let line = format_line(
            None,
            Some(&stats()),
            AdStatus::Live,
            120,
            true,
            Some(&psa()),
        );
        assert!(line.contains("\x1b[31m▲ A bot army"));
    }
}
