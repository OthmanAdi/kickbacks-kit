//! kickbacks-kit (`kb`): read-only companion tools for the kickbacks.ai
//! extension. Archive every ad you are shown and watch your stats in a TUI.
//!
//! Design principle: this crate only ever OBSERVES local artifacts the
//! extension already writes. It never posts an impression, click, or any other
//! billing event, and never talks to the kickbacks.ai backend. It records the
//! history the extension throws away — nothing more.

mod archive;
mod capture;
mod doctor;
mod export;
mod model;
mod paths;
mod render;
mod setup;
mod snapshot;
mod sources;
mod status;
mod statusline;
/// Test-only: renders a buffer to SVG for the README hero image.
#[cfg(test)]
mod svg;
mod top;
mod util;
mod watch;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::style::Stylize;
use std::path::PathBuf;

use archive::Archive;

#[derive(Parser)]
#[command(
    name = "kb",
    version,
    about = "Read-only companion tools for the kickbacks.ai extension",
    long_about = "kickbacks-kit observes the ads the kickbacks.ai extension shows you and \
                  keeps the history it discards: a searchable ad archive and a live TUI \
                  dashboard. It is strictly read-only and never reports a billing event."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Live TUI dashboard (captures while open)
    Top {
        /// Render with sample data instead of your archive (no capture, no writes)
        #[arg(long)]
        demo: bool,
    },
    /// Headless capture daemon: poll local artifacts into the archive
    Watch {
        /// Seconds between polls
        #[arg(long, default_value_t = 3)]
        interval: u64,
        /// Run a single pass and exit
        #[arg(long)]
        once: bool,
    },
    /// Inspect the local ad corpus
    Archive {
        #[command(subcommand)]
        command: ArchiveCommand,
    },
    /// Export the corpus as JSONL or CSV
    Export {
        /// Output format
        #[arg(long, value_enum, default_value_t = export::Format::Jsonl)]
        format: export::Format,
        /// Output file (default: stdout)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// One-shot dashboard render to stdout (for slash commands and scripts)
    Snapshot {
        /// Output width in columns (default: terminal width, clamped 80..=140)
        #[arg(long)]
        width: Option<u16>,
        /// Disable colors even on a terminal
        #[arg(long)]
        plain: bool,
    },
    /// One-line status for a CLI status bar: the current ad plus kb stats
    Statusline {
        /// Maximum visible columns (default: $COLUMNS or 120)
        #[arg(long)]
        width: Option<u16>,
        /// Disable colors and the ad hyperlink
        #[arg(long)]
        plain: bool,
    },
    /// Show whether ads are flowing right now, and why (killswitch, idle, etc.)
    Status,
    /// First-run setup: create the archive and capture once
    Setup,
    /// Check local data sources and the archive
    Doctor,
}

#[derive(Subcommand)]
enum ArchiveCommand {
    /// List captured ads, most recent first
    List {
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Summary statistics
    Stats,
    /// Advertiser leaderboard
    Top {
        #[arg(long, default_value_t = 15)]
        limit: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Top { demo } => {
            if demo {
                top::run_demo()
            } else {
                top::run()
            }
        }
        Command::Watch { interval, once } => watch::run(interval, once),
        Command::Archive { command } => run_archive(command),
        Command::Export { format, out } => export::run(format, out),
        Command::Snapshot { width, plain } => snapshot::run(width, plain),
        Command::Statusline { width, plain } => statusline::run(width, plain),
        Command::Status => status::run(),
        Command::Setup => setup::run(),
        Command::Doctor => doctor::run(),
    }
}

fn run_archive(command: ArchiveCommand) -> Result<()> {
    let archive = Archive::open(&paths::db_path()?)?;
    match command {
        ArchiveCommand::List { limit } => print_list(&archive, limit),
        ArchiveCommand::Stats => print_stats(&archive),
        ArchiveCommand::Top { limit } => print_leaderboard(&archive, limit),
    }
}

fn print_list(archive: &Archive, limit: usize) -> Result<()> {
    let ads = archive.list_ads(limit)?;
    if ads.is_empty() {
        println!("{}", "no ads captured yet — run `kb setup`".dim());
        return Ok(());
    }
    println!(
        "{:<18}  {:>5}  {:<16}  {}",
        "last seen".bold(),
        "seen".bold(),
        "advertiser".bold(),
        "ad".bold()
    );
    for ad in &ads {
        println!(
            "{:<18}  {:>5}  {:<16}  {}",
            util::fmt_datetime(ad.last_seen_ms),
            ad.times_seen,
            util::truncate(&ad.advertiser, 16),
            util::truncate(&ad.ad_text, 52),
        );
    }
    Ok(())
}

fn print_stats(archive: &Archive) -> Result<()> {
    let s = archive.stats(util::now_ms())?;
    let row = |label: &str, value: String| {
        println!("  {:<14}{}", label.dim(), value.to_string().bold());
    };
    println!("{}", "kickbacks-kit · archive stats".bold());
    println!();
    row("ads seen", s.distinct_ads.to_string());
    row("advertisers", s.advertisers.to_string());
    row("sightings", s.total_sightings.to_string());
    row("today", s.sightings_today.to_string());
    row("this week", s.sightings_week.to_string());
    if let (Some(first), Some(last)) = (s.first_seen_ms, s.last_seen_ms) {
        row("first seen", util::fmt_datetime(first));
        row("last seen", util::fmt_datetime(last));
    }
    Ok(())
}

fn print_leaderboard(archive: &Archive, limit: usize) -> Result<()> {
    let board = archive.advertiser_leaderboard(limit)?;
    if board.is_empty() {
        println!("{}", "no ads captured yet — run `kb setup`".dim());
        return Ok(());
    }
    println!(
        "{:>3}  {:<22}  {:>4}  {:>5}",
        "#".bold(),
        "advertiser".bold(),
        "ads".bold(),
        "seen".bold()
    );
    for (i, a) in board.iter().enumerate() {
        println!(
            "{:>3}  {:<22}  {:>4}  {:>5}",
            i + 1,
            util::truncate(&a.advertiser, 22),
            a.distinct_ads,
            a.sightings,
        );
    }
    Ok(())
}
