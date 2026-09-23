//! Azure DevOps Boards. Uses a PAT, or the Azure CLI session. No token means skip.

use anyhow::{anyhow, Result};

use crate::observe::Sight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub source_ref: String,
    pub open: bool,
}

pub fn org() -> Option<String> {
    std::env::var("AZURE_DEVOPS_ORG")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

pub fn parse_work_items(org: &str, body: &str) -> Result<Vec<Item>> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let rows = value.get("value").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(|v| v.as_u64()) else {
            continue;
        };
        let fields = row.get("fields");
        let title = fields
            .and_then(|f| f.get("System.Title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if title.is_empty() {
            continue;
        }
        let project = fields
            .and_then(|f| f.get("System.TeamProject"))
            .and_then(|v| v.as_str())
            .unwrap_or("project");
        let state = fields
            .and_then(|f| f.get("System.State"))
            .and_then(|v| v.as_str())
            .unwrap_or("Active");
        out.push(Item {
            title: title.to_string(),
            source_ref: format!("devops:{org}/{project}#{id}"),
            open: !is_done(state),
        });
    }
    Ok(out)
}

pub fn sight_from_work_item(body: &str) -> Result<Sight> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let state = value
        .pointer("/fields/System.State")
        .and_then(|v| v.as_str())
        .unwrap_or("Active");
    Ok(if is_done(state) { Sight::Closed } else { Sight::Open })
}

pub fn fetch_assigned() -> Result<Vec<Item>> {
    let org = org().ok_or_else(|| anyhow!("AZURE_DEVOPS_ORG がありません"))?;
    let token = token().ok_or_else(|| anyhow!("Azure DevOps の資格情報がありません"))?;
    let org_name = org_name(&org);
    let projects = projects(&org, &token)?;
    let mut out = Vec::new();
    for project in projects.into_iter().take(8) {
        let url = format!(
            "https://dev.azure.com/{org_name}/{project}/_apis/wit/wiql?api-version=7.1"
        );
        let query = r#"{"query":"Select [System.Id] From WorkItems Where [System.AssignedTo] = @Me"}"#;
        let listed = match post(&url, &token, query) {
            Ok(body) => body,
            Err(_) => continue,
        };
        let ids = wiql_ids(&listed);
        if ids.is_empty() {
            continue;
        }
        let id_list = ids.iter().take(50).map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        let get_url = format!(
            "https://dev.azure.com/{org_name}/{project}/_apis/wit/workitems?ids={id_list}&fields=System.Title,System.State,System.TeamProject&api-version=7.1"
        );
        if let Ok(body) = get(&get_url, &token) {
            if let Ok(items) = parse_work_items(&org, &body) {
                out.extend(items);
            }
        }
    }
    Ok(out)
}

pub fn fetch_sight(source_ref: &str) -> Option<Sight> {
    let (org, project, id) = parse_ref(source_ref)?;
    let token = token()?;
    let url = format!(
        "https://dev.azure.com/{}/{}",
        org_name(&org),
        format!("{project}/_apis/wit/workitems/{id}?fields=System.State&api-version=7.1")
    );
    let body = get(&url, &token).ok()?;
    sight_from_work_item(&body).ok()
}

fn parse_ref(source_ref: &str) -> Option<(String, String, u64)> {
    let rest = source_ref.strip_prefix("devops:")?;
    let (slug, id) = rest.rsplit_once('#')?;
    let (org, project) = slug.split_once('/')?;
    Some((org.to_string(), project.to_string(), id.parse().ok()?))
}

fn is_done(state: &str) -> bool {
    matches!(
        state.to_ascii_lowercase().as_str(),
        "closed" | "done" | "completed" | "removed" | "resolved"
    )
}

fn org_name(org: &str) -> String {
    org.trim_start_matches("https://dev.azure.com/")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string()
}

fn token() -> Option<String> {
    for name in ["AZURE_DEVOPS_PAT", "ADO_PAT"] {
        if let Ok(value) = std::env::var(name) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    az_token()
}

fn az_token() -> Option<String> {
    let output = std::process::Command::new("az")
        .args([
            "account",
            "get-access-token",
            "--resource",
            "499b84ac-1321-427f-aa17-267ca6975798",
            "--query",
            "accessToken",
            "-o",
            "tsv",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?;
    let token = token.trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

fn projects(org: &str, token: &str) -> Result<Vec<String>> {
    if let Ok(project) = std::env::var("AZURE_DEVOPS_PROJECT") {
        let project = project.trim().to_string();
        if !project.is_empty() {
            return Ok(vec![project]);
        }
    }
    let url = format!(
        "https://dev.azure.com/{}/_apis/projects?api-version=7.1",
        org_name(org)
    );
    let body = get(&url, token)?;
    let value: serde_json::Value = serde_json::from_str(&body)?;
    let names = value
        .get("value")
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("name").and_then(|v| v.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(names)
}

fn wiql_ids(body: &str) -> Vec<u64> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    value
        .get("workItems")
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("id").and_then(|v| v.as_u64()))
                .collect()
        })
        .unwrap_or_default()
}

fn auth(token: &str) -> String {
    if token.starts_with("ey") {
        format!("Bearer {token}")
    } else {
        format!("Basic {}", base64(&format!(":{token}")))
    }
}

fn get(url: &str, token: &str) -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(8))
        .build();
    let response = agent
        .get(url)
        .set("Authorization", &auth(token))
        .set("Accept", "application/json")
        .set("User-Agent", "dayloop")
        .call()
        .map_err(|e| anyhow!("Azure DevOps に聞けません: {e}"))?;
    Ok(response.into_string()?)
}

fn post(url: &str, token: &str, body: &str) -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(8))
        .build();
    let response = agent
        .post(url)
        .set("Authorization", &auth(token))
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .set("User-Agent", "dayloop")
        .send_string(body)
        .map_err(|e| anyhow!("Azure DevOps に聞けません: {e}"))?;
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
    match bytes.len() - i {
        1 => {
            let n = (bytes[i] as u32) << 16;
            out.push(T[((n >> 18) & 63) as usize] as char);
            out.push(T[((n >> 12) & 63) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
            out.push(T[((n >> 18) & 63) as usize] as char);
            out.push(T[((n >> 12) & 63) as usize] as char);
            out.push(T[((n >> 6) & 63) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_work_item_is_done_and_active_stays_open() {
        let body = r#"{"value":[
            {"id":12,"fields":{"System.Title":"直す","System.State":"Active","System.TeamProject":"app"}},
            {"id":13,"fields":{"System.Title":"済","System.State":"Closed","System.TeamProject":"app"}}
        ]}"#;
        let items = parse_work_items("contoso", body).unwrap();
        assert_eq!(items[0].source_ref, "devops:contoso/app#12");
        assert!(items[0].open);
        assert!(!items[1].open);
        assert_eq!(
            sight_from_work_item(r#"{"fields":{"System.State":"Done"}}"#).unwrap(),
            Sight::Closed
        );
    }
}
