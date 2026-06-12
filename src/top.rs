//! `kb top` — the live dashboard. A single rounded frame holding "now playing",
//! lifetime totals, a 24-hour sightings sparkline, the advertiser leaderboard,
//! and the most recent creatives. It refreshes once a second and captures on
//! every tick, so simply leaving it open grows your archive.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Padding, Paragraph, Row, Sparkline, Table,
};
use ratatui::{DefaultTerminal, Frame};

use crate::archive::{AdvertiserStat, Archive, Stats};
use crate::capture::capture_pass;
use crate::model::{host_of, AdRow};
use crate::sources::{self, LiveState};
use crate::{paths, util};

// ---- palette --------------------------------------------------------------

const GOLD: Color = Color::Rgb(245, 197, 66);
const TEAL: Color = Color::Rgb(94, 234, 212);
const GREEN: Color = Color::Rgb(126, 211, 33);
const RED: Color = Color::Rgb(255, 95, 109);
const FG: Color = Color::Rgb(222, 222, 232);
const DIM: Color = Color::Rgb(120, 122, 138);
const FRAME: Color = Color::Rgb(70, 72, 92);

/// `cli-ad.json` freshness window, mirroring the extension.
const FRESH_MS: i64 = 600_000;

// ---- app state ------------------------------------------------------------

#[derive(Default)]
struct App {
    now_ms: i64,
    stats: Stats,
    /// Hourly activity, oldest first. `None` = kb was not watching that hour.
    sparkline: Vec<Option<u64>>,
    leaderboard: Vec<AdvertiserStat>,
    recent: Vec<AdRow>,
    live: LiveState,
    current: Option<crate::model::CliAd>,
    demo: bool,
}

impl App {
    fn refresh(&mut self, archive: &Archive) -> Result<()> {
        let now = util::now_ms();
        self.now_ms = now;
        self.stats = archive.stats(now)?;
        self.sparkline = archive.hourly_activity(now, 24)?;
        self.leaderboard = archive.advertiser_leaderboard(8)?;
        self.recent = archive.list_ads(8)?;
        self.live = sources::read_live_state().unwrap_or_default();
        self.current = sources::read_cli_ad().ok().flatten();
        Ok(())
    }
}

/// Entry point: set up the terminal, run the loop, and always restore on exit.
pub fn run() -> Result<()> {
    let mut archive = Archive::open(&paths::db_path()?)?;
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, &mut archive);
    ratatui::restore();
    result
}

/// Render the dashboard with built-in sample data, so the layout can be seen
/// without waiting for real ads. Touches no archive and writes nothing.
pub fn run_demo() -> Result<()> {
    let app = demo_app();
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|frame| ui(frame, &app))?;
            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        let ctrl_c = key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL);
                        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) || ctrl_c {
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

/// Representative sample data for `--demo` and the README screenshot. Clearly
/// labelled "demo data" in the header so it is never mistaken for real earnings.
fn demo_app() -> App {
    let advertiser = |name: &str, ads: i64, seen: i64| AdvertiserStat {
        advertiser: name.to_string(),
        distinct_ads: ads,
        sightings: seen,
    };
    let recent = |adv: &str, text: &str, url: &str, ago_ms: i64| AdRow {
        id: adv.to_string(),
        advertiser: adv.to_string(),
        ad_text: text.to_string(),
        click_url: Some(url.to_string()),
        first_seen_ms: 1_780_000_000_000,
        last_seen_ms: 1_781_210_000_000 - ago_ms,
        times_seen: 1,
    };
    App {
        now_ms: 1_781_210_000_000,
        stats: Stats {
            distinct_ads: 42,
            advertisers: 17,
            total_sightings: 128,
            sightings_today: 12,
            sightings_week: 63,
            first_seen_ms: Some(1_780_640_040_000),
            last_seen_ms: Some(1_781_209_988_000),
        },
        sparkline: [
            1, 2, 1, 3, 4, 6, 5, 7, 6, 8, 6, 9, 7, 5, 4, 6, 8, 7, 5, 3, 4, 6, 5, 2,
        ]
        .into_iter()
        .map(Some)
        .collect(),
        leaderboard: vec![
            advertiser("Tailscale", 4, 23),
            advertiser("Linear", 3, 19),
            advertiser("Vercel", 5, 17),
            advertiser("Neon", 2, 14),
            advertiser("Sentry", 3, 12),
            advertiser("Supabase", 2, 11),
            advertiser("Fly.io", 1, 9),
            advertiser("Cloudflare", 2, 8),
        ],
        recent: vec![
            recent(
                "Tailscale",
                "Tailscale · the VPN that disappears",
                "https://tailscale.com/",
                12_000,
            ),
            recent(
                "Linear",
                "Linear · issues you actually close",
                "https://linear.app/",
                9 * 60_000,
            ),
            recent(
                "Vercel",
                "Vercel · ship in seconds",
                "https://vercel.com/",
                21 * 60_000,
            ),
            recent(
                "Neon",
                "Neon · Postgres that scales to zero",
                "https://neon.tech/",
                38 * 60_000,
            ),
            recent(
                "Sentry",
                "Sentry · catch errors before users do",
                "https://sentry.io/",
                52 * 60_000,
            ),
            recent(
                "Supabase",
                "Supabase · the open source Firebase",
                "https://supabase.com/",
                70 * 60_000,
            ),
            recent(
                "Fly.io",
                "Fly.io · run your app close to users",
                "https://fly.io/",
                95 * 60_000,
            ),
            recent(
                "Cloudflare",
                "Cloudflare · the network is the computer",
                "https://cloudflare.com/",
                120 * 60_000,
            ),
        ],
        live: LiveState {
            signed_in: Some(true),
            injection_on: Some(true),
            ..Default::default()
        },
        current: Some(crate::model::CliAd {
            ad_text: "Tailscale · the VPN that disappears".to_string(),
            click_url: Some("https://tailscale.com/".to_string()),
            icon_url: None,
            icon_ref: None,
            ts: 1_781_209_988_000,
        }),
        demo: true,
    }
}

fn run_loop(terminal: &mut DefaultTerminal, archive: &mut Archive) -> Result<()> {
    let mut app = App::default();
    capture_pass(archive)?;
    app.refresh(archive)?;
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|frame| ui(frame, &app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let ctrl_c = key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL);
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        _ if ctrl_c => break,
                        KeyCode::Char('r') => {
                            capture_pass(archive)?;
                            app.refresh(archive)?;
                        }
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= Duration::from_secs(1) {
            capture_pass(archive)?;
            app.refresh(archive)?;
            last_tick = Instant::now();
        }
    }
    Ok(())
}

// ---- rendering ------------------------------------------------------------

fn ui(frame: &mut Frame, app: &App) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(FRAME))
        .padding(Padding::new(1, 1, 0, 0))
        .title_top(brand_title(app.demo))
        .title_top(status_chips(&app.live).right_aligned())
        .title_bottom(keybinds_line())
        .title_bottom(ethic_line().right_aligned());

    let inner = outer.inner(frame.area());
    frame.render_widget(outer, frame.area());

    let columns = Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)])
        .spacing(1)
        .split(inner);

    render_left(frame, columns[0], app);
    render_right(frame, columns[1], app);
}

fn brand_title(demo: bool) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            " kickbacks",
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "-kit ",
            Style::default().fg(FG).add_modifier(Modifier::BOLD),
        ),
        Span::styled("· kbtop ", Style::default().fg(DIM)),
    ];
    if demo {
        spans.push(Span::styled(
            "· demo data ",
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        ));
    }
    Line::from(spans)
}

fn status_chips(live: &LiveState) -> Line<'static> {
    let mut spans = Vec::new();
    let signed = live.signed_in.unwrap_or(false);
    spans.push(chip(signed, "signed in", "signed out"));
    spans.push(Span::raw("  "));
    let ads_on = live.injection_on.unwrap_or(false);
    spans.push(chip(ads_on, "ads on", "ads off"));
    if live.killed.unwrap_or(false) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("● killed", Style::default().fg(RED)));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn chip(on: bool, yes: &str, no: &str) -> Span<'static> {
    if on {
        Span::styled(format!("● {yes}"), Style::default().fg(GREEN))
    } else {
        Span::styled(format!("○ {no}"), Style::default().fg(DIM))
    }
}

fn keybinds_line() -> Line<'static> {
    Line::from(vec![
        Span::styled(" q ", Style::default().fg(GOLD)),
        Span::styled("quit  ", Style::default().fg(DIM)),
        Span::styled("r ", Style::default().fg(GOLD)),
        Span::styled("refresh ", Style::default().fg(DIM)),
    ])
}

fn ethic_line() -> Line<'static> {
    Line::from(Span::styled(
        " read-only · observes, never bills ",
        Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
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
fn section(frame: &mut Frame, area: Rect, label: &str) -> Rect {
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let head = Paragraph::new(Line::from(Span::styled(
        label,
        Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(head, parts[0]);
    parts[1]
}

fn render_now_playing(frame: &mut Frame, area: Rect, app: &App) {
    let body = section(frame, area, "NOW PLAYING");
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
                Style::default().fg(RED).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "kickbacks killswitch active",
                Style::default().fg(RED),
            )),
            Line::from(Span::styled(
                "server-side, not you",
                Style::default().fg(DIM),
            )),
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
                    Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    util::truncate(&tagline, area.width.saturating_sub(2) as usize),
                    Style::default().fg(FG),
                )),
                Line::from(Span::styled(
                    format!("{host} · {age}"),
                    Style::default().fg(DIM),
                )),
            ]
        }
        _ => vec![
            Line::from(Span::styled(
                "no ad right now",
                Style::default().fg(DIM).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "code with the extension running to see ads",
                Style::default().fg(DIM),
            )),
        ],
    };

    frame.render_widget(Paragraph::new(lines), body);
}

fn render_totals(frame: &mut Frame, area: Rect, app: &App) {
    let body = section(frame, area, "TOTALS");
    let s = &app.stats;
    let span = |label: &'static str, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<12}"), Style::default().fg(DIM)),
            Span::styled(
                value,
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            ),
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
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        )));
    }
    frame.render_widget(Paragraph::new(lines), body);
}

fn render_sparkline(frame: &mut Frame, area: Rect, app: &App) {
    let body = section(frame, area, "SIGHTINGS · LAST 24H");
    if app.sparkline.iter().all(Option::is_none) {
        let hint = Paragraph::new(Line::from(Span::styled(
            "not watching — run kb watch or keep kb top open",
            Style::default().fg(DIM),
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
    let spark = Sparkline::default()
        .data(app.sparkline.iter().copied())
        .style(Style::default().fg(GOLD))
        .absent_value_symbol("░")
        .absent_value_style(Style::default().fg(DIM));
    frame.render_widget(spark, spark_area);
    if let Some(legend) = legend_area {
        let note = Paragraph::new(Line::from(Span::styled(
            "░ hours kb was not watching",
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        )));
        frame.render_widget(note, legend);
    }
}

fn render_leaderboard(frame: &mut Frame, area: Rect, app: &App) {
    let body = section(frame, area, "TOP ADVERTISERS");
    if app.leaderboard.is_empty() {
        frame.render_widget(empty_hint(), body);
        return;
    }
    let name_w = body.width.saturating_sub(14) as usize;
    let rows = app.leaderboard.iter().enumerate().map(|(i, a)| {
        Row::new(vec![
            Cell::from(Span::styled(
                format!("{:>2}", i + 1),
                Style::default().fg(DIM),
            )),
            Cell::from(Span::styled(
                util::truncate(&a.advertiser, name_w),
                Style::default().fg(FG),
            )),
            Cell::from(Span::styled(
                a.distinct_ads.to_string(),
                Style::default().fg(DIM),
            )),
            Cell::from(Span::styled(
                a.sightings.to_string(),
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            )),
        ])
    });
    let header = Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled("advertiser", Style::default().fg(DIM))),
        Cell::from(Span::styled("ads", Style::default().fg(DIM))),
        Cell::from(Span::styled("seen", Style::default().fg(DIM))),
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
    let body = section(frame, area, "RECENT ADS");
    if app.recent.is_empty() {
        frame.render_widget(empty_hint(), body);
        return;
    }
    let width = body.width.saturating_sub(2) as usize;
    let lines: Vec<Line> = app
        .recent
        .iter()
        .map(|ad| {
            Line::from(vec![
                Span::styled("· ", Style::default().fg(GOLD)),
                Span::styled(util::truncate(&ad.ad_text, width), Style::default().fg(FG)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body);
}

fn empty_hint() -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        "nothing captured yet",
        Style::default().fg(DIM),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::AdvertiserStat;
    use crate::model::{AdRow, CliAd};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn rendered(app: &App) -> String {
        let backend = TestBackend::new(90, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| ui(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
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
            demo: false,
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
    fn demo_app_renders() {
        let out = rendered(&demo_app());
        assert!(out.contains("Tailscale"));
        assert!(out.contains("demo data"));
        assert!(out.contains("TOP ADVERTISERS"));
    }

    /// Not a test: a generator for the README hero image. Run explicitly with
    /// `cargo test --release -- --ignored generate_readme_svg`. Writes
    /// `media/kbtop.svg` from the demo dashboard.
    #[test]
    #[ignore = "asset generator, run manually"]
    fn generate_readme_svg() {
        let backend = TestBackend::new(86, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| ui(f, &demo_app())).unwrap();
        let svg = crate::svg::buffer_to_svg(terminal.backend().buffer());
        std::fs::create_dir_all("media").unwrap();
        std::fs::write("media/kbtop.svg", svg).unwrap();
    }
}
