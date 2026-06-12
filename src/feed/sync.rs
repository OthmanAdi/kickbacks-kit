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

    // Upstream repo stats.
    fetch_into(
        archive,
        &client,
        "github_repo",
        &github::repo_url(github::UPSTREAM_REPO),
        now,
        |body| {
            github::parse_repo(body)
                .map(|s| vec![github::upstream_stat_item(&s)])
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
                .map(|v| vec![github::version_item(&v)])
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
        |body| github::parse_issues(body, 8),
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
                health.circuit_until_ms = body.rate_reset_ms.map(|r| r.max(now + COOLDOWN_BASE_MS));
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
            if health.failures >= FAILURE_THRESHOLD {
                health.circuit_until_ms = Some(now + cooldown_for(health.failures, now));
            }
            // On error the cached items are left in place (stale-while-revalidate).
            let _ = archive.set_feed_source(&health);
        }
    }
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
    fn sync_offline_returns_cache_without_network() {
        let mut archive = Archive::open_in_memory().unwrap();
        // feed.enabled = false forces offline, so no network is attempted.
        let snap = sync(&mut archive, &cfg(false), false);
        assert!(snap.offline);
        // Only the static row is present (no fetch happened).
        assert!(snap.items.iter().all(|i| i.source == "static"));
    }
}
