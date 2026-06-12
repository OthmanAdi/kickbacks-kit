//! `kb top` — the live, interactive dashboard. The actual drawing lives in
//! [`crate::render`]; this module owns only the terminal lifecycle and the
//! event loop. It refreshes once a second and captures on every tick, so
//! simply leaving it open grows your archive.
//!
//! Theme handling lives here too: `t` opens a picker overlay, the arrow keys
//! preview each theme live, Enter saves the choice to the config, and Esc
//! reverts. The initial theme comes from the `--theme` flag, else the saved
//! config (which defaults to auto-detect).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::archive::Archive;
use crate::capture::capture_pass;
use crate::chart::ChartStyle;
use crate::config::{self, Config};
use crate::feed::sync;
use crate::feed::FeedSnapshot;
use crate::paths;
use crate::render::{demo_app, ui, App, ThemePicker};
use crate::theme::Theme;

/// How often the background thread refreshes the live feed.
const FEED_REFRESH: Duration = Duration::from_secs(900);

/// What a key press asks the loop to do beyond mutating `App`.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// Nothing for the loop to do (the key was fully handled in `App`).
    None,
    /// Quit the dashboard.
    Quit,
    /// Re-capture and refresh now, and poke the feed thread.
    Refresh,
    /// Open the current selection's URL in a browser.
    Open,
}

/// A message from the background feed thread to the UI loop.
enum FeedUpdate {
    /// A fetch has started (show the syncing indicator).
    Fetching,
    /// A fetch finished with this snapshot.
    Done(FeedSnapshot),
}

/// Entry point: set up the terminal, run the loop, and always restore on exit.
/// `theme_arg`/`chart_arg` are the flags; when absent the saved config decides.
pub fn run(theme_arg: Option<Theme>, chart_arg: Option<ChartStyle>) -> Result<()> {
    let mut archive = Archive::open(&paths::db_path()?)?;
    let cfg = config::load();
    let theme = theme_arg.unwrap_or(cfg.theme);
    let chart = chart_arg.unwrap_or(cfg.chart_style);
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, &mut archive, &cfg, theme, chart);
    ratatui::restore();
    result
}

/// Render the dashboard with built-in sample data, so the layout can be seen
/// without waiting for real ads. Touches no archive and writes nothing, so the
/// theme picker here previews but never persists.
pub fn run_demo(theme_arg: Option<Theme>, chart_arg: Option<ChartStyle>) -> Result<()> {
    let mut app = demo_app();
    app.set_theme(theme_arg.unwrap_or(Theme::Dark));
    if let Some(chart) = chart_arg {
        app.chart_style = chart;
    }
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|frame| ui(frame, &app))?;
            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match handle_key(&mut app, key, false) {
                            Action::Quit => break,
                            Action::Open => open_selected(&app),
                            _ => {}
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

fn run_loop(
    terminal: &mut DefaultTerminal,
    archive: &mut Archive,
    cfg: &Config,
    theme: Theme,
    chart: ChartStyle,
) -> Result<()> {
    let mut app = App::default();
    app.set_theme(theme);
    app.chart_style = chart;
    capture_pass(archive)?;
    app.refresh(archive)?;

    // Seed the Feed tab from cache immediately, so it is populated the moment
    // the user switches to it, before the first network round trip returns.
    app.offline = sync::offline_reason(false, cfg);
    app.feed = sync::read_cached(archive, app.offline.is_some());
    app.clamp_cursors();

    // Start the background fetch thread unless the feed is offline. It owns its
    // own archive handle (SQLite WAL allows the concurrent writer) and reports
    // snapshots back over a channel; the UI never blocks on the network.
    let (snap_tx, snap_rx) = mpsc::channel::<FeedUpdate>();
    let (req_tx, req_rx) = mpsc::channel::<()>();
    if app.offline.is_none() {
        let cfg = cfg.clone();
        thread::spawn(move || feed_fetch_loop(&snap_tx, &req_rx, &cfg));
    }

    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|frame| ui(frame, &app))?;

        // Drain any feed updates the background thread has posted.
        while let Ok(update) = snap_rx.try_recv() {
            match update {
                FeedUpdate::Fetching => app.fetching = true,
                FeedUpdate::Done(snapshot) => {
                    app.feed = snapshot;
                    app.fetching = false;
                    app.clamp_cursors();
                }
            }
        }

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match handle_key(&mut app, key, true) {
                        Action::Quit => break,
                        Action::Refresh => {
                            capture_pass(archive)?;
                            app.refresh(archive)?;
                            // Ask the feed thread to fetch now (ignored if offline).
                            let _ = req_tx.send(());
                        }
                        Action::Open => open_selected(&app),
                        Action::None => {}
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
    // Dropping req_tx/snap_rx disconnects the channels; the fetch thread sees
    // the disconnect and exits. We do not join: a fetch in flight has a bounded
    // timeout, and the process is exiting anyway, so quitting stays instant.
    Ok(())
}

/// The background feed loop: fetch immediately, then on every refresh request or
/// after the refresh interval, until the UI disconnects the channel.
fn feed_fetch_loop(snap_tx: &Sender<FeedUpdate>, req_rx: &Receiver<()>, cfg: &Config) {
    loop {
        if snap_tx.send(FeedUpdate::Fetching).is_err() {
            return;
        }
        // A fresh handle each cycle keeps this independent of the UI's handle.
        let snapshot = match paths::db_path().and_then(|p| Archive::open(&p)) {
            Ok(mut archive) => sync::sync(&mut archive, cfg, false),
            Err(_) => FeedSnapshot {
                offline: false,
                ..Default::default()
            },
        };
        if snap_tx.send(FeedUpdate::Done(snapshot)).is_err() {
            return;
        }
        match req_rx.recv_timeout(FEED_REFRESH) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Open the current selection's URL, if any. Best effort: a failure to launch a
/// browser must never disturb the running TUI.
fn open_selected(app: &App) {
    if let Some(url) = app.selected_url() {
        let _ = webbrowser::open(&url);
    }
}

/// Handle one key press, mutating `App` and returning an [`Action`] for the
/// things the loop must do (quit, refresh, open a URL). `persist` controls
/// whether confirming a theme or chart writes it to the config (off for the
/// no-write demo).
fn handle_key(app: &mut App, key: event::KeyEvent, persist: bool) -> Action {
    let ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl_c {
        return Action::Quit;
    }

    // The theme picker captures all keys while open.
    if app.picker.is_some() {
        handle_picker(app, key.code, persist);
        return Action::None;
    }

    match key.code {
        KeyCode::Char('q') => return Action::Quit,
        // Esc quits only from the dashboard; on a list tab it returns to it.
        KeyCode::Esc => {
            if app.tab == crate::render::Tab::Dashboard {
                return Action::Quit;
            }
            app.tab = crate::render::Tab::Dashboard;
        }
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => app.tab = app.tab.cycle(true),
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => app.tab = app.tab.cycle(false),
        KeyCode::Char('1') => app.tab = crate::render::Tab::Dashboard,
        KeyCode::Char('2') => app.tab = crate::render::Tab::Feed,
        KeyCode::Char('3') => app.tab = crate::render::Tab::Links,
        KeyCode::Char('r') => return Action::Refresh,
        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(true),
        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(false),
        KeyCode::Char('o') | KeyCode::Enter => return Action::Open,
        // `t` and `c` are dashboard controls; ignore them on the other tabs so
        // they cannot surprise the user with an off-screen change.
        KeyCode::Char('t') if app.tab == crate::render::Tab::Dashboard => {
            app.picker = Some(ThemePicker::open(app.theme));
        }
        KeyCode::Char('c') if app.tab == crate::render::Tab::Dashboard => {
            app.chart_style = app.chart_style.next();
            if persist {
                let _ = config::save_prefs(app.theme, app.chart_style);
            }
        }
        _ => {}
    }
    Action::None
}

/// Drive the open theme picker. Arrow keys (or j/k) move the cursor and preview
/// the theme live; Enter commits (and saves when `persist`); Esc or q reverts.
fn handle_picker(app: &mut App, code: KeyCode, persist: bool) {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(p) = app.picker.as_mut() {
                p.up();
            }
            preview_selected(app);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(p) = app.picker.as_mut() {
                p.down();
            }
            preview_selected(app);
        }
        KeyCode::Enter => {
            if let Some(theme) = app.picker.as_ref().map(ThemePicker::selected) {
                app.set_theme(theme);
                if persist {
                    let _ = config::save_prefs(theme, app.chart_style);
                }
            }
            app.picker = None;
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            if let Some(theme) = app.picker.as_ref().map(|p| p.original) {
                app.set_theme(theme);
            }
            app.picker = None;
        }
        _ => {}
    }
}

/// Apply the theme currently under the picker cursor, so the dashboard behind
/// the overlay updates as the user navigates.
fn preview_selected(app: &mut App) {
    if let Some(theme) = app.picker.as_ref().map(ThemePicker::selected) {
        app.set_theme(theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::{FeedItem, FeedKind, FeedSnapshot};
    use crate::render::{App, Tab};

    fn press(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn feed_with(n: usize) -> FeedSnapshot {
        let items = (0..n)
            .map(|i| {
                FeedItem::new(
                    FeedKind::Issue,
                    format!("#{i} issue"),
                    "",
                    Some(format!("https://example.com/{i}")),
                    Some(i as i64),
                    "github",
                )
            })
            .collect();
        FeedSnapshot {
            items,
            ..Default::default()
        }
    }

    #[test]
    fn t_opens_and_esc_reverts_theme() {
        let mut app = App::default();
        app.set_theme(Theme::Dark);
        assert_eq!(
            handle_key(&mut app, press(KeyCode::Char('t')), false),
            Action::None
        );
        assert!(app.picker.is_some());

        handle_picker(&mut app, KeyCode::Down, false); // dark -> light
        assert_eq!(app.theme, Theme::Light);
        handle_picker(&mut app, KeyCode::Esc, false);
        assert!(app.picker.is_none());
        assert_eq!(app.theme, Theme::Dark);
    }

    #[test]
    fn enter_commits_previewed_theme() {
        let mut app = App::default();
        app.set_theme(Theme::Auto);
        app.picker = Some(ThemePicker::open(app.theme));
        handle_picker(&mut app, KeyCode::Down, false); // -> dark
        handle_picker(&mut app, KeyCode::Enter, false);
        assert!(app.picker.is_none());
        assert_eq!(app.theme, Theme::Dark);
    }

    #[test]
    fn ctrl_c_quits_even_in_picker() {
        let mut app = App {
            picker: Some(ThemePicker::open(Theme::Dark)),
            ..Default::default()
        };
        let ctrl_c = event::KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(handle_key(&mut app, ctrl_c, false), Action::Quit);
    }

    #[test]
    fn c_cycles_chart_only_on_dashboard() {
        let mut app = App::default();
        assert_eq!(app.tab, Tab::Dashboard);
        assert_eq!(app.chart_style, ChartStyle::Heat);
        handle_key(&mut app, press(KeyCode::Char('c')), false);
        assert_eq!(app.chart_style, ChartStyle::Bars);
        // On the Feed tab, c is inert (no off-screen surprise).
        app.tab = Tab::Feed;
        handle_key(&mut app, press(KeyCode::Char('c')), false);
        assert_eq!(app.chart_style, ChartStyle::Bars);
    }

    #[test]
    fn q_quits_outside_picker_but_closes_inside() {
        let mut app = App::default();
        app.set_theme(Theme::Dark);
        app.picker = Some(ThemePicker::open(Theme::Dark));
        assert_eq!(
            handle_key(&mut app, press(KeyCode::Char('q')), false),
            Action::None
        );
        assert!(app.picker.is_none());
        assert_eq!(
            handle_key(&mut app, press(KeyCode::Char('q')), false),
            Action::Quit
        );
    }

    #[test]
    fn tab_cycles_and_numbers_jump() {
        let mut app = App::default();
        handle_key(&mut app, press(KeyCode::Tab), false);
        assert_eq!(app.tab, Tab::Feed);
        handle_key(&mut app, press(KeyCode::Tab), false);
        assert_eq!(app.tab, Tab::Links);
        handle_key(&mut app, press(KeyCode::Tab), false);
        assert_eq!(app.tab, Tab::Dashboard); // wraps
        handle_key(&mut app, press(KeyCode::Char('2')), false);
        assert_eq!(app.tab, Tab::Feed);
    }

    #[test]
    fn esc_returns_to_dashboard_then_quits() {
        let mut app = App {
            tab: Tab::Feed,
            ..Default::default()
        };
        // First Esc leaves the list tab for the dashboard.
        assert_eq!(
            handle_key(&mut app, press(KeyCode::Esc), false),
            Action::None
        );
        assert_eq!(app.tab, Tab::Dashboard);
        // From the dashboard, Esc quits.
        assert_eq!(
            handle_key(&mut app, press(KeyCode::Esc), false),
            Action::Quit
        );
    }

    #[test]
    fn feed_scroll_moves_and_clamps() {
        let mut app = App {
            tab: Tab::Feed,
            feed: feed_with(3),
            ..Default::default()
        };
        assert_eq!(app.feed_cursor, 0);
        handle_key(&mut app, press(KeyCode::Char('j')), false);
        handle_key(&mut app, press(KeyCode::Char('j')), false);
        handle_key(&mut app, press(KeyCode::Char('j')), false); // past the end
        assert_eq!(app.feed_cursor, 2); // clamped to last
        handle_key(&mut app, press(KeyCode::Char('k')), false);
        assert_eq!(app.feed_cursor, 1);
    }

    #[test]
    fn open_returns_action_with_selected_url() {
        let mut app = App {
            tab: Tab::Feed,
            feed: feed_with(2),
            ..Default::default()
        };
        assert_eq!(
            handle_key(&mut app, press(KeyCode::Char('o')), false),
            Action::Open
        );
        // ordered() puts the newest item first; cursor 0 selects it.
        assert_eq!(app.selected_url().as_deref(), Some("https://example.com/1"));
    }

    #[test]
    fn refresh_key_requests_refresh() {
        let mut app = App::default();
        assert_eq!(
            handle_key(&mut app, press(KeyCode::Char('r')), false),
            Action::Refresh
        );
    }
}
