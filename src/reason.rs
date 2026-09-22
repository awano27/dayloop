//! Closed reason codes. Free text still counts; a code is stored as its Japanese label.

use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonCode {
    Waiting,
    Blocked,
    NoTime,
    NotNeeded,
    TooBig,
}

impl ReasonCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ReasonCode::Waiting => "waiting",
            ReasonCode::Blocked => "blocked",
            ReasonCode::NoTime => "no_time",
            ReasonCode::NotNeeded => "not_needed",
            ReasonCode::TooBig => "too_big",
        }
    }

    pub fn label_ja(self) -> &'static str {
        match self {
            ReasonCode::Waiting => "待ち",
            ReasonCode::Blocked => "ブロック",
            ReasonCode::NoTime => "時間不足",
            ReasonCode::NotNeeded => "不要",
            ReasonCode::TooBig => "大きすぎる",
        }
    }

    pub fn parse(s: &str) -> Option<ReasonCode> {
        match s.trim() {
            "waiting" | "待ち" => Some(ReasonCode::Waiting),
            "blocked" | "ブロック" => Some(ReasonCode::Blocked),
            "no_time" | "時間不足" => Some(ReasonCode::NoTime),
            "not_needed" | "不要" => Some(ReasonCode::NotNeeded),
            "too_big" | "大きすぎる" => Some(ReasonCode::TooBig),
            _ => None,
        }
    }
}

/// Empty fails. A known code becomes its code string. Other text is kept trimmed.
pub fn canonical_code(s: &str) -> Result<String> {
    let t = s.trim();
    if t.is_empty() {
        bail!("理由が空です");
    }
    Ok(ReasonCode::parse(t)
        .map(|c| c.as_str().to_string())
        .unwrap_or_else(|| t.to_string()))
}

/// Value written into `state_reason`.
pub fn reason_for_ledger(stored: &str) -> String {
    ReasonCode::parse(stored)
        .map(|c| c.label_ja().to_string())
        .unwrap_or_else(|| stored.trim().to_string())
}
