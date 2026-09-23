//! Jev is asked only when the decision graph has no edge.
//! An answer at or above the floor is recorded, so the same node is not asked tomorrow.

use anyhow::Result;

use crate::graph::{self, Situation};
use crate::store::Store;

#[derive(Debug, Clone, PartialEq)]
pub enum JevOutcome {
    Answer { choice: String, confidence: f64 },
    Unavailable,
}

pub trait Decider {
    fn decide(&mut self, state: &str, choices: &[String]) -> JevOutcome;
}

#[derive(Debug, Clone)]
pub struct Proposal {
    pub id: String,
    pub title: String,
    pub choice: String,
    pub confidence: f64,
}

const TASK_CHOICES: &[&str] = &["done", "not_done", "carry", "drop", "ask"];
const CANDIDATE_CHOICES: &[&str] = &["today", "backlog", "shelve", "reject", "ask"];

/// Used when `commit_confidence` is unset and Jev is on.
pub const DEFAULT_FLOOR: f64 = 0.5;

pub fn disposition_choices() -> Vec<String> {
    let mut choices = vec!["done".to_string(), "start".to_string(), "ask".to_string()];
    for action in ["not_done", "carry", "drop"] {
        for code in ["waiting", "blocked", "no_time", "not_needed", "too_big"] {
            choices.push(format!("{action}:{code}"));
        }
    }
    choices
}

/// Apply known edges first. Remaining open tasks are sent to `decider`.
/// Nothing here writes the ledger or the graph. `ask` and outages stay questions.
pub fn propose_unknown(
    store: &Store,
    date: &str,
    decider: &mut dyn Decider,
    min_confidence: f64,
) -> Result<Vec<Proposal>> {
    graph::apply_known_tasks(store, date)?;
    let mut out = Vec::new();
    for t in store.open_tasks_for_day(date)? {
        let sit = Situation::from_task(&t, "close");
        if graph::resolve(store, &sit)?.is_some() {
            continue;
        }
        let choices: Vec<String> = TASK_CHOICES.iter().map(|s| (*s).to_string()).collect();
        let state = graph::state_line(store, &sit)?;
        let proposal = match decider.decide(&state, &choices) {
            JevOutcome::Answer { choice, confidence }
                if choice != "ask"
                    && confidence >= min_confidence
                    && TASK_CHOICES.contains(&choice.as_str()) =>
            {
                Proposal {
                    id: t.id,
                    title: t.title,
                    choice,
                    confidence,
                }
            }
            _ => Proposal {
                id: t.id,
                title: t.title,
                choice: "ask".into(),
                confidence: 0.0,
            },
        };
        out.push(proposal);
    }
    Ok(out)
}

/// Ask Jev about unknown candidates and tasks, then record answers at or above `floor`.
/// `ask`, a low score, and an outage stay open for a person.
pub fn grow(
    store: &Store,
    date: &str,
    decider: &mut dyn Decider,
    floor: f64,
) -> Result<usize> {
    let mut n = grow_candidates(store, date, decider, floor)?;
    n += grow_tasks(store, date, decider, floor)?;
    n += grow_order(store, date, decider, floor)?;
    Ok(n)
}

fn grow_candidates(
    store: &Store,
    date: &str,
    decider: &mut dyn Decider,
    floor: f64,
) -> Result<usize> {
    graph::apply_known_candidates(store, date)?;
    let choices: Vec<String> = CANDIDATE_CHOICES.iter().map(|s| (*s).to_string()).collect();
    let mut n = 0;
    for c in store.open_candidates()? {
        let sit = Situation::intake(&c.title, &c.source, date);
        if let Some(edge) = graph::resolve(store, &sit)? {
            match edge.to_choice.as_str() {
                "today" | "backlog" | "shelve" | "reject" => {
                    apply_candidate_choice(store, &c.id, date, &edge.to_choice)?;
                    n += 1;
                    continue;
                }
                "done" | "not_done" | "drop" => {
                    store.shelve_candidate_silent(&c.id)?;
                    n += 1;
                    continue;
                }
                _ => {}
            }
        }
        let state = graph::state_line(store, &sit)?;
        let JevOutcome::Answer { choice, confidence } = decider.decide(&state, &choices) else {
            continue;
        };
        if confidence < floor || !matches!(choice.as_str(), "today" | "backlog" | "shelve" | "reject") {
            continue;
        }
        graph::record(store, &sit, &choice, None)?;
        apply_candidate_choice(store, &c.id, date, &choice)?;
        n += 1;
    }
    Ok(n)
}

fn apply_candidate_choice(store: &Store, id: &str, date: &str, choice: &str) -> Result<()> {
    match choice {
        "today" => {
            store.accept_candidate_silent(id, Some(date))?;
        }
        "backlog" => {
            store.accept_candidate_silent(id, None)?;
        }
        "shelve" => store.shelve_candidate_silent(id)?,
        "reject" => store.reject_candidate_silent(id)?,
        _ => {}
    }
    Ok(())
}

fn grow_tasks(
    store: &Store,
    date: &str,
    decider: &mut dyn Decider,
    floor: f64,
) -> Result<usize> {
    graph::apply_known_tasks(store, date)?;
    let choices = disposition_choices();
    let mut n = 0;
    for t in store.open_tasks_for_day(date)? {
        let sit = Situation::from_task(&t, "close");
        if let Some(edge) = graph::resolve(store, &sit)? {
            if matches!(edge.to_choice.as_str(), "done" | "start" | "not_done" | "carry" | "drop") {
                store.revise_task(&t.id, &edge.to_choice, edge.reason_code.as_deref())?;
                n += 1;
                continue;
            }
        }
        let state = graph::state_line(store, &sit)?;
        let JevOutcome::Answer { choice, confidence } = decider.decide(&state, &choices) else {
            continue;
        };
        if confidence < floor {
            continue;
        }
        let Some((action, reason)) = split_disposition(&choice) else {
            continue;
        };
        store.revise_task(&t.id, action, reason)?;
        n += 1;
    }
    Ok(n)
}

fn grow_order(
    store: &Store,
    date: &str,
    decider: &mut dyn Decider,
    floor: f64,
) -> Result<usize> {
    let tasks = crate::order::day_tasks(store, date)?;
    let spans = crate::order::spans_from_events(&store.events_for_day(date)?);
    let Some((left, right)) = crate::order::first_open_tie(date, &tasks, &spans) else {
        return Ok(0);
    };
    if graph::saved_first(store, &left.title, &right.title)?.is_some() {
        return Ok(0);
    }
    let choices = vec![left.title.clone(), right.title.clone()];
    let state = format!("order a={} b={}", left.title, right.title);
    let JevOutcome::Answer { choice, confidence } = decider.decide(&state, &choices) else {
        return Ok(0);
    };
    if confidence < floor || (choice != left.title && choice != right.title) {
        return Ok(0);
    }
    let second = if choice == left.title { &right.title } else { &left.title };
    graph::record_order(store, &choice, second)?;
    Ok(1)
}

fn split_disposition(choice: &str) -> Option<(&str, Option<&str>)> {
    if matches!(choice, "done" | "start") {
        return Some((choice, None));
    }
    let (action, code) = choice.split_once(':')?;
    if !matches!(action, "not_done" | "carry" | "drop") {
        return None;
    }
    if crate::reason::ReasonCode::parse(code).is_none() {
        return None;
    }
    Some((action, Some(code)))
}

/// Calls Jev only when mode, route, and `DAYLOOP_JEV_API_KEY` are set.
pub fn grow_if_configured(store: &Store, date: &str) -> Result<usize> {
    let cfg = crate::config::load();
    if !cfg.jev.mode.eq_ignore_ascii_case("on") {
        return Ok(0);
    }
    let key_ok = std::env::var("DAYLOOP_JEV_API_KEY")
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    if !key_ok || cfg.jev.route.trim().is_empty() {
        return Ok(0);
    }
    let floor = cfg.jev.commit_confidence.unwrap_or(DEFAULT_FLOOR);
    let mut decider = HttpDecider {
        route: cfg.jev.route.clone(),
        timeout_ms: cfg.jev.timeout_ms,
    };
    grow(store, date, &mut decider, floor)
}

/// Write unknown answers only when a measured floor exists.
/// `None` applies nothing. `not_done`, `carry`, and `drop` stay questions
/// because the reply has no reason code.
pub fn apply_if_measured(
    store: &Store,
    date: &str,
    decider: &mut dyn Decider,
    floor: Option<f64>,
) -> Result<usize> {
    let Some(floor) = floor else {
        return Ok(0);
    };
    let rows = propose_unknown(store, date, decider, floor)?;
    let mut n = 0;
    for row in rows {
        if row.confidence < floor || !matches!(row.choice.as_str(), "done" | "start") {
            continue;
        }
        store.revise_task(&row.id, &row.choice, None)?;
        n += 1;
    }
    Ok(n)
}

/// POST adapter. Field names follow this crate, not a copied slide example.
/// 401 / 429 / 529 / transport failure become [`JevOutcome::Unavailable`].
pub struct HttpDecider {
    pub route: String,
    pub timeout_ms: u64,
}

impl Decider for HttpDecider {
    fn decide(&mut self, state: &str, choices: &[String]) -> JevOutcome {
        if self.route.trim().is_empty() {
            return JevOutcome::Unavailable;
        }
        let body = serde_json::json!({
            "state": state,
            "questions": [{ "prompt": "選択", "choices": choices }],
        });
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_millis(self.timeout_ms))
            .build();
        let response = agent
            .post(self.route.trim())
            .set("Authorization", &auth_header())
            .send_json(body);
        match response {
            Ok(resp) => parse_body(&resp.into_string().unwrap_or_default()),
            Err(ureq::Error::Status(code, _)) if matches!(code, 401 | 429 | 529) => {
                JevOutcome::Unavailable
            }
            Err(_) => JevOutcome::Unavailable,
        }
    }
}

fn auth_header() -> String {
    match std::env::var("DAYLOOP_JEV_API_KEY") {
        Ok(key) if !key.is_empty() => format!("Bearer {key}"),
        _ => String::new(),
    }
}

fn parse_body(text: &str) -> JevOutcome {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return JevOutcome::Unavailable;
    };
    if v.get("unavailable").and_then(|x| x.as_bool()) == Some(true) {
        return JevOutcome::Unavailable;
    }
    let choice = v
        .get("choice")
        .and_then(|x| x.as_str())
        .or_else(|| v.pointer("/answer/choice").and_then(|x| x.as_str()));
    let confidence = v
        .get("confidence")
        .and_then(|x| x.as_f64())
        .or_else(|| v.pointer("/answer/confidence").and_then(|x| x.as_f64()))
        .unwrap_or(0.0);
    match choice {
        Some(choice) if !choice.is_empty() => JevOutcome::Answer {
            choice: choice.to_string(),
            confidence,
        },
        _ => JevOutcome::Unavailable,
    }
}
