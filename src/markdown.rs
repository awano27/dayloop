//! Daily Markdown mirror. The person can edit it by hand; `import` pulls edits back.

use anyhow::{bail, Result};

use crate::model::State;
use crate::store::{Store, CANDIDATE_STALE_DAYS};
use crate::util::{days_since, hhmm, parse_date};

pub fn export(store: &Store, date: &str) -> Result<std::path::PathBuf> {
    parse_date(date)?;
    let day = store.get_day(date)?;
    let tasks = store.tasks_for_day(date)?;
    let cands = store.open_candidates()?;
    let mut out = String::new();
    out.push_str(&format!("# {date}\n\n"));
    out.push_str(
        "<!-- dayloop: 手で編集できます。[x] を付けた行は `dayloop import` で完了になります。\n",
    );
    out.push_str("     「今日のタスク」に `- [ ] 新しい行` を足すと新規タスクになります。 -->\n\n");

    let events = store.events_for_day(date)?;
    if !events.is_empty() {
        out.push_str("## 今日の予定\n");
        for e in &events {
            let loc = e
                .location
                .as_deref()
                .map(|l| format!("（{l}）"))
                .unwrap_or_default();
            out.push_str(&format!(
                "- {}-{} {}{}\n",
                hhmm(&e.start),
                hhmm(&e.end),
                e.subject,
                loc
            ));
        }
        out.push('\n');
    }

    out.push_str("## 今日のタスク\n");
    let open: Vec<_> = tasks.iter().filter(|t| t.state.is_open()).collect();
    let done: Vec<_> = tasks.iter().filter(|t| t.state == State::Done).collect();
    let other: Vec<_> = tasks
        .iter()
        .filter(|t| !t.state.is_open() && t.state != State::Done)
        .collect();
    if tasks.is_empty() {
        out.push_str("- [ ] \n");
    }
    for t in open {
        let mark = if t.state == State::InProgress {
            "着手中 "
        } else {
            ""
        };
        out.push_str(&format!(
            "- [ ] {mark}{}{} <!-- id:{} -->\n",
            t.title,
            meta(t),
            t.id
        ));
    }
    for t in done {
        out.push_str(&format!(
            "- [x] {}{} <!-- id:{} -->\n",
            t.title,
            meta(t),
            t.id
        ));
    }
    for t in other {
        let reason = t.state_reason.as_deref().unwrap_or("");
        out.push_str(&format!(
            "- [-] {}: {}{} ({reason}) <!-- id:{} -->\n",
            t.state.label_ja(),
            t.title,
            meta(t),
            t.id
        ));
    }

    if !cands.is_empty() {
        out.push_str("\n## 候補（採用 / 却下待ち）\n");
        for c in cands {
            let age = days_since(&c.created_at);
            let stale = if age >= CANDIDATE_STALE_DAYS {
                " **放置**"
            } else {
                ""
            };
            out.push_str(&format!(
                "- {} （{}、{age}日前）{stale} <!-- cand:{} -->\n",
                c.title, c.source, c.id
            ));
        }
    }

    let reviews = store.reviews_for_day(date)?;
    if !reviews.is_empty() {
        out.push_str(
            "\n## 日次確認\n\n確認結果は `dayloop reviews record` またはチャットで記録します。\n\n",
        );
        for r in reviews {
            out.push_str(&format!(
                "- {}: {}{}\n",
                r.category.label_ja(),
                r.outcome.as_str(),
                r.reason
                    .map(|s| format!(" — {}", s.replace(['\r', '\n'], " ")))
                    .unwrap_or_default()
            ));
        }
    }
    out.push_str("\n## 状態\n");
    match &day {
        Some(d) => {
            out.push_str(&format!(
                "- 計画確定: {}\n",
                d.plan_confirmed_at.as_deref().map(hhmm).unwrap_or("未")
            ));
            out.push_str(&format!(
                "- クローズ: {}\n",
                d.closed_at.as_deref().map(hhmm).unwrap_or("未")
            ));
            if let Some(n) = &d.retro_note {
                out.push_str(&format!("- 振り返り: {n}\n"));
            }
        }
        None => {
            out.push_str("- 計画確定: 未\n- クローズ: 未\n");
        }
    }

    let path = store.data_dir().join("days").join(format!("{date}.md"));
    std::fs::write(&path, out)?;
    Ok(path)
}

fn meta(t: &crate::model::Task) -> String {
    let mut m = Vec::new();
    if let Some(d) = &t.due {
        m.push(format!("期限 {d}"));
    }
    if t.carried_count > 0 {
        m.push(format!("持ち越し{}回", t.carried_count));
    }
    if m.is_empty() {
        String::new()
    } else {
        format!(" ({})", m.join("・"))
    }
}

pub struct ImportReport {
    pub completed: usize,
    pub added: usize,
}

/// Apply hand edits: `[x]` completes an open task; new `- [ ]` lines become tasks.
pub fn import(store: &Store, date: &str) -> Result<ImportReport> {
    parse_date(date)?;
    let path = store.data_dir().join("days").join(format!("{date}.md"));
    let text = std::fs::read_to_string(&path)?;
    store.atomic(|| {
        let mut report = ImportReport {
            completed: 0,
            added: 0,
        };
        let mut in_tasks = false;
        for raw in text.lines() {
            let line = raw.trim_end();
            if line.starts_with("## ") {
                in_tasks = line.starts_with("## 今日のタスク");
                continue;
            }
            if !in_tasks {
                continue;
            }
            let Some(rest) = line.strip_prefix("- [") else {
                continue;
            };
            let Some(mark) = rest.chars().next() else {
                continue;
            };
            let body = rest.get(2..).unwrap_or("").trim();
            let (title_part, id) = match body.find("<!-- id:") {
                Some(i) => {
                    let id = body[i + 8..].trim_end_matches("-->").trim().to_string();
                    (body[..i].trim().to_string(), Some(id))
                }
                None => (body.to_string(), None),
            };
            match (mark, id) {
                ('x' | 'X', Some(id)) => {
                    let t = store.get_task(&id)?;
                    if t.id != id || t.plan_date.as_deref() != Some(date) {
                        bail!("MarkdownのタスクIDが対象日と一致しません: {id}");
                    }
                    if t.state.is_open() {
                        store.transition(&t.id, State::Done, None, Some("markdown"))?;
                        report.completed += 1;
                    }
                }
                (' ', None) if !title_part.is_empty() => {
                    store.add_task(&title_part, None, None, "manual", None, Some(date))?;
                    report.added += 1;
                }
                _ => {}
            }
        }
        Ok(report)
    })
}
