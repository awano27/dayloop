//! GitHub issues and pull requests. No token means unknown, never done.

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::observe::Sight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub kind: &'static str,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

pub fn token() -> Option<String> {
    env_token().or_else(gh_cli_token)
}

fn env_token() -> Option<String> {
    for key in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Follows the `github:credential` edge. `skip` reads nothing. `env` ignores `gh`.
pub fn credential(store: &crate::store::Store) -> Option<String> {
    let mode = crate::graph::find_step(store, "github:credential")
        .ok()
        .flatten()
        .map(|edge| edge.to_choice);
    match mode.as_deref() {
        Some("skip") => None,
        Some("env") => env_token(),
        _ => env_token().or_else(gh_cli_token),
    }
}

fn gh_cli_token() -> Option<String> {
    if cfg!(test) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let name = exe.file_stem()?.to_string_lossy();
    if !name.eq_ignore_ascii_case("dayloop") {
        return None;
    }
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?;
    let token = token.trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

/// `github:pr:owner/repo#12` or `github:issue:owner/repo#12`.
pub fn parse_ref(source_ref: &str) -> Option<Ref> {
    let (kind, rest) = if let Some(rest) = source_ref.strip_prefix("github:pr:") {
        ("pr", rest)
    } else if let Some(rest) = source_ref.strip_prefix("github:issue:") {
        ("issue", rest)
    } else {
        return None;
    };
    let (slug, number) = rest.rsplit_once('#')?;
    let (owner, repo) = slug.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    let number = number.parse().ok()?;
    Some(Ref {
        kind,
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
    })
}

pub fn sight_from_body(kind: &str, body: &str) -> Result<Sight> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let state = value
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("open");
    if kind == "pr" {
        let merged = value.get("merged").and_then(|v| v.as_bool()).unwrap_or(false);
        return Ok(if merged { Sight::Closed } else { Sight::Open });
    }
    Ok(if state == "closed" {
        Sight::Closed
    } else {
        Sight::Open
    })
}

pub fn fetch_sight(source_ref: &str) -> Option<Sight> {
    fetch_sight_with(source_ref, &token()?)
}

pub fn fetch_sight_with(source_ref: &str, token: &str) -> Option<Sight> {
    let parsed = parse_ref(source_ref)?;
    let token = token.to_string();
    let path = if parsed.kind == "pr" { "pulls" } else { "issues" };
    let url = format!(
        "https://api.github.com/repos/{}/{}/{path}/{}",
        parsed.owner, parsed.repo, parsed.number
    );
    let body = get(&url, &token).ok()?;
    sight_from_body(parsed.kind, &body).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub source_ref: String,
    pub open: bool,
}

pub fn parse_assigned(body: &str) -> Result<Vec<Item>> {
    let rows: Vec<ApiItem> = serde_json::from_str(body)?;
    let mut out = Vec::new();
    for row in rows {
        let Some((owner, repo)) = repo_of(&row.repository_url) else {
            continue;
        };
        if row.title.trim().is_empty() {
            continue;
        }
        let kind = if row.pull_request.is_some() { "pr" } else { "issue" };
        out.push(Item {
            title: row.title.trim().to_string(),
            source_ref: format!("github:{kind}:{owner}/{repo}#{}", row.number),
            open: row.state == "open",
        });
    }
    Ok(out)
}

pub fn fetch_assigned() -> Result<Vec<Item>> {
    let token = token().ok_or_else(|| anyhow!("GITHUB_TOKEN がありません"))?;
    assigned_with(&token)
}

pub fn assigned_with(token: &str) -> Result<Vec<Item>> {
    let body = get(
        "https://api.github.com/issues?filter=assigned&state=open&per_page=50",
        token,
    )?;
    parse_assigned(&body)
}

#[derive(Deserialize)]
struct ApiItem {
    title: String,
    number: u64,
    state: String,
    repository_url: String,
    pull_request: Option<serde_json::Value>,
}

fn repo_of(repository_url: &str) -> Option<(&str, &str)> {
    let rest = repository_url
        .strip_prefix("https://api.github.com/repos/")
        .or_else(|| repository_url.strip_prefix("http://api.github.com/repos/"))?;
    let (owner, repo) = rest.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner, repo))
}

fn get(url: &str, token: &str) -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let response = agent
        .get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", "dayloop")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| anyhow!("GitHub に聞けません: {e}"))?;
    Ok(response.into_string()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owner_repo_number() {
        let pr = parse_ref("github:pr:awano27/dayloop#7").unwrap();
        assert_eq!(pr.kind, "pr");
        assert_eq!(pr.number, 7);
        assert!(parse_ref("github:pr:dayloop#7").is_none());
    }

    #[test]
    fn merged_pr_is_closed_and_unmerged_is_not() {
        assert_eq!(
            sight_from_body("pr", r#"{"state":"closed","merged":true}"#).unwrap(),
            Sight::Closed
        );
        assert_eq!(
            sight_from_body("pr", r#"{"state":"closed","merged":false}"#).unwrap(),
            Sight::Open
        );
        assert_eq!(
            sight_from_body("issue", r#"{"state":"closed"}"#).unwrap(),
            Sight::Closed
        );
    }

    #[test]
    fn assigned_payload_becomes_source_refs() {
        let body = r#"[{
            "title":"直す",
            "number":4,
            "state":"open",
            "repository_url":"https://api.github.com/repos/awano27/dayloop",
            "pull_request":{"url":"https://api.github.com/repos/awano27/dayloop/pulls/4"}
        },{
            "title":"書く",
            "number":5,
            "state":"open",
            "repository_url":"https://api.github.com/repos/awano27/dayloop"
        }]"#;
        let items = parse_assigned(body).unwrap();
        assert_eq!(items[0].source_ref, "github:pr:awano27/dayloop#4");
        assert_eq!(items[1].source_ref, "github:issue:awano27/dayloop#5");
    }
}
