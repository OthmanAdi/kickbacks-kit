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

/// Why ads are or are not showing, derived purely from local signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdStatus {
    /// Not signed in, so nothing accrues.
    SignedOut,
    /// kickbacks.ai killswitch is active (server-side pause).
    Paused,
    /// Ad injection is switched off locally.
    InjectionOff,
    /// An ad is being served right now.
    Live,
    /// Signed in and enabled, but no active session is rendering an ad.
    Idle,
}

impl AdStatus {
    /// Short human label.
    pub fn label(self) -> &'static str {
        match self {
            AdStatus::SignedOut => "SIGNED OUT",
            AdStatus::Paused => "PAUSED (kickbacks killswitch active)",
            AdStatus::InjectionOff => "OFF (ad injection disabled)",
            AdStatus::Live => "LIVE (ad showing now)",
            AdStatus::Idle => "IDLE (no active Claude Code session)",
        }
    }

    /// True when ads are actually flowing.
    pub fn is_live(self) -> bool {
        matches!(self, AdStatus::Live)
    }
}

/// Decide the ad status from the latest lifecycle state and whether the current
/// ad file is fresh. Pure, so it is unit tested. `killed` takes priority over
/// everything except being signed out, because that is the surprising case.
pub fn ad_status(state: &LiveState, ad_fresh: bool) -> AdStatus {
    if !state.signed_in.unwrap_or(false) {
        return AdStatus::SignedOut;
    }
    if state.killed.unwrap_or(false) {
        return AdStatus::Paused;
    }
    if !state.injection_on.unwrap_or(false) {
        return AdStatus::InjectionOff;
    }
    if ad_fresh {
        AdStatus::Live
    } else {
        AdStatus::Idle
    }
}

/// Best-effort installed extension version, read from the VS Code extensions
/// directory name (`kickbacksai.kickbacks-ai-<version>`). Returns the highest
/// version found, or `None` if the extension is not installed there.
pub fn installed_extension_version() -> Option<String> {
    let home = dirs::home_dir()?;
    let dir = home.join(".vscode").join("extensions");
    let entries = std::fs::read_dir(dir).ok()?;
    let mut versions: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix("kickbacksai.kickbacks-ai-")
                .map(str::to_string)
        })
        .collect();
    versions.sort();
    versions.pop()
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

    fn state(signed: bool, injection: bool, killed: bool) -> LiveState {
        LiveState {
            signed_in: Some(signed),
            injection_on: Some(injection),
            killed: Some(killed),
            ..Default::default()
        }
    }

    #[test]
    fn ad_status_killswitch_takes_priority() {
        // Signed in, injection on, fresh ad, but killed -> Paused.
        assert_eq!(ad_status(&state(true, true, true), true), AdStatus::Paused);
    }

    #[test]
    fn ad_status_live_and_idle() {
        assert_eq!(ad_status(&state(true, true, false), true), AdStatus::Live);
        assert_eq!(ad_status(&state(true, true, false), false), AdStatus::Idle);
    }

    #[test]
    fn ad_status_signed_out_and_injection_off() {
        assert_eq!(
            ad_status(&state(false, true, false), true),
            AdStatus::SignedOut
        );
        assert_eq!(
            ad_status(&state(true, false, false), true),
            AdStatus::InjectionOff
        );
    }
}
