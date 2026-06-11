# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2]

### Fixed
* Advertiser names now parse creatives that use a hyphen or dash separator
  (for example "Solo - run your agents"), not just the middot, and fall back to
  the click host only when no short brand head is present.
* The advertiser label is refreshed when an ad is seen again, so earlier rows
  heal to the better name on the next rotation.

## [0.1.1]

### Added
* `kb top --demo`: render the dashboard with sample data, labelled "demo data",
  touching no archive and writing nothing.
* README hero image of the dashboard, generated as a standalone SVG from the
  demo render by an ignored asset test (`generate_readme_svg`).
* FAQ section and search-friendly description, headings, and repo topics.

## [0.1.0]

### Added
* `kb top`: live terminal dashboard with now playing, lifetime totals, a 24 hour
  sightings sparkline, the advertiser leaderboard, and recent ads.
* `kb watch`: headless capture daemon with a configurable poll interval.
* `kb archive` subcommands: `stats`, `list`, `top`.
* `kb export`: dump the corpus as JSONL or CSV, to stdout or a file.
* `kb setup` and `kb doctor` helpers for first run and diagnostics.
* SQLite archive with idempotent ad capture keyed by rotation timestamp.
* Read-only by design: no billing event is ever emitted.
