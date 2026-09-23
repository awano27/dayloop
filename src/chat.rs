//! Recent Teams chats via Microsoft Graph. No token means nothing is imported.

use anyhow::{anyhow, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub source_ref: String,
}

pub fn token() -> Option<String> {
    for key in ["TEAMS_TOKEN", "GRAPH_TOKEN"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

pub fn parse_chats(body: &str) -> Result<Vec<Item>> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let rows = value.get("value").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let topic = row.get("topic").and_then(|v| v.as_str()).unwrap_or("").trim();
        let preview = row
            .pointer("/lastMessagePreview/body/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let title = if !topic.is_empty() {
            topic
        } else {
            preview.lines().next().unwrap_or("").trim()
        };
        if title.is_empty() {
            continue;
        }
        let title: String = title.chars().take(120).collect();
        out.push(Item {
            title,
            source_ref: format!("teams:{id}"),
        });
    }
    Ok(out)
}

pub fn fetch_recent() -> Result<Vec<Item>> {
    let token = token().ok_or_else(|| anyhow!("Teams のトークンがありません"))?;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let response = agent
        .get("https://graph.microsoft.com/v1.0/me/chats?$top=25&$expand=lastMessagePreview")
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .set("User-Agent", "dayloop")
        .call()
        .map_err(|e| anyhow!("Teams に聞けません: {e}"))?;
    parse_chats(&response.into_string()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_or_preview_becomes_a_candidate() {
        let body = r#"{"value":[
            {"id":"19:abc","topic":"週次","lastMessagePreview":{"body":{"content":"資料を見て"}}},
            {"id":"19:def","topic":null,"lastMessagePreview":{"body":{"content":"お願い、見積を直して"}}}
        ]}"#;
        let items = parse_chats(body).unwrap();
        assert_eq!(items[0].title, "週次");
        assert_eq!(items[0].source_ref, "teams:19:abc");
        assert_eq!(items[1].title, "お願い、見積を直して");
    }
}
