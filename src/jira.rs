//! Jira issues assigned to the current user. Missing credentials mean unknown, never done.

use anyhow::{anyhow, Result};

use crate::observe::Sight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub source_ref: String,
    pub open: bool,
}

pub fn creds() -> Option<(String, String, String)> {
    let base = std::env::var("JIRA_BASE_URL").ok()?;
    let email = std::env::var("JIRA_EMAIL").ok()?;
    let token = std::env::var("JIRA_API_TOKEN").ok()?;
    let base = base.trim().trim_end_matches('/').to_string();
    let email = email.trim().to_string();
    let token = token.trim().to_string();
    if base.is_empty() || email.is_empty() || token.is_empty() {
        return None;
    }
    Some((base, email, token))
}

pub fn parse_key(source_ref: &str) -> Option<&str> {
    let key = source_ref.strip_prefix("jira:")?;
    if key.is_empty() || key.contains('/') || key.contains(' ') {
        return None;
    }
    Some(key)
}

pub fn parse_search(body: &str) -> Result<Vec<Item>> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let issues = value.get("issues").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for issue in issues {
        let Some(key) = issue.get("key").and_then(|v| v.as_str()) else {
            continue;
        };
        let fields = issue.get("fields");
        let title = fields
            .and_then(|f| f.get("summary"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if title.is_empty() {
            continue;
        }
        let category = fields
            .and_then(|f| f.get("status"))
            .and_then(|s| s.get("statusCategory"))
            .and_then(|c| c.get("key"))
            .and_then(|v| v.as_str())
            .unwrap_or("new");
        out.push(Item {
            title: title.to_string(),
            source_ref: format!("jira:{key}"),
            open: category != "done",
        });
    }
    Ok(out)
}

pub fn sight_from_issue(body: &str) -> Result<Sight> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let category = value
        .pointer("/fields/status/statusCategory/key")
        .and_then(|v| v.as_str())
        .unwrap_or("new");
    Ok(if category == "done" {
        Sight::Closed
    } else {
        Sight::Open
    })
}

pub fn fetch_assigned() -> Result<Vec<Item>> {
    let (base, email, token) = creds().ok_or_else(|| anyhow!("Jira の接続情報がありません"))?;
    let url = format!(
        "{base}/rest/api/3/search/jql?jql=assignee%3DcurrentUser()%20AND%20statusCategory%20!%3D%20Done&fields=summary,status&maxResults=50"
    );
    parse_search(&get(&url, &email, &token)?)
}

pub fn fetch_sight(source_ref: &str) -> Option<Sight> {
    let key = parse_key(source_ref)?;
    let (base, email, token) = creds()?;
    let url = format!("{base}/rest/api/3/issue/{key}?fields=status");
    let body = get(&url, &email, &token).ok()?;
    sight_from_issue(&body).ok()
}

fn get(url: &str, email: &str, token: &str) -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let response = agent
        .get(url)
        .set("Authorization", &format!("Basic {}", base64(&format!("{email}:{token}"))))
        .set("Accept", "application/json")
        .set("User-Agent", "dayloop")
        .call()
        .map_err(|e| anyhow!("Jira に聞けません: {e}"))?;
    Ok(response.into_string()?)
}

fn base64(input: &str) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | bytes[i + 2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    let rest = bytes.len() - i;
    if rest == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rest == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_category_closes_the_issue() {
        let body = r#"{"fields":{"status":{"statusCategory":{"key":"done"}}}}"#;
        assert_eq!(sight_from_issue(body).unwrap(), Sight::Closed);
        let body = r#"{"fields":{"status":{"statusCategory":{"key":"indeterminate"}}}}"#;
        assert_eq!(sight_from_issue(body).unwrap(), Sight::Open);
    }

    #[test]
    fn search_keeps_open_issues() {
        let body = r#"{"issues":[
            {"key":"DAY-1","fields":{"summary":"直す","status":{"statusCategory":{"key":"new"}}}},
            {"key":"DAY-2","fields":{"summary":"済","status":{"statusCategory":{"key":"done"}}}}
        ]}"#;
        let items = parse_search(body).unwrap();
        assert_eq!(items[0].source_ref, "jira:DAY-1");
        assert!(items[0].open);
        assert!(!items[1].open);
    }
}
