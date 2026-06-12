//! The fetch orchestrator: it decides whether the network may be used, walks
//! each public source behind a per-source circuit breaker, parses fresh bodies
//! into feed items, and persists everything to the archive cache. Read surfaces
//! never call the network; they call [`read_cached`].
//!
//! Stale-while-revalidate is the rule: a failed or skipped source keeps its last
//! good cached items, so the feed always shows the best data kb has, and a
//! transient outage degrades to "last synced 12m ago" rather than an empty pane.

use crate::archive::Archive;
use crate::config::Config;
use crate::feed::http::{FetchClient, FetchError, Fetched};
use crate::feed::{bulletin, github, FeedItem, FeedSnapshot, SourceHealth};
use crate::util;

/// Open the circuit after this many consecutive failures.
const FAILURE_THRESHOLD: u32 = 3;
/// Base cooldown once the circuit opens; grows with continued failure.
const COOLDOWN_BASE_MS: i64 = 5 * 60 * 1000;
/// Cooldown never exceeds this, so a long outage still retries periodically.
const COOLDOWN_MAX_MS: i64 = 30 * 60 * 1000;
/// Back off briefly when GitHub's remaining request budget runs low.
const RATE_FLOOR: u32 = 8;
/// A rate-limit cooldown is capped here (just over GitHub's hourly reset), so a
/// malformed or hostile `x-ratelimit-reset` cannot park a source indefinitely.
const RATE_LIMIT_MAX_MS: i64 = 65 * 60 * 1000;

/// How many feed items to keep / show.
pub const FEED_LIMIT: usize = 40;

/// Reasons the network is unavailable for this process, for an honest message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflineReason {
    /// `--offline` flag, or a non-fetching surface.
    Flag,
    /// `feed.enabled = false` in the config.
    Config,
    /// `KICKBACKS_KIT_OFFLINE` is set in the environment.
    Env,
}

/// Decide whether the feed may use the network. `cli_offline` is the per-command
/// `--offline` flag. The environment kill switch wins over everything.
pub fn offline_reason(cli_offline: bool, cfg: &Config) -> Option<OfflineReason> {
    if std::env::var_os("KICKBACKS_KIT_OFFLINE").is_some() {
        return Some(OfflineReason::Env);
    }
    if cli_offline {
        return Some(OfflineReason::Flag);
    }
    if !cfg.feed.enabled {
        return Some(OfflineReason::Config);
    }
    None
}

/// The User-Agent kb sends. GitHub requires a non-empty one and asks tools to
/// identify themselves; this is honest about who is calling.
fn user_agent() -> String {
    format!(
        "kickbacks-kit/{} (+https://github.com/{})",
        env!("CARGO_PKG_VERSION"),
        github::SELF_REPO
    )
}

/// Read the feed entirely from cache (no network). Used by the status line,
/// snapshots, and `kb feed --offline`.
pub fn read_cached(archive: &Archive, offline: bool) -> FeedSnapshot {
    let mut items = archive.read_feed_items(FEED_LIMIT).unwrap_or_default();
    // Ensure the static rows are present even on a cold cache.
    merge_static(&mut items);
    let sources = archive.all_feed_sources().unwrap_or_default();
    let last_sync_ms = sources.iter().filter_map(|s| s.last_sync_ms).max();
    FeedSnapshot {
        items,
        sources,
        last_sync_ms,
        offline,
    }
}

/// Run one fetch cycle across all sources, then return the refreshed cache.
/// Honors the offline decision; on offline it just returns the cache.
pub fn sync(archive: &mut Archive, cfg: &Config, cli_offline: bool) -> FeedSnapshot {
    if let Some(_reason) = offline_reason(cli_offline, cfg) {
        return read_cached(archive, true);
    }
    let client = FetchClient::new(user_agent());
    let now = util::now_ms();

    // The bulletin: the official status channel, one item.
    fetch_into(
        archive,
        &client,
        "bulletin",
        bulletin::BULLETIN_URL,
        now,
        |body| {
            bulletin::parse(body)
                .map(|b| vec![bulletin::to_item(&b)])
                .unwrap_or_default()
        },
    );

    // Upstream repo stats. Each builder stamps the item's source with the same
    // key the cache deletes by, so a refresh replaces rather than duplicates.
    fetch_into(
        archive,
        &client,
        "github_repo",
        &github::repo_url(github::UPSTREAM_REPO),
        now,
        |body| {
            github::parse_repo(body)
                .map(|s| vec![github::upstream_stat_item(&s, "github_repo")])
                .unwrap_or_default()
        },
    );

    // Upstream version (from the commit log; no releases/tags exist).
    fetch_into(
        archive,
        &client,
        "github_version",
        &github::commits_url(github::UPSTREAM_REPO),
        now,
        |body| {
            github::parse_latest_version(body)
                .map(|v| vec![github::version_item(&v, "github_version")])
                .unwrap_or_default()
        },
    );

    // Upstream issues.
    fetch_into(
        archive,
        &client,
        "github_issues",
        &github::issues_url(github::UPSTREAM_REPO),
        now,
        |body| github::parse_issues(body, 8, "github_issues"),
    );

    read_cached(archive, false)
}

/// Fetch one source through its circuit breaker, parse on a fresh body, and
/// persist both the items and the updated health. `parse` turns a body into the
/// items for this source.
fn fetch_into(
    archive: &mut Archive,
    client: &FetchClient,
    source: &str,
    url: &str,
    now: i64,
    parse: impl Fn(&str) -> Vec<FeedItem>,
) {
    let mut health = archive
        .feed_source(source)
        .unwrap_or_else(|_| SourceHealth {
            source: source.to_string(),
            ..Default::default()
        });

    // Circuit open: skip the call entirely, keep cached items.
    if let Some(until) = health.circuit_until_ms {
        if now < until {
            let mins = ((until - now) / 60_000).max(1);
            health.last_status = format!("cooling down ({mins}m)");
            let _ = archive.set_feed_source(&health);
            return;
        }
    }

    match client.get_json(url, health.etag.as_deref()) {
        Ok(Fetched::NotModified) => {
            health.failures = 0;
            health.circuit_until_ms = None;
            health.last_status = "up to date".to_string();
            health.last_sync_ms = Some(now);
            let _ = archive.set_feed_source(&health);
        }
        Ok(Fetched::Body(body)) => {
            let items = parse(&body.text);
            let _ = archive.replace_feed_items(source, &items, now);
            health.failures = 0;
            health.last_sync_ms = Some(now);
            if let Some(tag) = body.etag {
                health.etag = Some(tag);
            }
            // Respect a low GitHub budget by parking the source until reset.
            if matches!(body.rate_remaining, Some(r) if r < RATE_FLOOR) {
                health.circuit_until_ms = Some(rate_limit_until(body.rate_reset_ms, now));
                health.last_status = "rate limited".to_string();
            } else {
                health.circuit_until_ms = None;
                health.last_status = "ok".to_string();
            }
            let _ = archive.set_feed_source(&health);
        }
        Err(err) => {
            health.failures = health.failures.saturating_add(1);
            health.last_status = format!("error: {}", short_err(&err));
            // Open the circuit only after the threshold; below it, clear any
            // stale cooldown so a past-expiry value never lingers in the record.
            health.circuit_until_ms = if health.failures >= FAILURE_THRESHOLD {
                Some(now + cooldown_for(health.failures, now))
            } else {
                None
            };
            // On error the cached items are left in place (stale-while-revalidate).
            let _ = archive.set_feed_source(&health);
        }
    }
}

/// When to retry a rate-limited source. Trusts the server's reset time, but
/// clamps it into a sane window: at least `COOLDOWN_BASE_MS` out (so an absent
/// or already-past reset still parks the source, instead of leaving the circuit
/// closed and hammering a near-empty budget), and at most `RATE_LIMIT_MAX_MS`
/// (so a hostile or broken `x-ratelimit-reset` cannot park a source forever).
fn rate_limit_until(reset_ms: Option<i64>, now: i64) -> i64 {
    let floor = now + COOLDOWN_BASE_MS;
    let ceil = now + RATE_LIMIT_MAX_MS;
    reset_ms.unwrap_or(floor).clamp(floor, ceil)
}

/// Cooldown that grows with the failure streak, jittered so many installs do
/// not retry in lockstep, capped so a long outage still retries.
fn cooldown_for(failures: u32, now: i64) -> i64 {
    let steps = failures.saturating_sub(FAILURE_THRESHOLD);
    let base = COOLDOWN_BASE_MS.saturating_mul(1_i64 << steps.min(3));
    let capped = base.min(COOLDOWN_MAX_MS);
    // Jitter up to ~30s, derived from the clock (no rng dependency).
    let jitter = (now.unsigned_abs() % 30_000) as i64;
    capped + jitter
}

/// A compact, single-line form of a fetch error for the status field.
fn short_err(err: &FetchError) -> String {
    let s = err.to_string();
    util::truncate(&s.replace(['\n', '\r'], " "), 40)
}

/// Insert the static rows if the cache does not already carry them (it will not
/// on a cold start, since nothing has been fetched yet).
fn merge_static(items: &mut Vec<FeedItem>) {
    for s in crate::feed::static_items() {
        if !items.iter().any(|i| i.id == s.id) {
            items.push(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, FeedConfig};

    fn cfg(enabled: bool) -> Config {
        Config {
            feed: FeedConfig { enabled },
            ..Default::default()
        }
    }

    #[test]
    fn offline_flag_and_config_and_env() {
        // Env var dominates; guard the process-global with care.
        assert_eq!(offline_reason(false, &cfg(true)), None);
        assert_eq!(offline_reason(true, &cfg(true)), Some(OfflineReason::Flag));
        assert_eq!(
            offline_reason(false, &cfg(false)),
            Some(OfflineReason::Config)
        );
    }

    #[test]
    fn cold_cache_still_has_static_items() {
        let archive = Archive::open_in_memory().unwrap();
        let snap = read_cached(&archive, true);
        assert!(snap.offline);
        assert!(snap
            .items
            .iter()
            .any(|i| i.title.contains("@andrewmccalip")));
    }

    #[test]
    fn cooldown_grows_and_caps() {
        // Failures just over threshold give the base; far over gives the cap.
        let a = cooldown_for(FAILURE_THRESHOLD + 1, 0);
        let b = cooldown_for(FAILURE_THRESHOLD + 9, 0);
        assert!(a >= COOLDOWN_BASE_MS);
        assert!(b <= COOLDOWN_MAX_MS + 30_000);
        assert!(b >= a);
    }

    #[test]
    fn replace_items_is_per_source() {
        let mut archive = Archive::open_in_memory().unwrap();
        let a = FeedItem::new(
            crate::feed::FeedKind::Issue,
            "from A",
            "",
            Some("https://a/1".into()),
            Some(1),
            "src_a",
        );
        let b = FeedItem::new(
            crate::feed::FeedKind::Issue,
            "from B",
            "",
            Some("https://b/1".into()),
            Some(2),
            "src_b",
        );
        archive.replace_feed_items("src_a", &[a], 10).unwrap();
        archive.replace_feed_items("src_b", &[b], 10).unwrap();
        // Re-fetching src_a with nothing clears only src_a.
        archive.replace_feed_items("src_a", &[], 20).unwrap();
        let items = archive.read_feed_items(40).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, "src_b");
    }

    #[test]
    fn github_items_use_the_orchestrator_source_so_they_purge() {
        // Regression: the builders once hardcoded source "github" while the
        // orchestrator deletes by "github_issues", so a closed issue would
        // never be purged and showed as open forever. The item's source must
        // equal the key replace_feed_items deletes by.
        let mut archive = Archive::open_in_memory().unwrap();
        let issues_v1 = r#"[
            {"number":1,"title":"open A","created_at":"2026-06-12T10:00:00Z"},
            {"number":2,"title":"open B","created_at":"2026-06-12T09:00:00Z"}
        ]"#;
        let items = github::parse_issues(issues_v1, 8, "github_issues");
        archive
            .replace_feed_items("github_issues", &items, 1000)
            .unwrap();
        assert_eq!(archive.read_feed_items(40).unwrap().len(), 2);

        // Issue #2 closed: the next fetch returns only #1. The cache for this
        // source is replaced, so #2 is gone (not shown as open forever).
        let issues_v2 = r#"[{"number":1,"title":"open A","created_at":"2026-06-12T10:00:00Z"}]"#;
        let items = github::parse_issues(issues_v2, 8, "github_issues");
        archive
            .replace_feed_items("github_issues", &items, 2000)
            .unwrap();
        let remaining = archive.read_feed_items(40).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].title.starts_with("#1"));
    }

    #[test]
    fn rate_limit_until_clamps_into_a_sane_window() {
        let now = 1_000_000_000_000;
        // Absent reset header: still parks the source (was a bug: left it open).
        assert_eq!(rate_limit_until(None, now), now + COOLDOWN_BASE_MS);
        // A hostile/huge reset cannot park the source forever.
        assert_eq!(
            rate_limit_until(Some(i64::MAX), now),
            now + RATE_LIMIT_MAX_MS
        );
        // An already-past reset is floored to the base cooldown.
        assert_eq!(
            rate_limit_until(Some(now - 5000), now),
            now + COOLDOWN_BASE_MS
        );
        // A legitimate near reset is respected.
        let soon = now + 10 * 60 * 1000;
        assert_eq!(rate_limit_until(Some(soon), now), soon);
    }

    #[test]
    fn sync_offline_returns_cache_without_network() {
        let mut archive = Archive::open_in_memory().unwrap();
        // feed.enabled = false forces offline, so no network is attempted.
        let snap = sync(&mut archive, &cfg(false), false);
        assert!(snap.offline);
        // Only the static row is present (no fetch happened).
        assert!(snap.items.iter().all(|i| i.source == "static"));
    }
}
