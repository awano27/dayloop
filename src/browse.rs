//! Read Outlook on the web or Teams in the browser.
//! Jev may only open, continue, or stop. Send, delete, and reply are not choices.

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::jev::{self, HttpDecider, JevOutcome};
use crate::paths;
use crate::store::Store;

const MAX_OPENS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageItem {
    pub index: usize,
    pub label: String,
}

pub fn site_url(site: &str) -> Result<&'static str> {
    match site {
        "mail" => Ok("https://outlook.office.com/mail/"),
        "teams" => Ok("https://teams.microsoft.com/v2/"),
        _ => anyhow::bail!("site は mail か teams です"),
    }
}

pub fn choices(items: &[PageItem]) -> Vec<String> {
    let mut out: Vec<String> = items.iter().map(|item| format!("open:{}", item.index)).collect();
    out.push("stop".into());
    out
}

pub fn allowed_label(label: &str) -> bool {
    let lower = label.to_lowercase();
    let banned = [
        "送信", "削除", "返信", "転送", "破棄", "send", "delete", "reply", "forward", "discard",
    ];
    !label.trim().is_empty() && banned.iter().all(|word| !lower.contains(word))
}

pub fn run(store: &Store, site: &str) -> Result<usize> {
    let url = site_url(site)?;
    let mut page = Browser::launch()?;
    page.navigate(url)?;
    std::thread::sleep(std::time::Duration::from_secs(4));
    let mut opened = 0usize;
    let mut added = 0usize;
    let mut decider = HttpDecider {
        route: jev::route_of(""),
        timeout_ms: 4000,
    };
    while opened < MAX_OPENS {
        let items = page.list_items()?;
        if items.is_empty() {
            println!("一覧がまだ見えません。ブラウザでサインインしてから、もう一度実行してください");
            break;
        }
        let choice = match decider.decide(&format!("{site} の一覧です。読むものを1つ選ぶ"), &choices(&items)) {
            JevOutcome::Answer { choice, confidence } if confidence >= jev::DEFAULT_FLOOR => choice,
            _ => {
                println!("Jev が開けなかったので、見えている件名だけを候補にします");
                added += save_labels(store, site, &items)?;
                break;
            }
        };
        if choice == "stop" {
            break;
        }
        let Some(index) = choice.strip_prefix("open:").and_then(|n| n.parse::<usize>().ok()) else {
            break;
        };
        let Some(item) = items.iter().find(|item| item.index == index) else {
            break;
        };
        page.open(index)?;
        std::thread::sleep(std::time::Duration::from_secs(2));
        let body = page.read_pane().unwrap_or_default();
        if save_one(store, site, &item.label, &body)? {
            added += 1;
        }
        opened += 1;
    }
    Ok(added)
}

fn save_labels(store: &Store, site: &str, items: &[PageItem]) -> Result<usize> {
    let mut n = 0;
    for item in items {
        if save_one(store, site, &item.label, "")? {
            n += 1;
        }
    }
    Ok(n)
}

fn save_one(store: &Store, site: &str, label: &str, body: &str) -> Result<bool> {
    let title = label.lines().next().unwrap_or(label).trim();
    if title.is_empty() || !allowed_label(title) {
        return Ok(false);
    }
    let source = if site == "teams" { "teams" } else { "mail" };
    let source_ref = format!("browse:{site}:{title}");
    let shown = if body.trim().is_empty() {
        title.to_string()
    } else {
        let excerpt: String = body.split_whitespace().take(24).collect::<Vec<_>>().join(" ");
        format!("{title} {excerpt}")
    };
    Ok(store.add_candidate(&shown, source, Some(&source_ref))?.is_some())
}

struct Browser {
    socket: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    next_id: i64,
}

impl Browser {
    fn launch() -> Result<Self> {
        let edge = edge_path().ok_or_else(|| anyhow!("Microsoft Edge が見つかりません"))?;
        let profile = paths::data_dir().join("browser");
        std::fs::create_dir_all(&profile)?;
        let port = 9333;
        std::process::Command::new(edge)
            .args([
                &format!("--user-data-dir={}", profile.display()),
                &format!("--remote-debugging-port={port}"),
                "--no-first-run",
                "--new-window",
                "about:blank",
            ])
            .spawn()?;
        let ws = wait_page(port)?;
        let (socket, _) = tungstenite::connect(ws)?;
        Ok(Self { socket, next_id: 1 })
    }

    fn navigate(&mut self, url: &str) -> Result<()> {
        self.call("Page.enable", serde_json::json!({}))?;
        self.call("Page.navigate", serde_json::json!({ "url": url }))?;
        Ok(())
    }

    fn list_items(&mut self) -> Result<Vec<PageItem>> {
        let value = self.eval(LIST_JS)?;
        let labels = value.as_array().cloned().unwrap_or_default();
        Ok(labels
            .into_iter()
            .enumerate()
            .filter_map(|(index, label)| {
                let label = label.as_str()?.trim().to_string();
                if allowed_label(&label) {
                    Some(PageItem { index, label })
                } else {
                    None
                }
            })
            .collect())
    }

    fn open(&mut self, index: usize) -> Result<()> {
        self.eval(&format!("document.querySelector('[data-dayloop=\"{index}\"]').click(); 'ok'"))?;
        Ok(())
    }

    fn read_pane(&mut self) -> Result<String> {
        let value = self.eval(
            "(document.querySelector('[role=main]') || document.body).innerText.slice(0, 2000)",
        )?;
        Ok(value.as_str().unwrap_or("").to_string())
    }

    fn eval(&mut self, expression: &str) -> Result<Value> {
        let result = self.call(
            "Runtime.evaluate",
            serde_json::json!({ "expression": expression, "returnByValue": true }),
        )?;
        Ok(result.pointer("/result/result/value").cloned().unwrap_or(Value::Null))
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let text = serde_json::json!({ "id": id, "method": method, "params": params }).to_string();
        self.socket.send(tungstenite::Message::Text(text))?;
        loop {
            let message = self.socket.read()?;
            let tungstenite::Message::Text(body) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&body)?;
            if value.get("id").and_then(|v| v.as_i64()) == Some(id) {
                return Ok(value);
            }
        }
    }
}

fn wait_page(port: u16) -> Result<String> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    for _ in 0..25 {
        if let Ok(response) = ureq::get(&url).call() {
            if let Ok(text) = response.into_string() {
                if let Ok(Value::Array(pages)) = serde_json::from_str::<Value>(&text) {
                    if let Some(ws) = pages.iter().find_map(|page| {
                        if page.get("type").and_then(|v| v.as_str()) == Some("page") {
                            page.get("webSocketDebuggerUrl").and_then(|v| v.as_str()).map(str::to_string)
                        } else {
                            None
                        }
                    }) {
                        return Ok(ws);
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    anyhow::bail!("ブラウザが起動しませんでした")
}

fn edge_path() -> Option<std::path::PathBuf> {
    let candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    candidates.into_iter().map(std::path::PathBuf::from).find(|path| path.exists())
}

const LIST_JS: &str = r#"(() => {
  const bad = /送信|削除|返信|転送|破棄|send|delete|reply|forward|discard/i;
  const nodes = [...document.querySelectorAll('[role="option"],[role="listitem"]')];
  const items = [];
  for (const el of nodes) {
    const name = (el.getAttribute('aria-label') || el.innerText || '').trim().replace(/\s+/g, ' ').slice(0, 180);
    if (!name || bad.test(name)) continue;
    el.setAttribute('data-dayloop', String(items.length));
    items.push(name);
    if (items.length >= 8) break;
  }
  return items;
})()"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_delete_are_not_choices() {
        assert!(!allowed_label("返信"));
        assert!(!allowed_label("Delete"));
        assert!(allowed_label("見積の確認"));
        let items = vec![PageItem { index: 0, label: "見積の確認".into() }];
        assert_eq!(choices(&items), vec!["open:0".to_string(), "stop".to_string()]);
    }
}
