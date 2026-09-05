//! Side-effect-free snapshots: what the ledger looks like and what to ask the person.
//! CLI still asks via `rituals::Ui`; MCP returns these structs as JSON.

use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};

use crate::business::Review;
use crate::model::{Candidate, State, Task};
use crate::store::{Store, CANDIDATE_STALE_DAYS, MAX_CARRY};
use crate::util::{days_since, short, week_range};

#[derive(Debug, Clone, Serialize)]
pub struct Choice {
    pub label: String,
    pub tool: Option<String>,
    pub args: Value,
    pub needs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Question {
    pub kind: String,
    pub target_id: String,
    pub title: String,
    pub question: String,
    pub default: usize,
    pub options: Vec<Choice>,
}

fn choice(label: &str, tool: Option<&str>, args: Value, needs: &[&str]) -> Choice {
    Choice {
        label: label.into(),
        tool: tool.map(|s| s.to_string()),
        args,
        needs: needs.iter().map(|s| s.to_string()).collect(),
    }
}

fn id_args(id: &str) -> Value {
    json!({ "id": id })
}

impl Question {
    pub fn review(r: &Review) -> Self {
        let args = json!({"date":r.date,"category":r.category.as_str()});
        let mut confirmed = args.clone();
        confirmed["outcome"] = json!("confirmed");
        let mut action = args.clone();
        action["outcome"] = json!("needs_action");
        let mut not_checked = args;
        not_checked["outcome"] = json!("not_checked");
        Self {
            kind: "review".into(),
            target_id: format!("{}:{}", r.date, r.category.as_str()),
            title: r.category.label_ja().into(),
            question: format!("{} の {} を確認しましたか", r.date, r.category.label_ja()),
            default: 0,
            options: vec![
                choice("確認済み", Some("record_review"), confirmed, &[]),
                choice(
                    "対応が必要",
                    Some("record_review"),
                    action,
                    &["task_id または candidate_id"],
                ),
                choice(
                    "未確認（理由を残す）",
                    Some("record_review"),
                    not_checked,
                    &["reason"],
                ),
                choice("後で確認", None, json!({}), &[]),
            ],
        }
    }

    pub fn close_previous(date: &str) -> Self {
        Self {
            kind: "close_previous_day".into(),
            target_id: date.into(),
            title: format!("{date} の確定"),
            question: format!("{date} のタスクと確認事項を処理して、この日を閉じてください"),
            default: 0,
            options: vec![
                choice(
                    "この日の確認を開く",
                    Some("close_day"),
                    json!({"date":date}),
                    &[],
                ),
                choice("後で", None, json!({}), &[]),
            ],
        }
    }
    pub fn close_task(t: &Task) -> Self {
        let id = id_args(&t.id);
        Self {
            kind: "close_task".into(),
            target_id: t.id.clone(),
            title: t.title.clone(),
            question: format!("{} をどうしますか", t.title),
            default: 0,
            options: vec![
                choice("完了", Some("finish_task"), id.clone(), &[]),
                choice("未完了", Some("set_not_done"), id.clone(), &["reason"]),
                choice("持ち越し", Some("carry_over"), id.clone(), &["reason"]),
                choice("取り下げ", Some("drop_task"), id.clone(), &["reason"]),
            ],
        }
    }

    pub fn carry_blocked(t: &Task) -> Self {
        let id = id_args(&t.id);
        Self {
            kind: "carry_blocked".into(),
            target_id: t.id.clone(),
            title: t.title.clone(),
            question: format!(
                "「{}」は既に {} 回持ち越されています。分割・取り下げ・期限変更のどれかを選んでください",
                t.title, t.carried_count
            ),
            default: 0,
            options: vec![
                choice("分割する", Some("split_task"), id.clone(), &["titles", "reason"]),
                choice("取り下げる", Some("drop_task"), id.clone(), &["reason"]),
                choice(
                    "期限を変えて持ち越す",
                    Some("carry_over"),
                    id.clone(),
                    &["reason", "reschedule_due"],
                ),
                choice("後で決める", None, json!({}), &[]),
            ],
        }
    }

    pub fn candidate(c: &Candidate, date: &str) -> Self {
        let age = days_since(&c.created_at);
        let stale = if age >= CANDIDATE_STALE_DAYS {
            format!("  !! {age} 日放置")
        } else {
            String::new()
        };
        Self {
            kind: "candidate".into(),
            target_id: c.id.clone(),
            title: c.title.clone(),
            question: format!("{}  {}  ({}){stale}", short(&c.id), c.title, c.source),
            default: 0,
            options: vec![
                choice(
                    "今日の予定に入れる",
                    Some("accept_candidate"),
                    json!({ "id": c.id, "date": date }),
                    &[],
                ),
                choice(
                    "未計画に入れる",
                    Some("accept_candidate"),
                    json!({ "id": c.id, "backlog": true }),
                    &[],
                ),
                choice("却下", Some("reject_candidate"), id_args(&c.id), &[]),
                choice("後で決める", None, json!({}), &[]),
            ],
        }
    }

    pub fn backlog(t: &Task, date: &str) -> Self {
        Self {
            kind: "backlog".into(),
            target_id: t.id.clone(),
            title: t.title.clone(),
            question: format!("{}  {} を今日やりますか", short(&t.id), t.title),
            default: 0,
            options: vec![
                choice(
                    "今日やる",
                    Some("schedule_task"),
                    json!({ "id": t.id, "date": date }),
                    &[],
                ),
                choice("そのまま", None, json!({}), &[]),
            ],
        }
    }

    pub fn confirm_plan(date: &str, n: usize) -> Self {
        Self {
            kind: "confirm_plan".into(),
            target_id: date.to_string(),
            title: format!("{date} の予定"),
            question: format!("{date} の予定 {n} 件をこの内容で確定しますか"),
            default: 0,
            options: vec![
                choice(
                    "確定する",
                    Some("confirm_plan"),
                    json!({ "date": date }),
                    &[],
                ),
                choice("まだ", None, json!({}), &[]),
            ],
        }
    }

    pub fn check_task(t: &Task) -> Self {
        let id = id_args(&t.id);
        Self {
            kind: "check_task".into(),
            target_id: t.id.clone(),
            title: t.title.clone(),
            question: format!("未着手  {}  {}", short(&t.id), t.title),
            default: 0,
            options: vec![
                choice("今日中にやる", None, json!({}), &[]),
                choice("今から着手", Some("start_task"), id.clone(), &[]),
                choice("持ち越す", Some("carry_over"), id.clone(), &["reason"]),
                choice("取り下げる", Some("drop_task"), id.clone(), &["reason"]),
            ],
        }
    }
}

pub fn candidate_json(c: &Candidate) -> Value {
    let age = days_since(&c.created_at);
    json!({
        "id": c.id,
        "title": c.title,
        "source": c.source,
        "source_ref": c.source_ref,
        "created_at": c.created_at,
        "status": c.status,
        "age_days": age,
        "stale": age >= CANDIDATE_STALE_DAYS,
    })
}

pub fn today_view(store: &Store, date: &str) -> Result<Value> {
    let day = store.get_day(date)?;
    let tasks = store.tasks_for_day(date)?;
    let cands = store.open_candidates()?;
    let unclosed = store.unclosed_days_before(date)?;
    let events = store.events_for_day(date)?;
    Ok(json!({
        "date": date,
        "day": day,
        "tasks": tasks,
        "events": events,
        "open_candidate_count": cands.len(),
        "unclosed_days": unclosed,
        "reviews": store.reviews_for_day(date)?,
        "observations": store.observations_for_day(date)?,
        "fetch_reports": store.fetch_reports(Some(date))?,
    }))
}

pub fn plan_view(store: &Store, date: &str) -> Result<Value> {
    if store
        .get_day(date)?
        .is_some_and(|day| day.closed_at.is_some())
    {
        let mut snapshot = today_view(store, date)?;
        snapshot["closed"] = json!(true);
        snapshot["planned"] = json!(store.tasks_for_day(date)?);
        snapshot["questions"] = json!([]);
        return Ok(snapshot);
    }
    let unclosed = store.unclosed_days_before(date)?;
    let mut unclosed_out = Vec::new();
    let mut questions = Vec::new();
    for d in &unclosed {
        let open = store.open_tasks_for_day(d)?;
        unclosed_out.push(json!({ "date": d, "open": open }));
        for t in &open {
            questions.push(Question::close_task(t));
        }
        questions.extend(store.pending_reviews(d)?.iter().map(Question::review));
        questions.push(Question::close_previous(d));
    }

    let mut cands = store.open_candidates()?;
    cands.sort_by_key(|c| std::cmp::Reverse(days_since(&c.created_at) >= CANDIDATE_STALE_DAYS));
    for c in &cands {
        questions.push(Question::candidate(c, date));
    }

    let backlog = store.backlog()?;
    for t in &backlog {
        questions.push(Question::backlog(t, date));
    }

    let planned = store.tasks_for_day(date)?;
    let n_open = store.open_tasks_for_day(date)?.len();
    questions.push(Question::confirm_plan(date, n_open));

    let events = store.events_for_day(date)?;
    let reviews = store.reviews_for_day(date)?;
    questions.extend(store.pending_reviews(date)?.iter().map(Question::review));
    Ok(json!({
        "date": date,
        "unclosed_days": unclosed_out,
        "candidates": cands.iter().map(candidate_json).collect::<Vec<_>>(),
        "backlog": backlog,
        "planned": planned,
        "events": events,
        "reviews": reviews,
        "fetch_reports": store.fetch_reports(Some(date))?,
        "questions": questions,
    }))
}

pub fn check_view(store: &Store, date: &str) -> Result<Value> {
    let open = store.open_tasks_for_day(date)?;
    let doing: Vec<&Task> = open
        .iter()
        .filter(|t| t.state == State::InProgress)
        .collect();
    let untouched: Vec<&Task> = open.iter().filter(|t| t.state == State::Planned).collect();
    let mut questions: Vec<Question> = untouched.iter().map(|t| Question::check_task(t)).collect();
    questions.extend(store.pending_reviews(date)?.iter().map(Question::review));
    Ok(json!({
        "date": date,
        "in_progress": doing,
        "untouched": untouched,
        "reviews": store.reviews_for_day(date)?,
        "questions": questions,
    }))
}

pub fn close_view(store: &Store, date: &str) -> Result<Value> {
    let open = store.open_tasks_for_day(date)?;
    let mut questions: Vec<Question> = open.iter().map(Question::close_task).collect();
    questions.extend(store.pending_reviews(date)?.iter().map(Question::review));
    Ok(json!({
        "closed": false,
        "open": open,
        "reviews": store.reviews_for_day(date)?,
        "questions": questions,
    }))
}

pub fn retro_view(store: &Store, date: &str) -> Result<Value> {
    let (from, to) = week_range(date)?;
    let tasks = store.tasks_in_range(&from, &to)?;
    let count = |s: State| tasks.iter().filter(|t| t.state == s).count();
    let done = count(State::Done);
    let not_done = count(State::NotDone);
    let carried = count(State::Carried);
    let dropped = count(State::Dropped);
    let open = tasks.iter().filter(|t| t.state.is_open()).count();
    let decided = done + not_done + carried + dropped;
    let done_rate_pct = (done * 100).checked_div(decided);

    let mut by_day = Vec::new();
    let mut notes = Vec::new();
    let mut reviews = Vec::new();
    let mut d = crate::util::parse_date(&from)?;
    let end = crate::util::parse_date(&to)?;
    while d <= end {
        let ds = d.format("%Y-%m-%d").to_string();
        reviews.extend(store.reviews_for_day(&ds)?);
        if let Some(record) = store.get_day(&ds)? {
            if let Some(note) = record.retro_note {
                notes.push(json!({"date":ds,"retro_note":note}));
            }
        }
        let day: Vec<&Task> = tasks
            .iter()
            .filter(|t| t.plan_date.as_deref() == Some(&ds))
            .collect();
        if !day.is_empty() {
            let c = |s: State| day.iter().filter(|t| t.state == s).count();
            by_day.push(json!({
                "date": ds,
                "count": day.len(),
                "done": c(State::Done),
                "not_done": c(State::NotDone),
                "carried": c(State::Carried),
                "dropped": c(State::Dropped),
            }));
        }
        d += chrono::Duration::days(1);
    }

    let mut repeat: Vec<&Task> = tasks.iter().filter(|t| t.carried_count >= 2).collect();
    repeat.sort_by_key(|t| std::cmp::Reverse(t.carried_count));
    repeat.dedup_by(|a, b| a.title == b.title);
    let repeat_offenders: Vec<Value> = repeat
        .iter()
        .take(5)
        .map(|t| {
            json!({
                "title": t.title,
                "carried_count": t.carried_count,
                "state": t.state,
                "id": t.id,
            })
        })
        .collect();

    let blocked = store.blocked_carry_tasks()?;
    let questions: Vec<Question> = blocked.iter().map(Question::carry_blocked).collect();

    Ok(json!({
        "from": from,
        "to": to,
        "totals": {
            "planned": tasks.len(),
            "done": done,
            "not_done": not_done,
            "carried": carried,
            "dropped": dropped,
            "open": open,
            "done_rate_pct": done_rate_pct,
        },
        "by_day": by_day,
        "notes": notes,
        "reviews": reviews,
        "repeat_offenders": repeat_offenders,
        "blocked": blocked,
        "questions": questions,
        "carry_limit": MAX_CARRY,
    }))
}
