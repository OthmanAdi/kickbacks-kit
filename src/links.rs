//! `kb links` — export the distinct advertiser destinations you have been shown
//! as a list, CSV, JSONL, or a self-contained HTML bookmarks page.
//!
//! These are the click-through URLs the kickbacks.ai extension wrote to the
//! local ad file; kb only ever read them. Exporting is the same read-only,
//! your-data-is-yours promise as `kb export`, focused on the links.

use anyhow::{Context, Result};
use clap::ValueEnum;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use crate::archive::{Archive, LinkRow};
use crate::model::host_of;
use crate::util::{csv_field, html_escape};
use crate::{paths, util};

/// Most links any one export carries. Generous: the corpus is small.
const LIMIT: usize = 2000;

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Format {
    /// Aligned, human-readable columns (the default).
    Table,
    /// Comma-separated values with a header row.
    Csv,
    /// One JSON object per line.
    Jsonl,
    /// A self-contained HTML bookmarks page.
    Html,
}

/// Export the distinct advertiser links.
pub fn run(format: Format, out: Option<PathBuf>) -> Result<()> {
    let archive = Archive::open(&paths::db_path()?)?;
    let links = archive.distinct_links(LIMIT)?;
    let now = util::now_ms();

    let mut writer: Box<dyn Write> = match &out {
        Some(p) => Box::new(BufWriter::new(
            File::create(p).with_context(|| format!("creating {}", p.display()))?,
        )),
        None => Box::new(BufWriter::new(io::stdout().lock())),
    };

    let body = match format {
        Format::Table => render_table(&links, now),
        Format::Csv => render_csv(&links),
        Format::Jsonl => render_jsonl(&links)?,
        Format::Html => render_html(&links, now),
    };
    write!(writer, "{body}")?;
    writer.flush()?;

    if let Some(p) = out {
        eprintln!("exported {} links -> {}", links.len(), p.display());
    }
    Ok(())
}

/// A JSON-serializable view of a link (LinkRow itself is not Serialize).
#[derive(serde::Serialize)]
struct LinkJson<'a> {
    advertiser: &'a str,
    url: &'a str,
    host: Option<String>,
    times_seen: i64,
    last_seen_ms: i64,
}

fn render_jsonl(links: &[LinkRow]) -> Result<String> {
    let mut out = String::new();
    for l in links {
        let row = LinkJson {
            advertiser: &l.advertiser,
            url: &l.url,
            host: host_of(&l.url),
            times_seen: l.times_seen,
            last_seen_ms: l.last_seen_ms,
        };
        out.push_str(&serde_json::to_string(&row)?);
        out.push('\n');
    }
    Ok(out)
}

fn render_csv(links: &[LinkRow]) -> String {
    let mut out = String::from("advertiser,url,times_seen,last_seen_ms\n");
    for l in links {
        out.push_str(&format!(
            "{},{},{},{}\n",
            csv_field(&l.advertiser),
            csv_field(&l.url),
            l.times_seen,
            l.last_seen_ms,
        ));
    }
    out
}

fn render_table(links: &[LinkRow], now_ms: i64) -> String {
    if links.is_empty() {
        return "no advertiser links captured yet — run kb setup\n".to_string();
    }
    let mut out = format!("{:<22}  {:>5}  {:>7}  url\n", "advertiser", "seen", "last");
    for l in links {
        // The table prints to a terminal, so strip control characters from the
        // advertiser-supplied text and URL: they could otherwise emit their own
        // escape sequences. The structured formats (CSV, JSONL, HTML) escape on
        // their own terms.
        out.push_str(&format!(
            "{:<22}  {:>5}  {:>7}  {}\n",
            util::truncate(&util::sanitize_text(&l.advertiser), 22),
            l.times_seen,
            util::human_age_short(now_ms - l.last_seen_ms),
            util::sanitize_text(&l.url),
        ));
    }
    out
}

/// A self-contained, dependency-free bookmarks page. All advertiser text and
/// URLs are HTML-escaped, so a hostile creative cannot inject markup.
fn render_html(links: &[LinkRow], now_ms: i64) -> String {
    let mut rows = String::new();
    for l in links {
        let host = host_of(&l.url).unwrap_or_default();
        rows.push_str(&format!(
            "    <li><a href=\"{url}\">{adv}</a> <span class=\"host\">{host}</span> \
             <span class=\"meta\">{n}\u{00d7} \u{00b7} {age}</span></li>\n",
            url = html_escape(&l.url),
            adv = html_escape(&l.advertiser),
            host = html_escape(&host),
            n = l.times_seen,
            age = html_escape(&util::human_age_short(now_ms - l.last_seen_ms)),
        ));
    }
    format!(
        "<!doctype html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>kickbacks-kit · advertiser links</title>
<style>
  body {{ font: 15px/1.5 ui-sans-serif, system-ui, sans-serif; max-width: 50rem;
         margin: 2rem auto; padding: 0 1rem; color: #1a1a1a; }}
  h1 {{ font-size: 1.3rem; }}
  p.sub {{ color: #666; }}
  ul {{ list-style: none; padding: 0; }}
  li {{ padding: .4rem 0; border-bottom: 1px solid #eee; }}
  a {{ color: #0b62d6; text-decoration: none; }}
  a:hover {{ text-decoration: underline; }}
  .host {{ color: #888; font-size: .85em; }}
  .meta {{ color: #aaa; font-size: .8em; float: right; }}
</style>
</head>
<body>
<h1>Advertiser links</h1>
<p class=\"sub\">{count} distinct destinations captured by kickbacks-kit. Read-only: kb recorded these from the local ad file, nothing more.</p>
<ul>
{rows}</ul>
</body>
</html>
",
        count = links.len(),
        rows = rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(adv: &str, url: &str, n: i64, last: i64) -> LinkRow {
        LinkRow {
            advertiser: adv.to_string(),
            url: url.to_string(),
            times_seen: n,
            last_seen_ms: last,
        }
    }

    fn sample() -> Vec<LinkRow> {
        vec![
            link("Tailscale", "https://tailscale.com/", 8, 1000),
            link("Ramp, Inc", "https://ramp.com/?ref=a&b=c", 3, 900),
        ]
    }

    #[test]
    fn csv_quotes_commas_and_keeps_columns() {
        let csv = render_csv(&sample());
        assert!(csv.starts_with("advertiser,url,times_seen,last_seen_ms\n"));
        // The advertiser with a comma is quoted.
        assert!(csv.contains("\"Ramp, Inc\""));
        assert_eq!(csv.lines().count(), 3);
    }

    #[test]
    fn jsonl_has_one_object_per_link_with_host() {
        let out = render_jsonl(&sample()).unwrap();
        assert_eq!(out.lines().count(), 2);
        assert!(out.contains("\"host\":\"tailscale.com\""));
        // serde escapes the ampersand-bearing URL safely.
        assert!(out.contains("ramp.com/?ref=a&b=c"));
    }

    #[test]
    fn html_escapes_markup_and_links_urls() {
        let evil = vec![link("<script>x", "https://e.com/?a=1&b=2", 1, 10)];
        let html = render_html(&evil, 100);
        assert!(html.contains("&lt;script&gt;x"));
        assert!(!html.contains("<script>x"));
        // The & in the URL is escaped to &amp; in the href.
        assert!(html.contains("a=1&amp;b=2"));
    }

    #[test]
    fn table_is_aligned_and_handles_empty() {
        let table = render_table(&sample(), 2000);
        assert!(table.contains("advertiser"));
        assert!(table.contains("tailscale.com"));
        assert!(render_table(&[], 0).contains("no advertiser links"));
    }

    #[test]
    fn table_strips_control_chars_from_advertiser_and_url() {
        // The table prints to a terminal; an escape sequence in the captured
        // text must not survive into stdout.
        let evil = vec![link(
            "Evil\x1b]0;pwn\x07",
            "https://e.com/\x1b[31mred",
            1,
            10,
        )];
        let table = render_table(&evil, 100);
        assert!(!table.contains('\x1b'));
        assert!(!table.contains('\x07'));
    }
}
