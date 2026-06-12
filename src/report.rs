//! `kb report` — a one-shot digest of your local ad archive, as Markdown or a
//! self-contained HTML page: the summary, the advertiser leaderboard, the
//! recent links, and the killswitch timeline in one document you can keep or
//! share.
//!
//! Like everything else in kb it is read-only and honest: it reports the
//! history kb observed, it never reads or invents an earnings figure, and it
//! points at the real portfolio for that.

use anyhow::{Context, Result};
use clap::ValueEnum;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use crate::archive::{AdvertiserStat, Archive, KillswitchEvent, LinkRow, Stats};
use crate::model::host_of;
use crate::render::PORTFOLIO_URL;
use crate::util::html_escape;
use crate::{paths, util};

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Format {
    /// GitHub-flavored Markdown (the default).
    Md,
    /// A self-contained HTML page.
    Html,
}

/// Everything the report renders, gathered once from the archive.
pub struct Digest {
    pub generated_ms: i64,
    pub stats: Stats,
    pub advertisers: Vec<AdvertiserStat>,
    pub links: Vec<LinkRow>,
    pub killswitch: Vec<KillswitchEvent>,
}

/// Gather the digest from the archive at `now_ms`.
pub fn gather(archive: &Archive, now_ms: i64) -> Result<Digest> {
    Ok(Digest {
        generated_ms: now_ms,
        stats: archive.stats(now_ms)?,
        advertisers: archive.advertiser_leaderboard(10)?,
        links: archive.distinct_links(15)?,
        killswitch: archive.killswitch_timeline(10)?,
    })
}

/// Run `kb report`.
pub fn run(format: Format, out: Option<PathBuf>) -> Result<()> {
    let archive = Archive::open(&paths::db_path()?)?;
    let digest = gather(&archive, util::now_ms())?;
    let body = match format {
        Format::Md => render_markdown(&digest),
        Format::Html => render_html(&digest),
    };
    match &out {
        Some(p) => {
            let mut w = BufWriter::new(
                File::create(p).with_context(|| format!("creating {}", p.display()))?,
            );
            write!(w, "{body}")?;
            w.flush()?;
            eprintln!("wrote report -> {}", p.display());
        }
        None => print!("{body}"),
    }
    Ok(())
}

/// A short human label for a killswitch transition.
fn killswitch_label(killed: bool) -> &'static str {
    if killed {
        "paused (kickbacks killswitch active)"
    } else {
        "resumed"
    }
}

pub fn render_markdown(d: &Digest) -> String {
    let mut s = String::new();
    s.push_str("# kickbacks-kit report\n\n");
    s.push_str(&format!(
        "Generated {}. Read-only: kb records the ads you were shown and never reports a billing event.\n\n",
        util::fmt_datetime(d.generated_ms)
    ));

    s.push_str("## Summary\n\n");
    s.push_str(&format!("- Ads seen: {}\n", d.stats.distinct_ads));
    s.push_str(&format!("- Advertisers: {}\n", d.stats.advertisers));
    s.push_str(&format!(
        "- Sightings: {} (today {}, this week {})\n",
        d.stats.total_sightings, d.stats.sightings_today, d.stats.sightings_week
    ));
    if let (Some(first), Some(last)) = (d.stats.first_seen_ms, d.stats.last_seen_ms) {
        s.push_str(&format!(
            "- First seen {}, last seen {}\n",
            util::fmt_datetime(first),
            util::fmt_datetime(last)
        ));
    }
    s.push('\n');

    s.push_str("## Top advertisers\n\n");
    if d.advertisers.is_empty() {
        s.push_str("None captured yet.\n\n");
    } else {
        s.push_str("| # | advertiser | ads | sightings |\n|--:|:--|--:|--:|\n");
        for (i, a) in d.advertisers.iter().enumerate() {
            s.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                i + 1,
                md_cell(&a.advertiser),
                a.distinct_ads,
                a.sightings
            ));
        }
        s.push('\n');
    }

    s.push_str("## Recent links\n\n");
    if d.links.is_empty() {
        s.push_str("None captured yet.\n\n");
    } else {
        for l in &d.links {
            let host = host_of(&l.url).unwrap_or_default();
            // Angle-bracket the destination so a `)` in a tracking URL cannot
            // close the link early and inject text; escape the label and host.
            s.push_str(&format!(
                "- [{}](<{}>) · {} · {}×\n",
                md_cell(&l.advertiser),
                md_url(&l.url),
                md_cell(&host),
                l.times_seen
            ));
        }
        s.push('\n');
    }

    s.push_str("## Killswitch timeline\n\n");
    if d.killswitch.is_empty() {
        s.push_str("No killswitch events recorded.\n\n");
    } else {
        for e in &d.killswitch {
            s.push_str(&format!(
                "- {}: {}\n",
                util::fmt_datetime(e.ts_ms),
                killswitch_label(e.killed)
            ));
        }
        s.push('\n');
    }

    s.push_str("## Earnings\n\n");
    s.push_str(&format!(
        "kb does not read your balance: that needs the kickbacks.ai cloud backend, and kb stays read-only and offline. Your portfolio is at {PORTFOLIO_URL}.\n"
    ));
    s
}

/// Escape the characters that would break a Markdown table cell or a link
/// label. Also neutralizes a backtick so a value cannot open an inline code
/// span, and a pipe so it cannot split a table row.
fn md_cell(s: &str) -> String {
    s.replace('|', "\\|")
        .replace('`', "\\`")
        .replace(['\n', '\r'], " ")
}

/// Make a URL safe inside an angle-bracket Markdown destination `<...>`: drop
/// the angle brackets and any newline that would terminate it. The surrounding
/// `<>` then makes a `)` in the URL harmless.
fn md_url(s: &str) -> String {
    s.replace(['<', '>', '\n', '\r'], "")
}

pub fn render_html(d: &Digest) -> String {
    let mut rows = String::new();
    for (i, a) in d.advertisers.iter().enumerate() {
        rows.push_str(&format!(
            "    <tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            i + 1,
            html_escape(&a.advertiser),
            a.distinct_ads,
            a.sightings
        ));
    }
    let mut links = String::new();
    for l in &d.links {
        let host = host_of(&l.url).unwrap_or_default();
        links.push_str(&format!(
            "    <li><a href=\"{}\">{}</a> <span class=\"host\">{}</span> <span class=\"meta\">{}\u{00d7}</span></li>\n",
            html_escape(&l.url),
            html_escape(&l.advertiser),
            html_escape(&host),
            l.times_seen
        ));
    }
    let mut kill = String::new();
    if d.killswitch.is_empty() {
        kill.push_str("    <li class=\"meta\">No killswitch events recorded.</li>\n");
    } else {
        for e in &d.killswitch {
            kill.push_str(&format!(
                "    <li>{} <span class=\"meta\">{}</span></li>\n",
                html_escape(&util::fmt_datetime(e.ts_ms)),
                html_escape(killswitch_label(e.killed))
            ));
        }
    }

    let (first_last, since) = match (d.stats.first_seen_ms, d.stats.last_seen_ms) {
        (Some(f), Some(l)) => (
            true,
            format!(
                "first seen {}, last seen {}",
                util::fmt_datetime(f),
                util::fmt_datetime(l)
            ),
        ),
        _ => (false, String::new()),
    };

    format!(
        "<!doctype html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>kickbacks-kit report</title>
<style>
  body {{ font: 15px/1.6 ui-sans-serif, system-ui, sans-serif; max-width: 52rem;
         margin: 2rem auto; padding: 0 1rem; color: #1a1a1a; }}
  h1 {{ font-size: 1.4rem; }}
  h2 {{ font-size: 1.05rem; margin-top: 2rem; border-bottom: 1px solid #eee; padding-bottom: .3rem; }}
  table {{ border-collapse: collapse; width: 100%; }}
  th, td {{ text-align: left; padding: .35rem .6rem; border-bottom: 1px solid #f0f0f0; }}
  td:last-child, th:last-child, td:nth-child(3), th:nth-child(3) {{ text-align: right; }}
  ul {{ list-style: none; padding: 0; }}
  li {{ padding: .3rem 0; border-bottom: 1px solid #f4f4f4; }}
  a {{ color: #0b62d6; text-decoration: none; }}
  .host {{ color: #888; font-size: .85em; }}
  .meta {{ color: #aaa; font-size: .85em; float: right; }}
  .sub {{ color: #666; }}
</style>
</head>
<body>
<h1>kickbacks-kit report</h1>
<p class=\"sub\">Generated {generated}. Read-only: kb records the ads you were shown and never reports a billing event.</p>

<h2>Summary</h2>
<ul>
  <li>Ads seen <span class=\"meta\">{ads}</span></li>
  <li>Advertisers <span class=\"meta\">{advertisers}</span></li>
  <li>Sightings <span class=\"meta\">{sightings} (today {today}, week {week})</span></li>
  {since_li}
</ul>

<h2>Top advertisers</h2>
<table>
  <tr><th>#</th><th>advertiser</th><th>ads</th><th>sightings</th></tr>
{rows}</table>

<h2>Recent links</h2>
<ul>
{links}</ul>

<h2>Killswitch timeline</h2>
<ul>
{kill}</ul>

<h2>Earnings</h2>
<p>kb does not read your balance: that needs the kickbacks.ai cloud backend, and kb stays read-only and offline. Your portfolio is at <a href=\"{portfolio}\">{portfolio}</a>.</p>
</body>
</html>
",
        generated = html_escape(&util::fmt_datetime(d.generated_ms)),
        ads = d.stats.distinct_ads,
        advertisers = d.stats.advertisers,
        sightings = d.stats.total_sightings,
        today = d.stats.sightings_today,
        week = d.stats.sightings_week,
        since_li = if first_last {
            format!("<li class=\"sub\">{}</li>", html_escape(&since))
        } else {
            String::new()
        },
        rows = rows,
        links = links,
        kill = kill,
        portfolio = PORTFOLIO_URL,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest() -> Digest {
        Digest {
            generated_ms: 1_781_200_000_000,
            stats: Stats {
                distinct_ads: 65,
                advertisers: 49,
                total_sightings: 254,
                sightings_today: 254,
                sightings_week: 254,
                first_seen_ms: Some(1_781_100_000_000),
                last_seen_ms: Some(1_781_199_000_000),
            },
            advertisers: vec![AdvertiserStat {
                advertiser: "Tailscale | VPN".to_string(),
                distinct_ads: 2,
                sightings: 21,
            }],
            links: vec![LinkRow {
                advertiser: "Ramp".to_string(),
                url: "https://ramp.com/?a=1&b=2".to_string(),
                last_seen_ms: 1_781_199_000_000,
                times_seen: 8,
            }],
            killswitch: vec![
                KillswitchEvent {
                    ts_ms: 1_781_198_000_000,
                    ts_iso: "x".to_string(),
                    killed: false,
                },
                KillswitchEvent {
                    ts_ms: 1_781_197_000_000,
                    ts_iso: "y".to_string(),
                    killed: true,
                },
            ],
        }
    }

    #[test]
    fn markdown_link_url_cannot_inject() {
        let mut d = digest();
        // A click URL containing ')' would close a plain Markdown link early.
        d.links = vec![LinkRow {
            advertiser: "Evil".to_string(),
            url: "https://a.com/b)injected".to_string(),
            last_seen_ms: 0,
            times_seen: 1,
        }];
        let md = render_markdown(&d);
        // Angle-bracket destination keeps the whole URL inside the link.
        assert!(md.contains("](<https://a.com/b)injected>)"));
        assert!(!md.contains("](https://a.com/b)injected)"));
    }

    #[test]
    fn markdown_has_all_sections_and_escapes_pipes() {
        let md = render_markdown(&digest());
        assert!(md.contains("# kickbacks-kit report"));
        assert!(md.contains("## Summary"));
        assert!(md.contains("Ads seen: 65"));
        assert!(md.contains("## Top advertisers"));
        // A pipe in an advertiser name is escaped so the table is not broken.
        assert!(md.contains("Tailscale \\| VPN"));
        assert!(md.contains("## Killswitch timeline"));
        assert!(md.contains("resumed"));
        assert!(md.contains("paused (kickbacks killswitch active)"));
        // Honesty: points at the portfolio, never a number.
        assert!(md.contains("kickbacks.ai/me"));
        assert!(!md.to_lowercase().contains("you earned"));
    }

    #[test]
    fn html_escapes_and_is_self_contained() {
        let html = render_html(&digest());
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("kickbacks-kit report"));
        // The ampersand URL is escaped in the href.
        assert!(html.contains("a=1&amp;b=2"));
        assert!(html.contains("No killswitch events recorded.") || html.contains("resumed"));
        assert!(html.contains("kickbacks.ai/me"));
    }

    #[test]
    fn empty_archive_report_is_graceful() {
        let empty = Digest {
            generated_ms: 0,
            stats: Stats::default(),
            advertisers: vec![],
            links: vec![],
            killswitch: vec![],
        };
        let md = render_markdown(&empty);
        assert!(md.contains("None captured yet."));
        assert!(md.contains("No killswitch events recorded."));
    }
}
