//! Parsers and item builders for the public GitHub REST API.
//!
//! These are pure: they take a JSON string (as fetched by [`super::http`]) and
//! return feed items or stats. No network here, so they are exhaustively unit
//! tested against canned fixtures. serde structs use `#[serde(default)]` on
//! every field so a shape change upstream degrades to a missing value rather
//! than a hard parse error.

use serde::Deserialize;

use super::{FeedItem, FeedKind};

/// Owner/repo of the upstream product, and our own repo, as feed targets.
pub const UPSTREAM_REPO: &str = "andrewmccalip/kickbacks.ai";
pub const SELF_REPO: &str = "OthmanAdi/kickbacks-kit";

/// Build the API URL for a repo's metadata.
pub fn repo_url(repo: &str) -> String {
    format!("https://api.github.com/repos/{repo}")
}

/// Build the API URL for a repo's recent commits.
pub fn commits_url(repo: &str) -> String {
    format!("https://api.github.com/repos/{repo}/commits?per_page=5")
}

/// Build the API URL for a repo's recently updated issues (includes PRs, which
/// we filter out).
pub fn issues_url(repo: &str) -> String {
    format!("https://api.github.com/repos/{repo}/issues?state=open&sort=created&direction=desc&per_page=20")
}

#[derive(Debug, Default, Deserialize)]
struct Repo {
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    forks_count: u64,
    #[serde(default)]
    open_issues_count: u64,
    #[serde(default)]
    pushed_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

/// Distilled repository statistics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStats {
    pub stars: u64,
    pub forks: u64,
    /// GitHub's `open_issues_count` counts PRs too; treated as "open items".
    pub open_items: u64,
    pub pushed_at_ms: Option<i64>,
    pub html_url: Option<String>,
}

/// Parse a `/repos/{owner}/{repo}` response.
pub fn parse_repo(json: &str) -> Option<RepoStats> {
    let r: Repo = serde_json::from_str(json).ok()?;
    Some(RepoStats {
        stars: r.stargazers_count,
        forks: r.forks_count,
        open_items: r.open_issues_count,
        pushed_at_ms: r.pushed_at.as_deref().and_then(iso_to_ms),
        html_url: r.html_url,
    })
}

/// Build the repo-stats feed item for the upstream product repo.
pub fn upstream_stat_item(stats: &RepoStats) -> FeedItem {
    let body = format!(
        "{} stars · {} forks · {} open items",
        stats.stars, stats.forks, stats.open_items
    );
    FeedItem::new(
        FeedKind::Stat,
        "kickbacks.ai on GitHub",
        body,
        stats
            .html_url
            .clone()
            .or_else(|| Some(format!("https://github.com/{UPSTREAM_REPO}"))),
        stats.pushed_at_ms,
        "github",
    )
}

#[derive(Debug, Default, Deserialize)]
struct CommitItem {
    #[serde(default)]
    commit: CommitInner,
}

#[derive(Debug, Default, Deserialize)]
struct CommitInner {
    #[serde(default)]
    message: String,
    #[serde(default)]
    author: CommitAuthor,
}

#[derive(Debug, Default, Deserialize)]
struct CommitAuthor {
    #[serde(default)]
    date: Option<String>,
}

/// The latest synced extension version, parsed from the upstream commit log.
/// The repo publishes no releases or tags; its only version signal is the
/// commit message `Sync extension vX.Y.Z (sha)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionSync {
    pub version: String,
    pub ts_ms: Option<i64>,
}

/// Find the newest `Sync extension vX.Y.Z` across recent commits.
pub fn parse_latest_version(json: &str) -> Option<VersionSync> {
    let commits: Vec<CommitItem> = serde_json::from_str(json).ok()?;
    for c in commits {
        if let Some(version) = extract_sync_version(&c.commit.message) {
            return Some(VersionSync {
                version,
                ts_ms: c.commit.author.date.as_deref().and_then(iso_to_ms),
            });
        }
    }
    None
}

/// Pull `0.3.172` out of `Sync extension v0.3.172 (cbd05e1)`. Returns `None`
/// when the message is not a version sync, so a changed commit convention fails
/// soft (no version item) rather than surfacing garbage.
fn extract_sync_version(message: &str) -> Option<String> {
    let after = message.split("Sync extension v").nth(1)?;
    let version: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    // Require a dotted triple so a stray "v1" cannot masquerade as a version.
    if version.split('.').filter(|p| !p.is_empty()).count() >= 2 && version.contains('.') {
        Some(version)
    } else {
        None
    }
}

/// Build the version feed item.
pub fn version_item(v: &VersionSync) -> FeedItem {
    FeedItem::new(
        FeedKind::Version,
        format!("Extension synced to v{}", v.version),
        "kickbacks.ai VS Code extension",
        Some(format!("https://github.com/{UPSTREAM_REPO}/commits/main")),
        v.ts_ms,
        "github",
    )
}

#[derive(Debug, Default, Deserialize)]
struct Issue {
    #[serde(default)]
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    /// Present only on pull requests; its presence means "this is a PR".
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

/// Parse the issues endpoint into feed items, dropping pull requests and
/// keeping at most `limit` real issues, newest first.
pub fn parse_issues(json: &str, limit: usize) -> Vec<FeedItem> {
    let issues: Vec<Issue> = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    issues
        .into_iter()
        .filter(|i| i.pull_request.is_none() && !i.title.is_empty())
        .take(limit)
        .map(|i| {
            let ts = i
                .created_at
                .as_deref()
                .or(i.updated_at.as_deref())
                .and_then(iso_to_ms);
            FeedItem::new(
                FeedKind::Issue,
                format!("#{} {}", i.number, i.title),
                "open issue on kickbacks.ai",
                i.html_url.clone().or_else(|| {
                    Some(format!(
                        "https://github.com/{UPSTREAM_REPO}/issues/{}",
                        i.number
                    ))
                }),
                ts,
                "github",
            )
        })
        .collect()
}

/// RFC 3339 / ISO 8601 timestamp to epoch millis.
fn iso_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO_JSON: &str = r#"{
        "stargazers_count": 158, "forks_count": 33, "open_issues_count": 45,
        "pushed_at": "2026-06-12T05:37:39Z",
        "html_url": "https://github.com/andrewmccalip/kickbacks.ai"
    }"#;

    #[test]
    fn parses_repo_stats() {
        let s = parse_repo(REPO_JSON).unwrap();
        assert_eq!(s.stars, 158);
        assert_eq!(s.forks, 33);
        assert_eq!(s.open_items, 45);
        assert!(s.pushed_at_ms.unwrap() > 0);
        let item = upstream_stat_item(&s);
        assert!(item.body.contains("158 stars"));
        assert_eq!(item.kind, FeedKind::Stat);
    }

    #[test]
    fn parse_repo_tolerates_missing_fields() {
        let s = parse_repo(r#"{"stargazers_count": 5}"#).unwrap();
        assert_eq!(s.stars, 5);
        assert_eq!(s.forks, 0);
        assert_eq!(s.pushed_at_ms, None);
    }

    const COMMITS_JSON: &str = r#"[
        {"commit":{"message":"Sync extension v0.3.172 (cbd05e1)","author":{"date":"2026-06-12T05:37:38Z"}}},
        {"commit":{"message":"Sync extension v0.3.171 (3696683)","author":{"date":"2026-06-11T15:21:01Z"}}}
    ]"#;

    #[test]
    fn parses_latest_version_from_commits() {
        let v = parse_latest_version(COMMITS_JSON).unwrap();
        assert_eq!(v.version, "0.3.172");
        assert!(v.ts_ms.unwrap() > 0);
        let item = version_item(&v);
        assert!(item.title.contains("v0.3.172"));
    }

    #[test]
    fn version_parser_rejects_non_sync_commits() {
        assert_eq!(extract_sync_version("Initial commit"), None);
        assert_eq!(extract_sync_version("Sync extension v1 (abc)"), None);
        assert_eq!(
            extract_sync_version("Sync extension v0.3.174 (deadbee)").as_deref(),
            Some("0.3.174")
        );
        // No version commit anywhere -> None, not a panic.
        assert!(parse_latest_version(r#"[{"commit":{"message":"docs"}}]"#).is_none());
    }

    const ISSUES_JSON: &str = r#"[
        {"number":52,"title":"Displays ads but no kickback earned","html_url":"https://github.com/andrewmccalip/kickbacks.ai/issues/52","created_at":"2026-06-12T18:23:27Z"},
        {"number":51,"title":"A pull request","html_url":"https://github.com/x","created_at":"2026-06-12T18:00:00Z","pull_request":{"url":"x"}},
        {"number":50,"title":"Stablecoin payouts","created_at":"2026-06-12T17:00:00Z"}
    ]"#;

    #[test]
    fn parses_issues_and_drops_pulls() {
        let items = parse_issues(ISSUES_JSON, 10);
        assert_eq!(items.len(), 2); // the PR is dropped
        assert!(items[0].title.starts_with("#52"));
        assert!(items[0].url.as_deref().unwrap().contains("/issues/52"));
        // Issue without html_url still gets a synthesized link.
        assert!(items[1].url.as_deref().unwrap().contains("/issues/50"));
    }

    #[test]
    fn parse_issues_handles_garbage() {
        assert!(parse_issues("not json", 10).is_empty());
        assert!(parse_issues("{}", 10).is_empty());
    }

    #[test]
    fn issue_limit_is_respected() {
        assert_eq!(parse_issues(ISSUES_JSON, 1).len(), 1);
    }
}
