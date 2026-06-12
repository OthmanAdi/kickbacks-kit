<div align="center">

# kickbacks-kit — ad archive and live dashboard for kickbacks.ai

**A read-only companion for the [kickbacks.ai](https://kickbacks.ai) extension.**
Archive every ad you are shown, track the kickbacks.ai status, and watch it all
in a terminal dashboard.

kickbacks-kit records the sponsored ads kickbacks.ai shows in your Claude Code
and Codex spinner into a local SQLite archive, and renders them in a Rust TUI:
ad sightings, advertisers, 24 hour activity, plus a live status feed for the
kickbacks.ai service bulletin and the upstream project's GitHub activity.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-141%20passing-brightgreen.svg)](#testing)
[![Read only](https://img.shields.io/badge/read--only-never%20bills-success.svg)](#what-this-is-and-is-not)

<img src="media/kbtop.svg" alt="kickbacks-kit kbtop terminal dashboard for kickbacks.ai showing the current ad, total ads seen, a 24 hour sightings sparkline, the advertiser leaderboard, and recent ads in Claude Code" width="820">

</div>

---

The kickbacks.ai extension shows sponsored messages in the Claude Code and Codex
spinners and splits the revenue with you. It keeps almost no history: the current
ad lives in one file, and earnings sit in memory until the next API poll.

`kickbacks-kit` keeps the history the extension discards. It watches the local
files the extension already writes, records every ad into a small SQLite archive,
and renders the whole picture in a terminal UI. Your archive stays on your
machine and is never uploaded. The one piece that uses the network is an optional
status feed, which makes read-only requests to public endpoints (the GitHub API
and the public kickbacks.ai status bulletin) and can be turned off. See
[The status feed](#the-status-feed).

> The dashboard above shows sample data, for the screenshot. Your real numbers
> fill in as you code with the extension running.

## What this is, and is not

This tool **observes**. It reads the local files the extension writes for its own
status line and logging, stores what it finds, and adds a read-only status feed
of public information. That is the entire scope.

It does **not**, ever:

* post an impression, a view, a click, or any other billing event,
* call a kickbacks.ai billing, authentication, or earnings endpoint,
* read or show your balance (that needs the cloud backend; it links you to your
  portfolio instead),
* keep a session visible to inflate view time,
* do anything to earn credit you did not earn by genuinely using the extension.

The status feed reads two public, read-only sources: the GitHub REST API for the
upstream project, and the public kickbacks.ai status bulletin that the homepage
itself fetches. It is on by default and can be turned off entirely (see
[The status feed](#the-status-feed)).

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
kb setup      # create the archive and capture once
kb status     # are ads flowing right now, and if not, why
kb doctor     # confirm the extension files and archive are wired up
kb top        # live dashboard with Dashboard, Feed, and Links tabs
kb feed       # the kickbacks.ai status feed and upstream activity
kb snapshot   # one-shot dashboard render to stdout
kb report     # a digest of your archive as Markdown or HTML
kb watch      # headless capture in a spare terminal
```

Leave `kb top` or `kb watch` running while you code. Each new ad rotation is
recorded once. Polling fast never double counts.

### Inside Claude Code

```bash
kb install-claude   # add /kbtop and /kbstatus, and wire the status line
```

This installs four global slash commands (`/kbtop`, `/kbstatus`, `/kbfeed`,
`/kblinks`) and points Claude Code's status line at `kb statusline`, a single
line that keeps the kickbacks ad and appends your own stats. When a status feed
alert is present (a service bulletin, or a newer extension version), it adds that
too, on the right, where there is room:

```
ad· Ramp - business cards that close themselves  ·  kb 12 today · 17 advs · ● live  ·  ▲ A bot army is attacking us
```

An interactive TUI cannot run inside Claude Code's own pane, so `/kbtop` and
`/kbfeed` print one-shot snapshots (`kb snapshot`); run `kb top` in a separate
terminal for the live version. If the kickbacks extension already owns the status
line, the installer wraps it rather than replacing it, and keeps a backup. Undo
everything with `kb uninstall-claude`.

## Commands

| Command | What it does |
| :------ | :----------- |
| `kb top [--theme T] [--chart-style C]` | Live TUI dashboard with Dashboard, Feed, and Links tabs. Captures on every tick. `1`/`2`/`3` or `Tab` switch, `t` theme, `c` chart, `r` refresh. |
| `kb feed [--offline] [--json]` | The kickbacks.ai status bulletin and the upstream project's GitHub activity, from the local cache (refreshed when online). |
| `kb snapshot [--tab T] [--width N] [--plain] [--theme T] [--chart-style C]` | One-shot render of any tab (`dashboard`, `feed`, `links`) to stdout. |
| `kb links [--format table\|csv\|jsonl\|html] [--out FILE]` | Export the advertiser links you have been shown, including a self-contained HTML bookmarks page. |
| `kb report [--format md\|html] [--out FILE]` | A digest of your archive: stats, advertisers, links, and the killswitch timeline. |
| `kb statusline [--width N] [--plain] [--no-feed]` | One status-bar line: the current ad, your kb stats, and a feed alert when there is room. |
| `kb status` | Whether ads are flowing now, and why not (killswitch, idle, signed out). |
| `kb watch [--interval N] [--once]` | Headless capture loop. Default poll is 3 seconds. |
| `kb archive stats` | Summary: ads seen, advertisers, sightings, today, this week. |
| `kb archive list [--limit N]` | Captured ads, most recent first. |
| `kb archive top [--limit N]` | Advertiser leaderboard by sightings. |
| `kb export [--format jsonl\|csv] [--out FILE]` | Dump the corpus. JSONL loads straight into `datasets`. |
| `kb setup` | First-run: create the archive, capture once. |
| `kb doctor` | Check the local data sources and the archive. |
| `kb install-claude` / `kb uninstall-claude` | Add or remove the `/kbtop`, `/kbstatus`, `/kbfeed`, and `/kblinks` commands and the status line. |

## Themes

The dashboard reads on a dark or a light terminal. Pick a theme with `--theme`
on `kb top` or `kb snapshot`, or press `t` inside `kb top` for a live picker
(arrow keys preview, Enter saves, Esc reverts). The choice is remembered.

| Theme | What you get |
| :---- | :----------- |
| `auto` | Detect a light or dark terminal and match it. Falls back to dark when the terminal does not report its background. This is the default. |
| `dark` | A dark canvas, the same look as the screenshot above, on any terminal. |
| `light` | A light canvas tuned for bright terminals. Every color clears WCAG AA contrast. |
| `terminal` | No painted canvas: use your terminal's own colors and background. |

Auto-detection uses the `COLORFGBG` variable that many terminals export. When it
is absent (some terminals, including Windows Terminal, do not set it), `auto`
stays on the dark canvas, which is readable on a light background too. Pick
`light` or `terminal` explicitly, or use the picker, to override. Your selection
is saved to `<config-dir>/kickbacks-kit/config.json` (override with
`KICKBACKS_KIT_CONFIG`).

The sightings chart has two styles, saved in the same config. `heat` (the
default) is a calendar strip, one cell per hour colored by how busy it was, and
stays readable whether one hour or all twenty four have data. `bars` draws block
bars on a baseline floor when you want to read magnitude as height. Press `c` in
`kb top` to switch, or pass `--chart-style heat|bars`.

## The status feed

kickbacks.ai has no status page, so when ads stop it is hard to tell an outage
from a problem on your end. The status feed answers that in the terminal. `kb top`
has a **Feed** tab, and `kb feed` prints the same thing once:

* the kickbacks.ai **service bulletin**, the maintainer's de facto status channel
  (for example a killswitch or a payout note),
* the **extension version** that was last synced upstream, and a marker on the
  status line when it is newer than the one you have installed,
* the upstream repository's **stars and open issues**, and recent issues,
* a link to the maintainer's **X** channel.

On the Feed and Links tabs, `j` and `k` move the selection and `o` opens the
highlighted link in your browser. `r` refreshes now.

**The network, plainly.** The feed is the only part of kb that makes a network
request, and only read-only GET requests to two public endpoints: the GitHub
REST API for the upstream repository, and the public kickbacks.ai status bulletin
(`/api/bulletin`, the same document the homepage fetches client-side). It sends
no credentials, posts nothing, and never touches a billing, authentication, or
earnings route. Everything is cached locally, so the status line and snapshots
render the feed without any network at all.

It is on by default. Turn it off three ways:

| How | Effect |
| :-- | :----- |
| `kb feed --offline`, `kb top` reads cache only | This run makes no request. |
| `"feed": { "enabled": false }` in the config | The feed never fetches. |
| `KICKBACKS_KIT_OFFLINE=1` in the environment | A global kill switch for any invocation. |

The Feed view always shows its network state in a footer: when it last synced,
which sources are healthy, or that it is offline and showing the cache.

## Your links and reports

`kb links` exports the advertiser destinations you have been shown, as an aligned
table, CSV, JSONL, or a self-contained HTML bookmarks page you can open in a
browser:

```bash
kb links --format html --out advertisers.html
```

`kb report` writes a digest of your archive (the summary, the advertiser
leaderboard, the recent links, and the killswitch timeline) as Markdown or HTML:

```bash
kb report --format md --out kickbacks-report.md
```

Both are read-only, like everything else. The report never reads or invents an
earnings figure; it points at your portfolio for that.

## How it works

The kickbacks.ai extension writes a handful of local files. `kickbacks-kit` reads
them and nothing else for the archive:

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
or click, and never calls a kickbacks.ai billing, authentication, or earnings
endpoint. The honest way to earn more is to do real work with the extension
running. This tool just keeps the record.

**Can kickbacks-kit farm impressions or inflate my earnings?**
No, and it could not if it wanted to. It is architecturally read-only: it watches
local files and reads public status information, and it has no code path that
posts an impression, a view, a click, or any billing event. The advertisers pay
for real attention; this tool records what you were shown, it does not
manufacture it.

**Is kickbacks.ai safe to install?**
That is your call to make, not this project's, but a few facts help. The
extension is source-available, and its stated telemetry is ad-event metadata, not
your code or prompts. Reasonable cautions have been raised in its issue tracker
(an out-of-band auto-updater, for instance). kickbacks-kit itself is independent,
MIT licensed, and read-only: it does not change the extension or send your data
anywhere. Read both before deciding.

**What is the difference between kickbacks.ai and aikickbacks.com?**
They are different products. kickbacks-kit is a companion for **kickbacks.ai**
(Andrew McCalip / ShiftKeys), the extension that puts a sponsored line in your
Claude Code and Codex spinner. It is not affiliated with aikickbacks.com.

**Does kickbacks-kit read my code or prompts?**
No. It reads two local files the extension writes (the current ad and a lifecycle
log), and, for the status feed, two public web endpoints. It never reads your
source, your prompts, or your AI responses.

**Where do I see my actual earnings?**
On your kickbacks portfolio at [kickbacks.ai/me](https://kickbacks.ai/me). kb does
not read your balance: that needs the cloud backend, and kb stays read-only. The
dashboard and `kb status` link you there rather than showing a number they cannot
verify.

**Do I need to be signed in to kickbacks.ai?**
No. It reads local files, so it works whether you are signed in or not. Sign-in
only matters for the earnings the extension itself accrues.

**Where is my data stored, and is anything uploaded?**
In one SQLite file on your machine (see "Your data"). Nothing is uploaded. The
status feed downloads public information; it never sends your data out. `kb export`
gives you the whole corpus as JSONL or CSV.

**Does it work with Codex as well as Claude Code?**
The archive captures any ad written to the shared CLI file. Codex-specific surface
support is on the roadmap.

## Roadmap

* Anonymized, opt-in public dataset of spinner ad creatives.
* Codex surface support alongside Claude Code.

Reading live earnings was considered and dropped: it would require the
kickbacks.ai cloud backend, which crosses the read-only line this tool is built
on. Earnings stay where they belong, on your [portfolio](https://kickbacks.ai/me).

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
