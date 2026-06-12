//! The shared dashboard renderer. `kb top`, `kb snapshot`, and the README
//! asset generator all draw through the single `ui` function here, so the
//! three surfaces can never disagree about what the dashboard looks like.
//!
//! Colors come from a [`Palette`](crate::theme::Palette) carried on [`App`],
//! not from hardcoded constants, so the same renderer produces the dark, light,
//! and terminal-native looks. When the palette paints a background, `ui` fills
//! the whole canvas first so the dashboard reads the same on any terminal.
//!
//! Demo data is produced by seeding a real in-memory archive and running the
//! same refresh path as live data, so the demo cannot drift from reality.

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Padding, Paragraph, Row, Table};
use ratatui::Frame;

use crate::archive::{AdvertiserStat, Archive, Stats};
use crate::model::{host_of, AdRow, CliAd};
use crate::sources::{self, LiveState};
use crate::theme::{Palette, Theme};
use crate::util;

/// `cli-ad.json` freshness window, mirroring the extension.
const FRESH_MS: i64 = 600_000;

const HOUR_MS: i64 = 60 * 60 * 1000;

/// Where real earnings live. kb stays read-only and offline, so it points here
/// rather than reading balances (that needs the kickbacks.ai cloud backend,
/// which the honesty invariant keeps out of scope).
pub const PORTFOLIO_URL: &str = "https://kickbacks.ai/me";

// ---- app state ------------------------------------------------------------

/// State of the in-TUI theme picker overlay. Present only while the picker is
/// open. The dashboard behind it renders with the previewed theme so the user
/// sees the change live before committing.
#[derive(Debug, Clone)]
pub struct ThemePicker {
    pub options: Vec<Theme>,
    pub cursor: usize,
    /// The theme that was active when the picker opened, restored on cancel.
    pub original: Theme,
}

impl ThemePicker {
    /// Open the picker with the cursor on the currently active theme.
    pub fn open(current: Theme) -> Self {
        let options = Theme::all().to_vec();
        let cursor = options.iter().position(|&t| t == current).unwrap_or(0);
        ThemePicker {
            options,
            cursor,
            original: current,
        }
    }

    /// The theme currently under the cursor (the live preview).
    pub fn selected(&self) -> Theme {
        self.options[self.cursor]
    }

    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn down(&mut self) {
        if self.cursor + 1 < self.options.len() {
            self.cursor += 1;
        }
    }
}

#[derive(Default)]
pub struct App {
    pub now_ms: i64,
    pub stats: Stats,
    /// Hourly activity, oldest first. `None` = kb was not watching that hour.
    pub sparkline: Vec<Option<u64>>,
    pub leaderboard: Vec<AdvertiserStat>,
    pub recent: Vec<AdRow>,
    pub live: LiveState,
    pub current: Option<CliAd>,
    pub demo: bool,
    /// The active theme selection (shown in the keybind line, saved to config).
    pub theme: Theme,
    /// Concrete colors to draw with, derived from `theme`.
    pub palette: Palette,
    /// Some while the theme picker overlay is open.
    pub picker: Option<ThemePicker>,
}

impl App {
    /// Refresh everything: archive queries plus the live extension artifacts.
    pub fn refresh(&mut self, archive: &Archive) -> Result<()> {
        self.refresh_from_archive(archive, util::now_ms())?;
        self.live = sources::read_live_state().unwrap_or_default();
        self.current = sources::read_cli_ad().ok().flatten();
        Ok(())
    }

    /// Apply a theme: store it and resolve its concrete palette.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.palette = theme.palette();
    }

    /// Refresh only the archive-backed panels, relative to `now_ms`. The demo
    /// path uses this with a fixed clock so the output is stable.
    fn refresh_from_archive(&mut self, archive: &Archive, now_ms: i64) -> Result<()> {
        self.now_ms = now_ms;
        self.stats = archive.stats(now_ms)?;
        self.sparkline = archive.hourly_activity(now_ms, 24)?;
        self.leaderboard = archive.advertiser_leaderboard(8)?;
        self.recent = archive.list_ads(8)?;
        Ok(())
    }
}

// ---- demo data ------------------------------------------------------------

/// Fixed demo clock so generated assets are reproducible.
const DEMO_NOW_MS: i64 = 1_781_210_000_000;

/// Hourly sighting counts for the demo, oldest first.
const DEMO_HOURS: [u64; 24] = [
    1, 2, 1, 3, 4, 6, 5, 7, 6, 8, 6, 9, 7, 5, 4, 6, 8, 7, 5, 3, 4, 6, 5, 2,
];

const DEMO_CREATIVES: [(&str, &str); 16] = [
    (
        "Tailscale · the VPN that disappears",
        "https://tailscale.com/",
    ),
    (
        "Tailscale · zero-config mesh networking",
        "https://tailscale.com/",
    ),
    ("Linear · issues you actually close", "https://linear.app/"),
    ("Linear · built for speed", "https://linear.app/"),
    ("Vercel · ship in seconds", "https://vercel.com/"),
    ("Vercel · previews for every push", "https://vercel.com/"),
    ("Neon · Postgres that scales to zero", "https://neon.tech/"),
    ("Neon · branch your database", "https://neon.tech/"),
    (
        "Sentry · catch errors before users do",
        "https://sentry.io/",
    ),
    ("Sentry · trace every release", "https://sentry.io/"),
    (
        "Supabase · the open source Firebase",
        "https://supabase.com/",
    ),
    ("Supabase · auth in five minutes", "https://supabase.com/"),
    ("Fly.io · run your app close to users", "https://fly.io/"),
    ("Fly.io · machines that boot in millis", "https://fly.io/"),
    (
        "Cloudflare · the network is the computer",
        "https://cloudflare.com/",
    ),
    ("Cloudflare · cache everything", "https://cloudflare.com/"),
];

/// Seed an in-memory archive with representative data. Goes through the exact
/// same capture path as real ads.
fn demo_archive(now_ms: i64) -> Result<Archive> {
    let mut archive = Archive::open_in_memory()?;
    let end_hour = now_ms / HOUR_MS * HOUR_MS;
    let mut k = 0usize;
    for (i, &count) in DEMO_HOURS.iter().enumerate() {
        let hour_start = end_hour - (23 - i as i64) * HOUR_MS;
        archive.record_observation(hour_start)?;
        for j in 0..count {
            let (text, url) = DEMO_CREATIVES[k % DEMO_CREATIVES.len()];
            k += 1;
            let observed = hour_start + (j as i64) * 60_000 + 5_000;
            let ad = CliAd {
                ad_text: text.to_string(),
                click_url: Some(url.to_string()),
                icon_url: None,
                icon_ref: None,
                ts: observed,
            };
            archive.capture_ad(&ad, observed)?;
        }
    }
    Ok(archive)
}

/// Build the demo dashboard through the real refresh path. Clearly labelled
/// "demo data" in the header so it is never mistaken for real stats.
pub fn demo_app() -> App {
    let mut app = App {
        demo: true,
        ..App::default()
    };
    if let Ok(archive) = demo_archive(DEMO_NOW_MS) {
        app.refresh_from_archive(&archive, DEMO_NOW_MS).ok();
    }
    app.live = LiveState {
        signed_in: Some(true),
        injection_on: Some(true),
        ..Default::default()
    };
    app.current = Some(CliAd {
        ad_text: "Tailscale · the VPN that disappears".to_string(),
        click_url: Some("https://tailscale.com/".to_string()),
        icon_url: None,
        icon_ref: None,
        ts: DEMO_NOW_MS - 12_000,
    });
    app
}

// ---- rendering ------------------------------------------------------------

/// Foreground-only style. Background is handled once, by the canvas fill in
/// [`ui`], so individual spans never need to carry it.
fn fg(color: Color) -> Style {
    Style::default().fg(color)
}

/// A solid-background style for surfaces (the canvas, the picker overlay).
fn bg_style(pal: &Palette) -> Style {
    match pal.bg {
        Some(bg) => Style::default().bg(bg),
        None => Style::default(),
    }
}

pub fn ui(frame: &mut Frame, app: &App) {
    let pal = &app.palette;
    let area = frame.area();

    // Paint the whole canvas once. Every widget below sets only a foreground,
    // so these background cells survive and the dashboard reads the same on a
    // light or dark terminal. The `terminal` palette leaves `bg` as `None` and
    // inherits the terminal's own background.
    if pal.bg.is_some() {
        frame.render_widget(Block::default().style(bg_style(pal)), area);
    }

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(fg(pal.frame))
        .padding(Padding::new(1, 1, 0, 0))
        .title_top(brand_title(pal, app.demo))
        .title_top(status_chips(pal, &app.live).right_aligned())
        .title_bottom(keybinds_line(pal, app.theme))
        .title_bottom(ethic_line(pal).right_aligned());

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let columns = Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)])
        .spacing(1)
        .split(inner);

    render_left(frame, columns[0], app);
    render_right(frame, columns[1], app);

    if app.picker.is_some() {
        render_theme_picker(frame, area, app);
    }
}

fn brand_title(pal: &Palette, demo: bool) -> Line<'static> {
    let mut spans = vec![
        Span::styled(" kickbacks", fg(pal.gold).add_modifier(Modifier::BOLD)),
        Span::styled("-kit ", fg(pal.fg).add_modifier(Modifier::BOLD)),
        Span::styled("· kbtop ", fg(pal.dim)),
    ];
    if demo {
        spans.push(Span::styled(
            "· demo data ",
            fg(pal.dim).add_modifier(Modifier::ITALIC),
        ));
    }
    Line::from(spans)
}

fn status_chips(pal: &Palette, live: &LiveState) -> Line<'static> {
    let mut spans = Vec::new();
    let signed = live.signed_in.unwrap_or(false);
    spans.push(chip(pal, signed, "signed in", "signed out"));
    spans.push(Span::raw("  "));
    let ads_on = live.injection_on.unwrap_or(false);
    spans.push(chip(pal, ads_on, "ads on", "ads off"));
    if live.killed.unwrap_or(false) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("● killed", fg(pal.red)));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn chip(pal: &Palette, on: bool, yes: &str, no: &str) -> Span<'static> {
    if on {
        Span::styled(format!("● {yes}"), fg(pal.green))
    } else {
        Span::styled(format!("○ {no}"), fg(pal.dim))
    }
}

fn keybinds_line(pal: &Palette, theme: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(" q ", fg(pal.gold)),
        Span::styled("quit  ", fg(pal.dim)),
        Span::styled("r ", fg(pal.gold)),
        Span::styled("refresh  ", fg(pal.dim)),
        Span::styled("t ", fg(pal.gold)),
        Span::styled(format!("theme: {} ", theme.label()), fg(pal.dim)),
    ])
}

fn ethic_line(pal: &Palette) -> Line<'static> {
    Line::from(Span::styled(
        " read-only · observes, never bills ",
        fg(pal.dim).add_modifier(Modifier::ITALIC),
    ))
}

fn render_left(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(7), Constraint::Min(0)]).split(area);
    render_now_playing(frame, rows[0], app);
    render_totals(frame, rows[1], app);
}

fn render_right(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(7),
        Constraint::Length(11),
        Constraint::Min(0),
    ])
    .split(area);
    render_sparkline(frame, rows[0], app);
    render_leaderboard(frame, rows[1], app);
    render_recent(frame, rows[2], app);
}

/// Build a section: a label line, then the body rect beneath it.
fn section(frame: &mut Frame, area: Rect, pal: &Palette, label: &str) -> Rect {
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let head = Paragraph::new(Line::from(Span::styled(
        label,
        fg(pal.teal).add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(head, parts[0]);
    parts[1]
}

fn render_now_playing(frame: &mut Frame, area: Rect, app: &App) {
    let pal = &app.palette;
    let body = section(frame, area, pal, "NOW PLAYING");
    let fresh = app
        .current
        .as_ref()
        .map(|a| app.now_ms - a.ts <= FRESH_MS)
        .unwrap_or(false);

    // Killswitch is the surprising state: make it impossible to miss.
    if app.live.killed.unwrap_or(false) {
        let lines = vec![
            Line::from(Span::styled(
                "● ADS PAUSED",
                fg(pal.red).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled("kickbacks killswitch active", fg(pal.red))),
            Line::from(Span::styled("server-side, not you", fg(pal.dim))),
        ];
        frame.render_widget(Paragraph::new(lines), body);
        return;
    }

    let lines = match (&app.current, fresh) {
        (Some(ad), true) => {
            let advertiser = ad.advertiser();
            let tagline = ad.tagline();
            let host = ad
                .click_url
                .as_deref()
                .and_then(host_of)
                .unwrap_or_default();
            let age = util::human_age(app.now_ms - ad.ts);
            vec![
                Line::from(Span::styled(
                    advertiser,
                    fg(pal.gold).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    util::truncate(&tagline, area.width.saturating_sub(2) as usize),
                    fg(pal.fg),
                )),
                Line::from(Span::styled(format!("{host} · {age}"), fg(pal.dim))),
            ]
        }
        _ => vec![
            Line::from(Span::styled(
                "no ad right now",
                fg(pal.dim).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "code with the extension running to see ads",
                fg(pal.dim),
            )),
        ],
    };

    frame.render_widget(Paragraph::new(lines), body);
}

fn render_totals(frame: &mut Frame, area: Rect, app: &App) {
    let pal = &app.palette;
    let body = section(frame, area, pal, "TOTALS");
    let s = &app.stats;
    let span = |label: &'static str, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<12}"), fg(pal.dim)),
            Span::styled(value, fg(pal.teal).add_modifier(Modifier::BOLD)),
        ])
    };
    let mut lines = vec![
        span("ads seen", s.distinct_ads.to_string()),
        span("advertisers", s.advertisers.to_string()),
        span("sightings", s.total_sightings.to_string()),
        span("today", s.sightings_today.to_string()),
        span("this week", s.sightings_week.to_string()),
    ];
    if let Some(first) = s.first_seen_ms {
        lines.push(Line::from(Span::styled(
            format!("since {}", util::fmt_datetime(first)),
            fg(pal.dim).add_modifier(Modifier::ITALIC),
        )));
    }
    // Earnings deliberately live off-screen: kb never reads balances (that
    // needs the cloud backend). Point the user to the real number instead of
    // inventing one.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("$ earnings", fg(pal.dim))));
    lines.push(Line::from(Span::styled(PORTFOLIO_URL, fg(pal.gold))));
    lines.push(Line::from(Span::styled(
        "read-only · kb does not read balances",
        fg(pal.dim).add_modifier(Modifier::ITALIC),
    )));
    frame.render_widget(Paragraph::new(lines), body);
}

fn render_sparkline(frame: &mut Frame, area: Rect, app: &App) {
    let pal = &app.palette;
    let body = section(frame, area, pal, "SIGHTINGS · LAST 24H");
    if app.sparkline.iter().all(Option::is_none) {
        let hint = Paragraph::new(Line::from(Span::styled(
            "not watching — run kb watch or keep kb top open",
            fg(pal.dim),
        )));
        frame.render_widget(hint, body);
        return;
    }
    let has_gaps = app.sparkline.iter().any(Option::is_none);
    let (spark_area, legend_area) = if has_gaps && body.height > 1 {
        let parts = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(body);
        (parts[0], Some(parts[1]))
    } else {
        (body, None)
    };
    frame.render_widget(
        Paragraph::new(bars(&app.sparkline, spark_area, pal)),
        spark_area,
    );
    if let Some(legend) = legend_area {
        let note = Paragraph::new(Line::from(Span::styled(
            "░ hours kb was not watching",
            fg(pal.dim).add_modifier(Modifier::ITALIC),
        )));
        frame.render_widget(note, legend);
    }
}

/// Eighth-height block glyphs for a bar column, 1/8 (`▁`) to 8/8 (`█`).
const BAR_BLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Total eighths to fill for `value` against `max` in a column `rows` cells
/// high. Any nonzero value fills at least one eighth, so a real sighting never
/// renders as an empty column.
fn column_eighths(value: u64, max: u64, rows: u16) -> u16 {
    if value == 0 || max == 0 || rows == 0 {
        return 0;
    }
    let total = rows as u64 * 8;
    (value * total).div_ceil(max).clamp(1, total) as u16
}

/// The glyph for one cell, given the column's total eighths and how many rows
/// up from the baseline the cell sits. `None` means an empty cell.
fn bar_cell(col_eighths: u16, row_from_bottom: u16) -> Option<&'static str> {
    let base = row_from_bottom * 8;
    if col_eighths <= base {
        return None;
    }
    let local = (col_eighths - base).min(8);
    Some(BAR_BLOCKS[local as usize - 1])
}

/// Build the activity chart. Observed hours rise as gold bars scaled to the
/// busiest hour; unobserved hours show a single dim baseline mark instead of a
/// full-height shaded slab, which is what turned the old chart into a wall of
/// gray when most hours had no data. Watched-but-quiet hours (`Some(0)`) stay
/// blank, so they read differently from unobserved ones.
fn bars(data: &[Option<u64>], area: Rect, pal: &Palette) -> Vec<Line<'static>> {
    let w = (area.width as usize).min(data.len());
    let h = area.height.max(1);
    if w == 0 {
        return Vec::new();
    }
    let visible = &data[data.len() - w..];
    let max = visible.iter().filter_map(|v| *v).max().unwrap_or(0);

    let mut lines = Vec::with_capacity(h as usize);
    for row in (0..h).rev() {
        let mut spans = Vec::with_capacity(w);
        for &hour in visible {
            match hour {
                None if row == 0 => spans.push(Span::styled("░", fg(pal.dim))),
                Some(v) if v > 0 => match bar_cell(column_eighths(v, max, h), row) {
                    Some(block) => spans.push(Span::styled(block, fg(pal.gold))),
                    None => spans.push(Span::raw(" ")),
                },
                _ => spans.push(Span::raw(" ")),
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn render_leaderboard(frame: &mut Frame, area: Rect, app: &App) {
    let pal = &app.palette;
    let body = section(frame, area, pal, "TOP ADVERTISERS");
    if app.leaderboard.is_empty() {
        frame.render_widget(empty_hint(pal), body);
        return;
    }
    let name_w = body.width.saturating_sub(14) as usize;
    let rows = app.leaderboard.iter().enumerate().map(|(i, a)| {
        Row::new(vec![
            Cell::from(Span::styled(format!("{:>2}", i + 1), fg(pal.dim))),
            Cell::from(Span::styled(
                util::truncate(&a.advertiser, name_w),
                fg(pal.fg),
            )),
            Cell::from(Span::styled(a.distinct_ads.to_string(), fg(pal.dim))),
            Cell::from(Span::styled(
                a.sightings.to_string(),
                fg(pal.gold).add_modifier(Modifier::BOLD),
            )),
        ])
    });
    let header = Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled("advertiser", fg(pal.dim))),
        Cell::from(Span::styled("ads", fg(pal.dim))),
        Cell::from(Span::styled("seen", fg(pal.dim))),
    ]);
    let widths = [
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(4),
        Constraint::Length(5),
    ];
    let table = Table::new(rows, widths).header(header).column_spacing(1);
    frame.render_widget(table, body);
}

fn render_recent(frame: &mut Frame, area: Rect, app: &App) {
    let pal = &app.palette;
    let body = section(frame, area, pal, "RECENT ADS");
    if app.recent.is_empty() {
        frame.render_widget(empty_hint(pal), body);
        return;
    }
    let width = body.width.saturating_sub(2) as usize;
    let lines: Vec<Line> = app
        .recent
        .iter()
        .map(|ad| {
            Line::from(vec![
                Span::styled("· ", fg(pal.gold)),
                Span::styled(util::truncate(&ad.ad_text, width), fg(pal.fg)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body);
}

fn empty_hint(pal: &Palette) -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        "nothing captured yet",
        fg(pal.dim),
    )))
}

/// A centered rect of the given size, clamped to `area`.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// Draw the theme picker overlay on top of the (already previewed) dashboard.
fn render_theme_picker(frame: &mut Frame, area: Rect, app: &App) {
    let pal = &app.palette;
    let Some(picker) = &app.picker else { return };

    let width = 46;
    let height = picker.options.len() as u16 + 4;
    let rect = centered(area, width, height);

    // Clear the region, then paint our own surface so the overlay is opaque.
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(fg(pal.gold))
        .style(bg_style(pal))
        .padding(Padding::new(1, 1, 0, 0))
        .title_top(Line::from(Span::styled(
            " choose a theme ",
            fg(pal.gold).add_modifier(Modifier::BOLD),
        )));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let parts = Layout::vertical([
        Constraint::Length(picker.options.len() as u16),
        Constraint::Length(1),
    ])
    .split(inner);

    let lines: Vec<Line> = picker
        .options
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let selected = i == picker.cursor;
            let marker = if selected { "›" } else { " " };
            let name_style = if selected {
                fg(pal.gold).add_modifier(Modifier::BOLD)
            } else {
                fg(pal.fg)
            };
            Line::from(vec![
                Span::styled(format!("{marker} "), fg(pal.gold)),
                Span::styled(format!("{:<9}", t.label()), name_style),
                Span::styled(t.hint().to_string(), fg(pal.dim)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), parts[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑↓ preview · enter save · esc cancel",
            fg(pal.dim).add_modifier(Modifier::ITALIC),
        ))),
        parts[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Stats;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn buffer_of(app: &App) -> Buffer {
        let backend = TestBackend::new(90, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| ui(f, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn text_of(buf: &Buffer) -> String {
        buf.content.iter().map(|c| c.symbol()).collect()
    }

    fn rendered(app: &App) -> String {
        text_of(&buffer_of(app))
    }

    #[test]
    fn renders_empty_state_without_panicking() {
        let app = App::default();
        let out = rendered(&app);
        assert!(out.contains("kickbacks"));
        assert!(out.contains("NOW PLAYING"));
        assert!(out.contains("no ad right now"));
    }

    #[test]
    fn renders_populated_state() {
        let app = App {
            now_ms: 1_781_210_400_000,
            stats: Stats {
                distinct_ads: 3,
                advertisers: 3,
                total_sightings: 5,
                sightings_today: 5,
                sightings_week: 5,
                first_seen_ms: Some(1_781_210_098_155),
                last_seen_ms: Some(1_781_210_380_000),
            },
            sparkline: vec![None, Some(1), Some(2), Some(3), Some(2), Some(1), Some(4)],
            leaderboard: vec![AdvertiserStat {
                advertiser: "Tailscale".to_string(),
                distinct_ads: 1,
                sightings: 2,
            }],
            recent: vec![AdRow {
                id: "abc".to_string(),
                advertiser: "Tailscale".to_string(),
                ad_text: "Tailscale · the VPN that disappears".to_string(),
                click_url: Some("https://tailscale.com/".to_string()),
                first_seen_ms: 1_781_210_098_155,
                last_seen_ms: 1_781_210_380_000,
                times_seen: 2,
            }],
            live: LiveState {
                signed_in: Some(true),
                injection_on: Some(true),
                ..Default::default()
            },
            current: Some(CliAd {
                ad_text: "Tailscale · the VPN that disappears".to_string(),
                click_url: Some("https://tailscale.com/".to_string()),
                icon_url: None,
                icon_ref: None,
                ts: 1_781_210_399_000,
            }),
            ..App::default()
        };
        let out = rendered(&app);
        assert!(out.contains("Tailscale"));
        assert!(out.contains("TOP ADVERTISERS"));
        assert!(out.contains("signed in"));
    }

    #[test]
    fn sparkline_gaps_render_as_not_watching() {
        let app = App {
            now_ms: 1_781_210_400_000,
            sparkline: vec![None, Some(2), None, Some(4)],
            ..App::default()
        };
        let out = rendered(&app);
        assert!(out.contains("░"));
        assert!(out.contains("hours kb was not watching"));
    }

    #[test]
    fn sparkline_all_unobserved_says_not_watching() {
        let app = App {
            sparkline: vec![None; 24],
            ..App::default()
        };
        let out = rendered(&app);
        assert!(out.contains("not watching — run kb watch"));
    }

    #[test]
    fn bar_height_scales_and_never_vanishes() {
        assert_eq!(column_eighths(4, 4, 5), 40); // full value fills the column
        assert_eq!(column_eighths(1, 1000, 5), 1); // tiny value still shows 1/8
        assert_eq!(column_eighths(0, 4, 5), 0);
        assert_eq!(column_eighths(4, 0, 5), 0);
    }

    #[test]
    fn bar_cells_fill_bottom_up() {
        // Column of 10/40 eighths: bottom cell full, next cell 2/8, rest empty.
        assert_eq!(bar_cell(10, 0), Some("█"));
        assert_eq!(bar_cell(10, 1), Some("▂"));
        assert_eq!(bar_cell(10, 2), None);
    }

    #[test]
    fn gaps_only_mark_the_baseline_not_full_height() {
        // Mostly unobserved with one tall bar. Gap columns must show a single
        // baseline mark, never a full-height shaded slab (the old ugliness):
        // the shade-char count stays near one row, not rows times columns.
        let app = App {
            now_ms: 1,
            sparkline: {
                let mut d = vec![None; 24];
                d[23] = Some(10);
                d
            },
            ..App::default()
        };
        let text = rendered(&app);
        assert!(text.contains("█"), "the data bar should render");
        let shades = text.matches('░').count();
        assert!(shades <= 30, "shade slab regression: {shades}");
    }

    #[test]
    fn demo_app_renders() {
        let out = rendered(&demo_app());
        assert!(out.contains("Tailscale"));
        assert!(out.contains("demo data"));
        assert!(out.contains("TOP ADVERTISERS"));
    }

    #[test]
    fn earnings_pointer_is_honest_not_a_number() {
        // The dashboard must point to where earnings live, never show a
        // fabricated balance. This guards the honesty invariant.
        let out = rendered(&demo_app());
        assert!(out.contains("earnings"));
        assert!(out.contains("kickbacks.ai/me"));
        assert!(out.contains("does not read balances"));
    }

    #[test]
    fn demo_app_is_internally_consistent() {
        // The demo flows through the real archive, so its numbers must agree
        // with each other: sightings = sum of the hourly sparkline.
        let app = demo_app();
        let spark_total: u64 = app.sparkline.iter().map(|v| v.unwrap_or(0)).sum();
        assert_eq!(app.stats.total_sightings as u64, spark_total);
        assert_eq!(app.stats.advertisers, 8);
        assert!(app.sparkline.iter().all(Option::is_some));
        assert!(!app.leaderboard.is_empty());
        assert!(!app.recent.is_empty());
    }

    // ---- theming -----------------------------------------------------------

    #[test]
    fn every_theme_renders_without_panicking() {
        for theme in Theme::all() {
            let mut app = demo_app();
            app.set_theme(theme);
            let out = rendered(&app);
            assert!(out.contains("kickbacks"), "theme {:?} lost content", theme);
            assert!(out.contains("TOP ADVERTISERS"));
        }
    }

    #[test]
    fn painted_theme_fills_the_canvas() {
        // Dark/light paint a background, so the top-left cell carries the
        // palette's bg color rather than the terminal default. This is the fix
        // for the washed-out-on-a-light-terminal bug.
        for theme in [Theme::Dark, Theme::Light] {
            let mut app = demo_app();
            app.set_theme(theme);
            let buf = buffer_of(&app);
            let bg = app.palette.bg.unwrap();
            assert_eq!(buf.content[0].bg, bg, "theme {:?} did not paint", theme);
        }
    }

    #[test]
    fn terminal_theme_inherits_the_background() {
        let mut app = demo_app();
        app.set_theme(Theme::Terminal);
        let buf = buffer_of(&app);
        // No painted canvas: the cell keeps the default (reset) background.
        assert_eq!(buf.content[0].bg, Color::Reset);
    }

    #[test]
    fn theme_label_is_shown_in_the_keybind_line() {
        let mut app = demo_app();
        app.set_theme(Theme::Light);
        assert!(rendered(&app).contains("theme: light"));
    }

    #[test]
    fn picker_overlay_lists_themes() {
        let mut app = demo_app();
        app.picker = Some(ThemePicker::open(app.theme));
        let out = rendered(&app);
        assert!(out.contains("choose a theme"));
        assert!(out.contains("auto"));
        assert!(out.contains("light"));
        assert!(out.contains("terminal"));
        assert!(out.contains("enter save"));
    }

    #[test]
    fn picker_cursor_starts_on_current_theme() {
        let picker = ThemePicker::open(Theme::Light);
        assert_eq!(picker.selected(), Theme::Light);
        let picker = ThemePicker::open(Theme::Terminal);
        assert_eq!(picker.selected(), Theme::Terminal);
    }

    #[test]
    fn picker_navigation_is_clamped() {
        let mut picker = ThemePicker::open(Theme::Auto); // cursor 0
        picker.up();
        assert_eq!(picker.cursor, 0);
        for _ in 0..10 {
            picker.down();
        }
        assert_eq!(picker.cursor, picker.options.len() - 1);
    }

    /// Not a test: a generator for the README hero image. Run explicitly with
    /// `cargo test --release -- --ignored generate_readme_svg`. Writes
    /// `media/kbtop.svg` from the demo dashboard. Always uses the dark theme so
    /// the hero stays consistent.
    #[test]
    #[ignore = "asset generator, run manually"]
    fn generate_readme_svg() {
        let mut app = demo_app();
        app.set_theme(Theme::Dark);
        let backend = TestBackend::new(86, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| ui(f, &app)).unwrap();
        let svg = crate::svg::buffer_to_svg(terminal.backend().buffer());
        std::fs::create_dir_all("media").unwrap();
        std::fs::write("media/kbtop.svg", svg).unwrap();
    }
}
