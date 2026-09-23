// Some columns are stored for the MCP layer (stage 2) and not yet read by the CLI.
#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Backlog,
    Planned,
    InProgress,
    Done,
    NotDone,
    Carried,
    Dropped,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Backlog => "backlog",
            State::Planned => "planned",
            State::InProgress => "in_progress",
            State::Done => "done",
            State::NotDone => "not_done",
            State::Carried => "carried",
            State::Dropped => "dropped",
        }
    }

    pub fn parse(s: &str) -> Option<State> {
        Some(match s {
            "backlog" => State::Backlog,
            "planned" => State::Planned,
            "in_progress" => State::InProgress,
            "done" => State::Done,
            "not_done" => State::NotDone,
            "carried" => State::Carried,
            "dropped" => State::Dropped,
            _ => return None,
        })
    }

    pub fn label_ja(self) -> &'static str {
        match self {
            State::Backlog => "未計画",
            State::Planned => "予定",
            State::InProgress => "進行中",
            State::Done => "完了",
            State::NotDone => "未完了",
            State::Carried => "持ち越し",
            State::Dropped => "取り下げ",
        }
    }

    /// Still needs a decision today.
    pub fn is_open(self) -> bool {
        matches!(self, State::Planned | State::InProgress)
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            State::Done | State::NotDone | State::Carried | State::Dropped
        )
    }

    pub fn needs_reason(self) -> bool {
        matches!(self, State::NotDone | State::Carried | State::Dropped)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub source: String,
    pub source_ref: Option<String>,
    pub due: Option<String>,
    pub estimate_min: Option<i64>,
    pub plan_date: Option<String>,
    pub state: State,
    pub state_reason: Option<String>,
    pub carried_count: i64,
    pub evidence: Option<String>,
    pub created_at: String,
    pub closed_at: Option<String>,
    pub carried_from: Option<String>,
    pub proposed_state: Option<String>,
    pub proposed_reason_code: Option<String>,
    pub proposal_confidence: Option<f64>,
    pub state_note: Option<String>,
    pub decided_by: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Day {
    pub date: String,
    pub plan_confirmed_at: Option<String>,
    pub closed_at: Option<String>,
    pub retro_note: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    pub source: String,
    pub source_ref: Option<String>,
    pub created_at: String,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Event {
    pub entry_id: String,
    pub date: String,
    pub start: String,
    pub end: String,
    pub subject: String,
    pub location: Option<String>,
    pub organizer: Option<String>,
    pub is_organizer: bool,
    pub source: String,
    pub synced_at: String,
}
