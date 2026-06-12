//! `kb top` — the live, interactive dashboard. The actual drawing lives in
//! [`crate::render`]; this module owns only the terminal lifecycle and the
//! event loop. It refreshes once a second and captures on every tick, so
//! simply leaving it open grows your archive.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::archive::Archive;
use crate::capture::capture_pass;
use crate::paths;
use crate::render::{demo_app, ui, App};

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
