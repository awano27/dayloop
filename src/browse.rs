//! Read Outlook on the web or Teams in the browser.
//! Jev may only open, continue, or stop. Send, delete, and reply are not choices.

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::jev::{self, Decider, HttpDecider, JevOutcome};
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
        "gmail" => Ok("https://mail.google.com/mail/u/0/#inbox"),
        "teams" => Ok("https://teams.microsoft.com/v2/"),
        "boards" => Ok("https://dev.azure.com/pcedx/pcedx-1/_workitems/recentlyupdated/"),
        _ => anyhow::bail!("site は mail、gmail、teams、boards のいずれかです"),
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
        "送信", "削除", "返信", "転送", "破棄", "作成", "send", "delete", "reply", "forward",
        "discard", "compose",
    ];
    !label.trim().is_empty() && banned.iter().all(|word| !lower.contains(word))
}

pub fn subject_line(label: &str) -> String {
    let line = label.lines().next().unwrap_or(label).trim();
    let head = line.split(" - ").next().unwrap_or(line).trim();
    head.chars().take(80).collect()
}

const BOARD_CHROME: &[&str] = &[
    "New Work Item",
    "Column Options",
    "Recycle Bin",
    "Import Work Items",
    "Create Query",
    "Open in Queries",
    "Recently updated",
    "Back to work items",
    "新規作業項目",
    "列のオプション",
];

/// Boards rows mix the work-item title with toolbar text. Keep the title.
pub fn work_item_title(label: &str) -> String {
    let flat = label.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut text = flat;
    for phrase in BOARD_CHROME {
        text = strip_phrase(&text, phrase);
    }
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = subject_line(&flat);
    if title.chars().count() < 2 { String::new() } else { title }
}

fn strip_phrase(text: &str, phrase: &str) -> String {
    let lower = text.to_lowercase();
    let needle = phrase.to_lowercase();
    if lower.len() != text.len() {
        return text.replace(phrase, " ");
    }
    let mut out = String::new();
    let mut rest = text;
    let mut lower_rest = lower.as_str();
    while let Some(pos) = lower_rest.find(&needle) {
        out.push_str(&rest[..pos]);
        out.push(' ');
        let next = pos + needle.len();
        rest = &rest[next..];
        lower_rest = &lower_rest[next..];
    }
    out.push_str(rest);
    out
}

pub fn is_language_picker(items: &[PageItem]) -> bool {
    const NAMES: &[&str] = &[
        "Afrikaans", "Deutsch", "English", "Español", "Français", "Italiano", "日本語",
        "中文", "한국어", "Português",
    ];
    items
        .iter()
        .filter(|item| NAMES.iter().any(|name| item.label.contains(name)))
        .count()
        >= 3
}

pub fn keep_message(outcome: &JevOutcome, text: &str, keywords: &[String]) -> bool {
    match outcome {
        JevOutcome::Answer { choice, confidence }
            if choice == "keep" && *confidence >= jev::DEFAULT_FLOOR =>
        {
            true
        }
        JevOutcome::Answer { .. } => false,
        JevOutcome::Unavailable => keywords.iter().any(|word| !word.is_empty() && text.contains(word.as_str())),
    }
}

pub fn run(store: &Store, site: &str) -> Result<usize> {
    let url = site_url(site)?;
    let mut page = Browser::launch()?;
    page.navigate(url)?;
    let mut opened = 0usize;
    let mut added = 0usize;
    let mut decider = HttpDecider {
        route: jev::route_of(""),
        timeout_ms: 4000,
    };
    let mut announced = false;
    let mut items = Vec::new();
    for _ in 0..30 {
        items = page.list_items()?;
        if !items.is_empty() {
            break;
        }
        if !announced {
            println!("一覧を待っています。開いた窓でサインインしてください");
            announced = true;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    if items.is_empty() {
        println!("一覧がまだ見えません。開いた窓は残してあります");
        return Ok(0);
    }
    if is_language_picker(&items) {
        println!("言語の選択画面です。受信トレイが出るまで操作してください。候補には入れていません");
        return Ok(0);
    }
    let keywords = crate::config::load().intake.keywords;
    let limit = MAX_OPENS.min(items.len());
    for index in 0..limit {
        let Some(item) = items.iter().find(|item| item.index == index).cloned() else {
            continue;
        };
        let subject = if site == "boards" {
            let cleaned = work_item_title(&item.label);
            if cleaned.is_empty() {
                continue;
            }
            cleaned
        } else {
            subject_line(&item.label)
        };
        page.open(index)?;
        std::thread::sleep(std::time::Duration::from_secs(2));
        let body = page.read_pane().unwrap_or_default();
        let evidence = format!("{subject}\n{body}");
        let kind = if site == "boards" { "作業項目" } else { "メール" };
        let outcome = decider.decide(
            &format!("この{kind}を今日の候補にしますか。\n{evidence}"),
            &["keep".into(), "skip".into()],
        );
        match &outcome {
            JevOutcome::Answer { choice, confidence } => {
                println!("開いた: {subject} / Jev: {choice} ({confidence:.2})");
            }
            JevOutcome::Unavailable => {
                println!("開いた: {subject} / Jev は呼べなかったので、キーワードだけで判断します");
            }
        }
        if keep_message(&outcome, &evidence, &keywords) && save_one(store, site, &subject, &body)? {
            added += 1;
        }
        opened += 1;
        items = page.list_items().unwrap_or_default();
        if is_language_picker(&items) {
            break;
        }
    }
    let _ = opened;
    Ok(added)
}

fn save_one(store: &Store, site: &str, label: &str, body: &str) -> Result<bool> {
    let title = if site == "boards" {
        work_item_title(label)
    } else {
        label.lines().next().unwrap_or(label).trim().to_string()
    };
    if title.is_empty() || !allowed_label(&title) {
        return Ok(false);
    }
    let source = match site {
        "teams" => "teams",
        "boards" => "ticket",
        _ => "mail",
    };
    let source_ref = format!("browse:{site}:{title}");
    let shown = if site == "boards" || body.trim().is_empty() {
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
        if wait_page(port).is_err() {
            spawn_detached(
                &edge,
                &[
                    format!("--user-data-dir={}", profile.display()),
                    format!("--remote-debugging-port={port}"),
                    "--no-first-run".into(),
                    "--new-window".into(),
                    "--start-maximized".into(),
                    "about:blank".into(),
                ],
            )?;
        }
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
        let value = self.eval(&format!("{DOCS_JS}\n{LIST_JS}"))?;
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
        self.eval(&format!(
            "{DOCS_JS}\n(() => {{ for (const doc of dayloopDocs()) {{ const el = doc.querySelector('[data-dayloop=\"{index}\"]'); if (el) {{ el.click(); return 'ok'; }} }} return 'miss'; }})()"
        ))?;
        Ok(())
    }

    fn read_pane(&mut self) -> Result<String> {
        let value = self.eval(&format!("{DOCS_JS}\n{READ_JS}"))?;
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

fn spawn_detached(edge: &std::path::Path, args: &[String]) -> Result<()> {
    let mut parts = vec![format!("\"{}\"", edge.display())];
    parts.extend(args.iter().map(|arg| format!("\"{arg}\"")));
    let command_line = parts.join(" ");
    let script = format!(
        "Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{{ CommandLine = '{}' }} | Out-Null",
        command_line.replace('\'', "''")
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()?;
    if !status.success() {
        anyhow::bail!("ブラウザを切り離して起動できませんでした");
    }
    Ok(())
}

fn edge_path() -> Option<std::path::PathBuf> {
    let candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    candidates.into_iter().map(std::path::PathBuf::from).find(|path| path.exists())
}

const DOCS_JS: &str = r#"function dayloopDocs() {
  const docs = [document];
  for (const frame of document.querySelectorAll('iframe')) {
    try { if (frame.contentDocument) docs.push(frame.contentDocument); } catch (e) {}
  }
  return docs;
}
"#;

const LIST_JS: &str = r#"(() => {
  const bad = /送信|削除|返信|転送|破棄|作成|send|delete|reply|forward|discard|compose/i;
  const items = [];
  for (const doc of dayloopDocs()) {
    const nodes = [...doc.querySelectorAll('tr.zA, [role="option"], [role="listitem"], [role="row"]')];
    for (const el of nodes) {
      const name = (el.getAttribute('aria-label') || el.innerText || '').trim().replace(/\s+/g, ' ').slice(0, 180);
      if (!name || name.length < 8 || bad.test(name)) continue;
      el.setAttribute('data-dayloop', String(items.length));
      items.push(name);
      if (items.length >= 8) break;
    }
    if (items.length >= 8) break;
  }
  return items;
})()"#;

const READ_JS: &str = r#"(() => {
  for (const doc of dayloopDocs()) {
    const pane = doc.querySelector('[role="main"]') || doc.body;
    const text = (pane && pane.innerText || '').trim();
    if (text.length > 80) return text.slice(0, 2000);
  }
  return '';
})()"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_delete_are_not_choices() {
        assert!(!allowed_label("返信"));
        assert!(!allowed_label("Delete"));
        assert!(allowed_label("見積の確認"));
        assert!(!allowed_label("Compose"));
        assert_eq!(site_url("gmail").unwrap(), "https://mail.google.com/mail/u/0/#inbox");
        assert_eq!(
            site_url("boards").unwrap(),
            "https://dev.azure.com/pcedx/pcedx-1/_workitems/recentlyupdated/"
        );
        let picker = vec![
            PageItem { index: 0, label: "English".into() },
            PageItem { index: 1, label: "Deutsch".into() },
            PageItem { index: 2, label: "日本語".into() },
        ];
        assert!(is_language_picker(&picker));
        assert_eq!(subject_line("田中 見積の確認 - 明日まで"), "田中 見積の確認");
        assert_eq!(work_item_title("test New Work Item Column Options"), "test");
        assert!(work_item_title("New Work Item Column Options").is_empty());
        assert!(keep_message(
            &JevOutcome::Answer { choice: "keep".into(), confidence: 0.8 },
            "見積",
            &[]
        ));
        assert!(!keep_message(&JevOutcome::Unavailable, "ニュースレター", &["お願い".into()]));
        assert!(keep_message(&JevOutcome::Unavailable, "お願い 確認", &["お願い".into()]));
        let items = vec![PageItem { index: 0, label: "見積の確認".into() }];
        assert_eq!(choices(&items), vec!["open:0".to_string(), "stop".to_string()]);
    }
}
