<div align="center">

# kickbacks-kit

**A read-only companion for the [kickbacks.ai](https://kickbacks.ai) extension.**
Archive every ad you are shown, and watch your stats in a live terminal dashboard.

Track the sponsored ads kickbacks.ai shows in your Claude Code and Codex spinner:
a local ad archive plus a Rust TUI dashboard for impressions, advertisers, and earnings history.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-14%20passing-brightgreen.svg)](#testing)
[![Read only](https://img.shields.io/badge/read--only-never%20bills-success.svg)](#what-this-is-and-is-not)

<img src="media/kbtop.svg" alt="kickbacks-kit kbtop terminal dashboard for kickbacks.ai showing the current ad, total ads seen, a 24 hour sightings sparkline, the advertiser leaderboard, and recent ads in Claude Code" width="820">

</div>

---

The kickbacks.ai extension shows sponsored messages in the Claude Code and Codex
spinners and splits the revenue with you. It keeps almost no history: the current
ad lives in one file, and earnings sit in memory until the next API poll.

`kickbacks-kit` keeps the history the extension discards. It watches the local
files the extension already writes, records every ad into a small SQLite archive,
and renders the whole picture in a terminal UI. Nothing leaves your machine.

> The dashboard above runs on `kb top --demo` with sample data. Your real numbers
> fill in as you code with the extension running.

## What this is, and is not

This tool **observes**. It reads the files the extension writes for its own status
line and logging, and it stores what it finds. That is the entire scope.

It does **not**, ever:

* post an impression, a view, a click, or any other billing event,
* talk to the kickbacks.ai backend,
* keep a session visible to inflate view time,
* do anything to earn credit you did not earn by genuinely using the extension.

The advertisers pay for real attention, and the split to you rests on that. This
project records your history. It does not manufacture it. If you want more (and
better) ads, the honest lever is to do real work with the extension running.

`kickbacks-kit` is an independent community tool. It is not affiliated with
kickbacks.ai or ShiftKeys, Inc.

## Install

From source (requires a [Rust toolchain](https://rustup.rs/)):

```bash
cargo install --git https://github.com/OthmanAdi/kickbacks-kit
```

Or clone and build:

```bash
git clone https://github.com/OthmanAdi/kickbacks-kit
cd kickbacks-kit
cargo build --release
# binary at target/release/kb
```

## Quickstart

```bash
kb top --demo # see the dashboard right now, with sample data
kb setup      # create the archive and capture once
kb status     # are ads flowing right now, and if not, why
kb doctor     # confirm the extension files and archive are wired up
kb top        # live dashboard (also captures while open)
kb watch      # headless capture in a spare terminal
```

Leave `kb top` or `kb watch` running while you code. Each new ad rotation is
recorded once. Polling fast never double counts.

## Commands

| Command | What it does |
| :------ | :----------- |
| `kb top` | Live TUI dashboard. Captures on every tick. |
| `kb top --demo` | Render the dashboard with sample data. No archive, no writes. |
| `kb status` | Whether ads are flowing now, and why not (killswitch, idle, signed out). |
| `kb watch [--interval N] [--once]` | Headless capture loop. Default poll is 3 seconds. |
| `kb archive stats` | Summary: ads seen, advertisers, sightings, today, this week. |
| `kb archive list [--limit N]` | Captured ads, most recent first. |
| `kb archive top [--limit N]` | Advertiser leaderboard by sightings. |
| `kb export [--format jsonl\|csv] [--out FILE]` | Dump the corpus. JSONL loads straight into `datasets`. |
| `kb setup` | First-run: create the archive, capture once. |
| `kb doctor` | Check the local data sources and the archive. |

## How it works

The kickbacks.ai extension writes a handful of local files. `kickbacks-kit` reads
them and nothing else:

| Source | Provides | Used for |
| :----- | :------- | :------- |
| `~/.vibe-ads/cli-ad.json` | The current ad: text, click URL, icon, rotation timestamp. | Capturing each ad into the archive. |
| `~/.vibe-ads/debug.log` | Lifecycle events: signed in, ads on, killswitch, Claude Code version. | The status header and session state. |

Each ad is keyed by a hash of its click URL and text, so repeated rotations of the
same creative collapse to one row with a `times_seen` count. A rotation is recorded
once, by its timestamp, which makes capture idempotent.

## Your data

The archive is a single SQLite file under your platform data directory:

* Windows: `%APPDATA%\kickbacks-kit\kickbacks.db`
* macOS: `~/Library/Application Support/kickbacks-kit/kickbacks.db`
* Linux: `~/.local/share/kickbacks-kit/kickbacks.db`

It never leaves your machine. `kb export` hands it back to you as JSONL or CSV
whenever you want it. Override the location with `KICKBACKS_KIT_DB`, and point the
reader at a different artifact directory with `KICKBACKS_VIBE_DIR`.

## Status and the killswitch

kickbacks.ai can pause ads server-side with a killswitch. When that happens, the
extension keeps running but no ads show, which looks like something on your end
broke. `kb status` reads the extension's own last reported state and tells you
plainly:

```
ads  ● PAUSED (kickbacks killswitch active)
```

`kb top` shows the same thing as a red "ADS PAUSED" banner. The state comes from
the extension log, so reload the VS Code window if you want it refreshed.

kickbacks.ai does not publish a status page. The maintainer posts outages and
"we're back" notes on X: [@andrewmccalip](https://x.com/andrewmccalip). That, plus
`kb status`, is the most honest read available.

## FAQ

**Why did ads suddenly stop showing?**
Usually the kickbacks.ai killswitch, which pauses ads on their side (for example
during a backend outage). Run `kb status`: if it says PAUSED, it is not your setup
and you cannot override it. If it says IDLE, just start a Claude Code task so a
spinner runs.

**What is kickbacks-kit?**
A small Rust command line tool that archives the ads the kickbacks.ai extension
shows in your Claude Code and Codex spinner, and displays them in a terminal
dashboard. Two pieces: a local ad archive (`kb archive`) and a live TUI (`kb top`).

**Does it earn me more money or boost my kickbacks earnings?**
No, and that is deliberate. It is read-only. It never posts an impression, view,
or click, and never contacts the kickbacks.ai backend. The honest way to earn more
is to do real work with the extension running. This tool just keeps the record.

**Do I need to be signed in to kickbacks.ai?**
No. It reads local files, so it works whether you are signed in or not. Sign-in
only matters for the earnings the extension itself accrues.

**Where is my data stored, and is anything uploaded?**
In one SQLite file on your machine (see "Your data"). Nothing is uploaded. `kb export`
gives you the whole corpus as JSONL or CSV.

**Does it work with Codex as well as Claude Code?**
The archive captures any ad written to the shared CLI file. Codex-specific surface
support is on the roadmap.

## Roadmap

* Loopback read of live earnings (no token rotation, read only).
* Anonymized, opt-in public dataset of spinner ad creatives.
* Codex surface support alongside Claude Code.
* Per-advertiser detail view in the dashboard.

## Testing

```bash
cargo test       # unit tests plus TUI render snapshots
cargo clippy --all-targets -- -D warnings
```

## Contributing

Issues and pull requests are welcome. Contributors are credited in the CHANGELOG
and `CONTRIBUTORS.md`, not in commit trailers. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).

Built by [Ahmad-Othman Adi](https://github.com/OthmanAdi) (CodingWithAdi).
