//! The local ad corpus: a SQLite database of every ad creative we have
//! observed, every rotation sighting, and the extension lifecycle events.
//!
//! The archive is append-only and observational. It records what was shown to
//! this machine; it never fabricates a sighting and never reports anything to
//! the kickbacks.ai backend.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::model::AdRow;
use crate::model::CliAd;
use crate::sources::LogEvent;

/// One clock hour in epoch milliseconds.
const HOUR_MS: i64 = 60 * 60 * 1000;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS ads (
    id            TEXT PRIMARY KEY,
    advertiser    TEXT NOT NULL,
    ad_text       TEXT NOT NULL,
    click_url     TEXT,
    icon_url      TEXT,
    first_seen_ms INTEGER NOT NULL,
    last_seen_ms  INTEGER NOT NULL,
    times_seen    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS sightings (
    ad_id       TEXT NOT NULL,
    seen_ms     INTEGER NOT NULL,   -- rotation ts from cli-ad.json
    observed_ms INTEGER NOT NULL,   -- our wall-clock when we polled
    PRIMARY KEY (ad_id, seen_ms)
);
CREATE INDEX IF NOT EXISTS idx_sightings_observed ON sightings(observed_ms);

CREATE TABLE IF NOT EXISTS events (
    ts_iso       TEXT NOT NULL,
    ts_ms        INTEGER NOT NULL,
    name         TEXT NOT NULL,
    signed_in    INTEGER,
    has_ad       INTEGER,
    injection_on INTEGER,
    killed       INTEGER,
    cc_version   TEXT,
    raw          TEXT,
    PRIMARY KEY (ts_iso, name, raw)
);

CREATE TABLE IF NOT EXISTS meta (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);

-- Hours (absolute clock-hour start, epoch ms) in which a capture pass ran.
-- Lets the dashboard distinguish "0 ads" from "kb was not watching".
CREATE TABLE IF NOT EXISTS coverage (
    hour_ms INTEGER PRIMARY KEY
);
"#;

/// Summary statistics rendered by `kb archive stats` and the dashboard.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub distinct_ads: i64,
    pub advertisers: i64,
    pub total_sightings: i64,
    pub sightings_today: i64,
    pub sightings_week: i64,
    pub first_seen_ms: Option<i64>,
    pub last_seen_ms: Option<i64>,
}

/// One advertiser leaderboard entry.
#[derive(Debug, Clone)]
pub struct AdvertiserStat {
    pub advertiser: String,
    pub distinct_ads: i64,
    pub sightings: i64,
}

/// Result of a single capture pass.
#[derive(Debug, Clone, Default)]
pub struct CaptureReport {
    /// A new rotation of an ad was recorded (first time we saw this `seen_ms`).
    pub new_sighting: bool,
    /// The advertiser involved, if any ad was present.
    pub advertiser: Option<String>,
    /// Number of new lifecycle events ingested from `debug.log`.
    pub new_events: usize,
}

/// Handle to the archive database.
pub struct Archive {
    conn: Connection,
}

impl Archive {
    /// Open (creating if needed) the archive at `path` and run migrations.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening archive db: {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON").ok();
        conn.execute_batch(SCHEMA).context("running migrations")?;
        Ok(Self { conn })
    }

    /// Open an in-memory archive (used by tests and the demo data path).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    // ---- writes -----------------------------------------------------------

    /// Record an observed ad. A sighting is only recorded the first time we see
    /// a given (ad, rotation-timestamp) pair, so polling fast never double
    /// counts. Returns `true` when a new rotation was recorded.
    pub fn capture_ad(&mut self, ad: &CliAd, observed_ms: i64) -> Result<bool> {
        let id = ad.id();
        let advertiser = ad.advertiser();
        let tx = self.conn.transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO sightings (ad_id, seen_ms, observed_ms) VALUES (?1, ?2, ?3)",
            params![id, ad.ts, observed_ms],
        )?;
        if inserted > 0 {
            tx.execute(
                "INSERT INTO ads (id, advertiser, ad_text, click_url, icon_url, first_seen_ms, last_seen_ms, times_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 1)
                 ON CONFLICT(id) DO UPDATE SET
                     last_seen_ms = excluded.last_seen_ms,
                     times_seen   = times_seen + 1,
                     advertiser   = excluded.advertiser,
                     ad_text      = excluded.ad_text,
                     click_url    = excluded.click_url,
                     icon_url     = excluded.icon_url",
                params![id, advertiser, ad.ad_text, ad.click_url, ad.icon_url, ad.ts],
            )?;
        }
        tx.commit()?;
        Ok(inserted > 0)
    }

    /// Ingest a batch of lifecycle events. Duplicate rows are ignored. Returns
    /// the count actually inserted.
    pub fn record_events(&mut self, events: &[LogEvent]) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        let mut inserted = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO events
                 (ts_iso, ts_ms, name, signed_in, has_ad, injection_on, killed, cc_version, raw)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for ev in events {
                inserted += stmt.execute(params![
                    ev.ts_iso,
                    ev.ts_ms,
                    ev.name,
                    ev.signed_in,
                    ev.has_ad,
                    ev.injection_on,
                    ev.killed,
                    ev.cc_version,
                    ev.raw,
                ])?;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Record that a capture pass observed the local artifacts at `now_ms`.
    /// One row per absolute clock hour; re-recording the same hour is a no-op.
    pub fn record_observation(&mut self, now_ms: i64) -> Result<()> {
        let hour = now_ms / HOUR_MS * HOUR_MS;
        self.conn.execute(
            "INSERT OR IGNORE INTO coverage (hour_ms) VALUES (?1)",
            params![hour],
        )?;
        Ok(())
    }

    /// Read a string value from the `meta` table.
    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        let v = self
            .conn
            .query_row("SELECT v FROM meta WHERE k = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .ok();
        Ok(v)
    }

    /// Write a string value to the `meta` table.
    pub fn meta_set(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- reads ------------------------------------------------------------

    /// Compute summary statistics relative to `now_ms`.
    pub fn stats(&self, now_ms: i64) -> Result<Stats> {
        let day_ago = now_ms - 24 * 60 * 60 * 1000;
        let week_ago = now_ms - 7 * 24 * 60 * 60 * 1000;

        let distinct_ads = self
            .conn
            .query_row("SELECT COUNT(*) FROM ads", [], |r| r.get(0))?;
        let advertisers =
            self.conn
                .query_row("SELECT COUNT(DISTINCT advertiser) FROM ads", [], |r| {
                    r.get(0)
                })?;
        let total_sightings = self
            .conn
            .query_row("SELECT COUNT(*) FROM sightings", [], |r| r.get(0))?;
        let sightings_today = self.conn.query_row(
            "SELECT COUNT(*) FROM sightings WHERE observed_ms >= ?1",
            params![day_ago],
            |r| r.get(0),
        )?;
        let sightings_week = self.conn.query_row(
            "SELECT COUNT(*) FROM sightings WHERE observed_ms >= ?1",
            params![week_ago],
            |r| r.get(0),
        )?;
        let first_seen_ms = self
            .conn
            .query_row("SELECT MIN(first_seen_ms) FROM ads", [], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .unwrap_or(None);
        let last_seen_ms = self
            .conn
            .query_row("SELECT MAX(last_seen_ms) FROM ads", [], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .unwrap_or(None);

        Ok(Stats {
            distinct_ads,
            advertisers,
            total_sightings,
            sightings_today,
            sightings_week,
            first_seen_ms,
            last_seen_ms,
        })
    }

    /// List captured ads, most recently seen first.
    pub fn list_ads(&self, limit: usize) -> Result<Vec<AdRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, advertiser, ad_text, click_url, first_seen_ms, last_seen_ms, times_seen
             FROM ads ORDER BY last_seen_ms DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], Self::map_ad_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every captured ad, ordered by first sighting (used by exports).
    pub fn all_ads(&self) -> Result<Vec<AdRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, advertiser, ad_text, click_url, first_seen_ms, last_seen_ms, times_seen
             FROM ads ORDER BY first_seen_ms ASC",
        )?;
        let rows = stmt
            .query_map([], Self::map_ad_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Advertiser leaderboard by total sightings, then distinct creatives.
    pub fn advertiser_leaderboard(&self, limit: usize) -> Result<Vec<AdvertiserStat>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.advertiser,
                    COUNT(DISTINCT a.id) AS distinct_ads,
                    COALESCE(SUM(a.times_seen), 0) AS sightings
             FROM ads a
             GROUP BY a.advertiser
             ORDER BY sightings DESC, distinct_ads DESC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(AdvertiserStat {
                    advertiser: r.get(0)?,
                    distinct_ads: r.get(1)?,
                    sightings: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Activity for the last `hours` clock hours, oldest first, ending with
    /// the hour containing `now_ms`. `Some(n)` means kb was observing during
    /// that hour and recorded `n` sightings; `None` means kb was not watching,
    /// so the hour holds no data (which is not the same as zero ads).
    pub fn hourly_activity(&self, now_ms: i64, hours: usize) -> Result<Vec<Option<u64>>> {
        let end_hour = now_ms / HOUR_MS * HOUR_MS;
        let start = end_hour - (hours as i64 - 1) * HOUR_MS;
        let mut buckets: Vec<Option<u64>> = vec![None; hours];

        let mut stmt = self
            .conn
            .prepare("SELECT hour_ms FROM coverage WHERE hour_ms >= ?1")?;
        let observed = stmt
            .query_map(params![start], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for h in observed {
            let idx = ((h - start) / HOUR_MS) as usize;
            if idx < hours {
                buckets[idx] = Some(0);
            }
        }

        let mut stmt = self
            .conn
            .prepare("SELECT observed_ms FROM sightings WHERE observed_ms >= ?1")?;
        let times = stmt
            .query_map(params![start], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for t in times {
            let idx = ((t - start) / HOUR_MS) as usize;
            if idx < hours {
                // A recorded sighting proves we were watching that hour, even
                // for archives that predate the coverage table.
                buckets[idx] = Some(buckets[idx].unwrap_or(0) + 1);
            }
        }
        Ok(buckets)
    }

    fn map_ad_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AdRow> {
        Ok(AdRow {
            id: r.get(0)?,
            advertiser: r.get(1)?,
            ad_text: r.get(2)?,
            click_url: r.get(3)?,
            first_seen_ms: r.get(4)?,
            last_seen_ms: r.get(5)?,
            times_seen: r.get(6)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(text: &str, url: &str, ts: i64) -> CliAd {
        CliAd {
            ad_text: text.to_string(),
            click_url: Some(url.to_string()),
            icon_url: None,
            icon_ref: None,
            ts,
        }
    }

    #[test]
    fn capture_dedupes_same_rotation() {
        let mut a = Archive::open_in_memory().unwrap();
        let one = ad(
            "Tailscale · the VPN that disappears",
            "https://tailscale.com/",
            1000,
        );
        // Same rotation polled three times -> one sighting.
        assert!(a.capture_ad(&one, 10).unwrap());
        assert!(!a.capture_ad(&one, 11).unwrap());
        assert!(!a.capture_ad(&one, 12).unwrap());
        let s = a.stats(2000).unwrap();
        assert_eq!(s.distinct_ads, 1);
        assert_eq!(s.total_sightings, 1);
    }

    #[test]
    fn new_rotation_increments_times_seen() {
        let mut a = Archive::open_in_memory().unwrap();
        a.capture_ad(&ad("Tailscale · x", "https://tailscale.com/", 1000), 10)
            .unwrap();
        a.capture_ad(&ad("Tailscale · x", "https://tailscale.com/", 2000), 20)
            .unwrap();
        let rows = a.list_ads(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].times_seen, 2);
        assert_eq!(rows[0].first_seen_ms, 1000);
        assert_eq!(rows[0].last_seen_ms, 2000);
    }

    #[test]
    fn distinct_advertisers_and_leaderboard() {
        let mut a = Archive::open_in_memory().unwrap();
        a.capture_ad(&ad("Tailscale · a", "https://tailscale.com/", 1), 1)
            .unwrap();
        a.capture_ad(&ad("Tailscale · a", "https://tailscale.com/", 2), 2)
            .unwrap();
        a.capture_ad(&ad("Linear · b", "https://linear.app/", 3), 3)
            .unwrap();
        let s = a.stats(100).unwrap();
        assert_eq!(s.distinct_ads, 2);
        assert_eq!(s.advertisers, 2);
        let board = a.advertiser_leaderboard(10).unwrap();
        assert_eq!(board[0].advertiser, "Tailscale");
        assert_eq!(board[0].sightings, 2);
    }

    #[test]
    fn meta_roundtrip() {
        let mut a = Archive::open_in_memory().unwrap();
        assert_eq!(a.meta_get("wm").unwrap(), None);
        a.meta_set("wm", "2026-06-11T20:00:00.000Z").unwrap();
        assert_eq!(
            a.meta_get("wm").unwrap().as_deref(),
            Some("2026-06-11T20:00:00.000Z")
        );
    }

    #[test]
    fn hourly_activity_buckets_recent_sightings() {
        let mut a = Archive::open_in_memory().unwrap();
        let now = 100 * HOUR_MS + 30 * 60 * 1000; // 100h30m
        a.capture_ad(&ad("X · 1", "https://x.com/", 1), now - 30 * 60 * 1000)
            .unwrap(); // hour 100
        a.capture_ad(&ad("X · 2", "https://x.com/", 2), now - 90 * 60 * 1000)
            .unwrap(); // hour 99
        let buckets = a.hourly_activity(now, 3).unwrap();
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets, vec![None, Some(1), Some(1)]);
    }

    #[test]
    fn hourly_activity_distinguishes_unobserved_from_zero() {
        let mut a = Archive::open_in_memory().unwrap();
        let now = 100 * HOUR_MS + 30 * 60 * 1000;
        // Hour 99: a capture pass ran but saw no ads -> Some(0), not None.
        a.record_observation(99 * HOUR_MS + 10).unwrap();
        let buckets = a.hourly_activity(now, 3).unwrap();
        assert_eq!(buckets, vec![None, Some(0), None]);
    }

    #[test]
    fn record_observation_is_idempotent_per_hour() {
        let mut a = Archive::open_in_memory().unwrap();
        a.record_observation(99 * HOUR_MS + 10).unwrap();
        a.record_observation(99 * HOUR_MS + 20_000).unwrap();
        let n: i64 = a
            .conn
            .query_row("SELECT COUNT(*) FROM coverage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
