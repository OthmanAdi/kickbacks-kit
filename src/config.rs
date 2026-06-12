//! User configuration: a tiny JSON file that currently holds only the chosen
//! dashboard theme. Reading and writing it is best-effort and never fatal: a
//! missing, unreadable, or malformed config falls back to defaults, because a
//! theme preference must never stop the tool from running.
//!
//! Location: `KICKBACKS_KIT_CONFIG` if set, else
//! `<config-dir>/kickbacks-kit/config.json` (on Windows that is
//! `%APPDATA%\kickbacks-kit\config.json`).

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::chart::ChartStyle;
use crate::theme::Theme;

/// Persisted user preferences. Every field has a serde default, so a config
/// written by an older build (missing a field) still loads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Dashboard theme for `kb top` and `kb snapshot`.
    #[serde(default = "default_theme")]
    pub theme: Theme,
    /// Activity chart style for the sightings panel.
    #[serde(default)]
    pub chart_style: ChartStyle,
    /// Live status feed settings.
    #[serde(default)]
    pub feed: FeedConfig,
}

/// Settings for the live status feed (the network surface).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedConfig {
    /// When false, kb never touches the network: the feed renders from cache
    /// only and no fetch is attempted. The headline feature is on by default,
    /// with a clear, documented way to turn it off here, via `--offline`, or via
    /// the `KICKBACKS_KIT_OFFLINE` environment variable.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for FeedConfig {
    fn default() -> Self {
        FeedConfig { enabled: true }
    }
}

fn default_true() -> bool {
    true
}

/// First-run default: detect the terminal background rather than imposing a
/// look. Falls back to the painted dark canvas when nothing can be detected.
fn default_theme() -> Theme {
    Theme::Auto
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: default_theme(),
            chart_style: ChartStyle::default(),
            feed: FeedConfig::default(),
        }
    }
}

/// Resolve the config file path, honoring `KICKBACKS_KIT_CONFIG`.
pub fn config_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("KICKBACKS_KIT_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    let base = dirs::config_dir().context("could not resolve platform config directory")?;
    Ok(base.join("kickbacks-kit").join("config.json"))
}

/// Load the config, returning defaults on any error (missing file, bad JSON,
/// unreadable path). Never fails: callers always get a usable `Config`.
pub fn load() -> Config {
    let Ok(path) = config_path() else {
        return Config::default();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Persist only the dashboard preferences (theme and chart style), preserving
/// any other settings already on disk (such as the feed toggle). The in-TUI
/// pickers change just these two, so they must not clobber the rest of config.
pub fn save_prefs(theme: Theme, chart_style: ChartStyle) -> Result<()> {
    let mut cfg = load();
    cfg.theme = theme;
    cfg.chart_style = chart_style;
    save(&cfg)
}

/// Persist the config, creating the parent directory as needed. Returns an
/// error so callers can surface a save failure, but a failure here is never
/// fatal to the running command.
pub fn save(config: &Config) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating config dir: {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(config).context("serializing config")?;
    std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `config_path` reads a process-global env var; serialize the tests that
    // depend on it so they do not race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_theme_is_auto() {
        assert_eq!(Config::default().theme, Theme::Auto);
    }

    #[test]
    fn roundtrips_through_disk() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("kb-config-test-{}", std::process::id()));
        let file = dir.join("config.json");
        std::env::set_var("KICKBACKS_KIT_CONFIG", &file);

        save(&Config {
            theme: Theme::Light,
            chart_style: ChartStyle::Bars,
            feed: FeedConfig { enabled: false },
        })
        .unwrap();
        let loaded = load();
        assert_eq!(loaded.theme, Theme::Light);
        assert_eq!(loaded.chart_style, ChartStyle::Bars);
        assert!(!loaded.feed.enabled);

        std::env::remove_var("KICKBACKS_KIT_CONFIG");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_prefs_preserves_feed_setting() {
        let _guard = ENV_LOCK.lock().unwrap();
        let file =
            std::env::temp_dir().join(format!("kb-config-prefs-{}.json", std::process::id()));
        std::env::set_var("KICKBACKS_KIT_CONFIG", &file);

        save(&Config {
            theme: Theme::Dark,
            chart_style: ChartStyle::Heat,
            feed: FeedConfig { enabled: false },
        })
        .unwrap();
        // Changing only the theme must not re-enable the feed.
        save_prefs(Theme::Light, ChartStyle::Bars).unwrap();
        let loaded = load();
        assert_eq!(loaded.theme, Theme::Light);
        assert_eq!(loaded.chart_style, ChartStyle::Bars);
        assert!(!loaded.feed.enabled);

        std::env::remove_var("KICKBACKS_KIT_CONFIG");
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn config_without_chart_style_field_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        let file =
            std::env::temp_dir().join(format!("kb-config-legacy-{}.json", std::process::id()));
        std::env::set_var("KICKBACKS_KIT_CONFIG", &file);
        // A config written by an older build, before chart_style existed.
        std::fs::write(&file, r#"{"theme":"dark"}"#).unwrap();
        let loaded = load();
        assert_eq!(loaded.theme, Theme::Dark);
        assert_eq!(loaded.chart_style, ChartStyle::default());

        std::env::remove_var("KICKBACKS_KIT_CONFIG");
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn missing_or_bad_file_falls_back_to_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        let file =
            std::env::temp_dir().join(format!("kb-config-missing-{}.json", std::process::id()));
        std::fs::remove_file(&file).ok();
        std::env::set_var("KICKBACKS_KIT_CONFIG", &file);
        assert_eq!(load().theme, Theme::Auto);

        std::fs::write(&file, "{ not valid json").unwrap();
        assert_eq!(load().theme, Theme::Auto);

        std::env::remove_var("KICKBACKS_KIT_CONFIG");
        std::fs::remove_file(&file).ok();
    }
}
