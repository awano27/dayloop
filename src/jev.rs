//! Jev is asked only when the decision graph has no edge.
//! Accepting an answer is what grows the graph (`Store::revise_task`).

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
