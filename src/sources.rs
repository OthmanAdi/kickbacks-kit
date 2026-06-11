//! Readers for the kickbacks.ai extension's local artifacts.
//!
//! These functions are strictly read-only. The crate never writes to any file
//! the extension owns, and never emits a network request to any billing
//! endpoint. We observe what the extension already records; we never
//! manufacture an impression.

use anyhow::{Context, Result};
use std::fs;

use crate::model::CliAd;
use crate::paths;

/// Read the current ad from `cli-ad.json`. Returns `Ok(None)` when the file is
/// absent or empty (extension not running, or signed out), which is a normal
/// state, not an error.
pub fn read_cli_ad() -> Result<Option<CliAd>> {
    let path = paths::cli_ad_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let ad: CliAd =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(ad))
}

/// A parsed line from the extension lifecycle log (`debug.log`).
///
/// Lines look like:
/// `2026-06-11T19:42:13.745Z [ext] info - session.state {"signedIn":true,...}`
#[derive(Debug, Clone, PartialEq)]
pub struct LogEvent {
    pub ts_iso: String,
    pub ts_ms: i64,
    pub name: String,
    pub signed_in: Option<bool>,
    pub has_ad: Option<bool>,
    pub injection_on: Option<bool>,
    pub killed: Option<bool>,
    pub cc_version: Option<String>,
    pub raw: String,
}

/// Parse a single log line. Returns `None` for blank or malformed lines so the
/// caller can skip them without failing the whole read.
pub fn parse_log_line(line: &str) -> Option<LogEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    // `<ts> [src] <level> - <name> <json?>`
    let (ts_iso, rest) = line.split_once(' ')?;
    let after_dash = rest.split_once(" - ")?.1;
    let (name, json_str) = match after_dash.split_once(" {") {
        Some((n, j)) => (n.trim().to_string(), format!("{{{j}")),
        None => (after_dash.trim().to_string(), String::new()),
    };
    if name.is_empty() {
        return None;
    }

    let ts_ms = chrono::DateTime::parse_from_rfc3339(ts_iso)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0);

    let mut ev = LogEvent {
        ts_iso: ts_iso.to_string(),
        ts_ms,
        name,
        signed_in: None,
        has_ad: None,
        injection_on: None,
        killed: None,
        cc_version: None,
        raw: line.to_string(),
    };

    if !json_str.is_empty() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json_str) {
            ev.signed_in = v.get("signedIn").and_then(|x| x.as_bool());
            ev.has_ad = v.get("hasAd").and_then(|x| x.as_bool());
            ev.injection_on = v.get("injectionOn").and_then(|x| x.as_bool());
            ev.killed = v.get("killed").and_then(|x| x.as_bool());
            ev.cc_version = v
                .get("ccVersion")
                .and_then(|x| x.as_str())
                .map(str::to_string);
        }
    }

    Some(ev)
}

/// Read lifecycle events from `debug.log`, returning only lines whose ISO
/// timestamp is strictly greater than `after_iso` (lexical compare, which is
/// correct for fixed-width Zulu ISO-8601). Pass `None` to read everything.
pub fn read_events_since(after_iso: Option<&str>) -> Result<Vec<LogEvent>> {
    let path = paths::debug_log_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut out = Vec::new();
    for line in raw.lines() {
        if let Some(ev) = parse_log_line(line) {
            let keep = match after_iso {
                Some(w) => ev.ts_iso.as_str() > w,
                None => true,
            };
            if keep {
                out.push(ev);
            }
        }
    }
    Ok(out)
}

/// The most recent lifecycle state, used by the dashboard header.
#[derive(Debug, Clone, Default)]
pub struct LiveState {
    pub signed_in: Option<bool>,
    pub injection_on: Option<bool>,
    pub killed: Option<bool>,
    pub has_ad: Option<bool>,
    pub cc_version: Option<String>,
    pub last_ts_iso: Option<String>,
}

/// Fold the log into the latest known `session.state`.
pub fn read_live_state() -> Result<LiveState> {
    let events = read_events_since(None)?;
    let mut st = LiveState::default();
    for ev in events {
        if ev.name == "session.state" {
            if ev.signed_in.is_some() {
                st.signed_in = ev.signed_in;
            }
            if ev.injection_on.is_some() {
                st.injection_on = ev.injection_on;
            }
            if ev.killed.is_some() {
                st.killed = ev.killed;
            }
            if ev.has_ad.is_some() {
                st.has_ad = ev.has_ad;
            }
            if ev.cc_version.is_some() {
                st.cc_version = ev.cc_version.clone();
            }
            st.last_ts_iso = Some(ev.ts_iso.clone());
        }
    }
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_state_line() {
        let line = r#"2026-06-11T19:42:23.748Z [ext] info - session.state {"signedIn":true,"authHealthy":"ok","injectionOn":true,"killed":false,"hasAd":true,"ccVersion":"2.1.173"}"#;
        let ev = parse_log_line(line).expect("should parse");
        assert_eq!(ev.name, "session.state");
        assert_eq!(ev.signed_in, Some(true));
        assert_eq!(ev.injection_on, Some(true));
        assert_eq!(ev.killed, Some(false));
        assert_eq!(ev.has_ad, Some(true));
        assert_eq!(ev.cc_version.as_deref(), Some("2.1.173"));
        assert!(ev.ts_ms > 0);
    }

    #[test]
    fn parses_event_without_json() {
        let line = "2026-06-11T19:42:13.838Z [ext] info - boot.cycle.start {}";
        let ev = parse_log_line(line).expect("should parse");
        assert_eq!(ev.name, "boot.cycle.start");
        assert_eq!(ev.signed_in, None);
    }

    #[test]
    fn skips_blank_and_malformed() {
        assert!(parse_log_line("").is_none());
        assert!(parse_log_line("   ").is_none());
        assert!(parse_log_line("not a real line").is_none());
    }

    #[test]
    fn since_filter_is_strict() {
        let line = "2026-06-11T19:42:13.838Z [ext] info - x {}";
        let ev = parse_log_line(line).unwrap();
        assert!(ev.ts_iso.as_str() > "2026-06-11T19:00:00.000Z");
        assert!(ev.ts_iso.as_str() <= "2026-06-11T20:00:00.000Z");
    }
}
