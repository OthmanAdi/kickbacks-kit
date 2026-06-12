//! Small formatting helpers shared across commands.

use chrono::{Local, TimeZone, Utc};

/// Current wall-clock time in epoch milliseconds.
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Compact human age from a millisecond delta (`now - then`).
pub fn human_age(ms_ago: i64) -> String {
    if ms_ago < 0 {
        return "just now".to_string();
    }
    let secs = ms_ago / 1000;
    if secs < 60 {
        return format!("{secs}s ago");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    format!("{days}d ago")
}

/// Format an epoch-millis instant in the local timezone, e.g. `2026-06-11 22:34`.
pub fn fmt_datetime(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => "—".to_string(),
    }
}

/// Compact relative age for dense feed rows: `now`, `5m`, `3h`, `2d`. A
/// negative delta (a future timestamp, e.g. clock skew between us and a remote
/// source) clamps to `now` rather than showing a nonsense value.
pub fn human_age_short(ms_ago: i64) -> String {
    if ms_ago < 0 {
        return "now".to_string();
    }
    let secs = ms_ago / 1000;
    if secs < 60 {
        return "now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d");
    }
    format!("{}w", days / 7)
}

/// Strip control characters from text that came from outside this process (an
/// ad creative, a remote feed item) and collapse internal whitespace runs to a
/// single space. Removes C0, DEL, and C1 controls so external text can never
/// emit its own terminal escapes, newlines, or tabs that would break a single
/// rendered line. This is the same injection-safety guarantee the status line
/// applies to ad text, shared so every external-text surface uses one rule.
pub fn sanitize_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            // Real whitespace (including embedded newlines from multi-line
            // remote text) collapses to a single space.
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        if c.is_control() || ('\u{80}'..='\u{9f}').contains(&c) {
            // Other controls (ESC, BEL, DEL, C1) are removed outright, with no
            // substitute character, so a word is not split by a stray escape.
            continue;
        }
        out.push(c);
        prev_space = false;
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Truncate a string to `max` display columns, appending `…` when cut.
/// Counts `char`s (good enough for the Latin + punctuation creatives here).
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_buckets() {
        assert_eq!(human_age(5_000), "5s ago");
        assert_eq!(human_age(90_000), "1m ago");
        assert_eq!(human_age(3 * 3_600_000), "3h ago");
        assert_eq!(human_age(2 * 86_400_000), "2d ago");
        assert_eq!(human_age(-1), "just now");
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn short_age_buckets() {
        assert_eq!(human_age_short(5_000), "now");
        assert_eq!(human_age_short(90_000), "1m");
        assert_eq!(human_age_short(3 * 3_600_000), "3h");
        assert_eq!(human_age_short(2 * 86_400_000), "2d");
        assert_eq!(human_age_short(10 * 86_400_000), "1w");
        assert_eq!(human_age_short(-1), "now");
    }

    #[test]
    fn sanitize_strips_controls_and_collapses_space() {
        assert_eq!(sanitize_text("hello   world"), "hello world");
        assert_eq!(sanitize_text("a\tb\nc"), "a b c");
        assert_eq!(sanitize_text("  trim me  "), "trim me");
        // An OSC injection attempt in external text is defused: the ESC and
        // BEL are removed without splitting the surrounding word.
        assert_eq!(sanitize_text("evil\x1b]0;pwned\x07end"), "evil]0;pwnedend");
        // C1 control (0x9b, CSI) is removed outright.
        assert_eq!(sanitize_text("x\u{9b}y"), "xy");
    }
}
