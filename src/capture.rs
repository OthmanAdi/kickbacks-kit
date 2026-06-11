//! The single capture pass shared by `kb watch` and `kb top`.
//!
//! One pass = read the current ad and any new lifecycle events from the
//! extension's local files, then fold them into the archive. It is idempotent:
//! running it on a tight loop never double counts, because de-duplication lives
//! in the archive (rotation timestamp for ads, a watermark for events).

use anyhow::Result;
use chrono::Utc;

use crate::archive::{Archive, CaptureReport};
use crate::sources;

/// `meta` key holding the ISO timestamp of the last ingested log line.
const WATERMARK_KEY: &str = "debug_log_watermark";

/// Perform one capture pass against the open archive.
pub fn capture_pass(archive: &mut Archive) -> Result<CaptureReport> {
    let now_ms = Utc::now().timestamp_millis();
    let mut report = CaptureReport::default();

    if let Some(ad) = sources::read_cli_ad()? {
        report.advertiser = Some(ad.advertiser());
        report.new_sighting = archive.capture_ad(&ad, now_ms)?;
    }

    let watermark = archive.meta_get(WATERMARK_KEY)?;
    let events = sources::read_events_since(watermark.as_deref())?;
    if !events.is_empty() {
        let newest = events
            .iter()
            .map(|e| e.ts_iso.as_str())
            .max()
            .map(str::to_string);
        report.new_events = archive.record_events(&events)?;
        if let Some(ts) = newest {
            archive.meta_set(WATERMARK_KEY, &ts)?;
        }
    }

    Ok(report)
}
