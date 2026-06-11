# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
* `kb top`: live terminal dashboard with now playing, lifetime totals, a 24 hour
  sightings sparkline, the advertiser leaderboard, and recent ads.
* `kb watch`: headless capture daemon with a configurable poll interval.
* `kb archive` subcommands: `stats`, `list`, `top`.
* `kb export`: dump the corpus as JSONL or CSV, to stdout or a file.
* `kb setup` and `kb doctor` helpers for first run and diagnostics.
* SQLite archive with idempotent ad capture keyed by rotation timestamp.
* Read-only by design: no billing event is ever emitted.
