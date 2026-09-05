//! Egress filtering for an explicitly selected MCP connection profile.
//! This controls dayloop responses, not a client application's identity or network.
use anyhow::{bail, Result};
use serde_json::Value;

use crate::config::{CloudProfile, Config};

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum Profile {
    #[default]
    Local,
    GithubCopilot,
}

pub struct AccessPolicy {
    cloud: Option<CloudProfile>,
}

impl AccessPolicy {
    pub fn new(profile: Profile, config: &Config) -> Result<Self> {
        let cloud = match profile {
            Profile::Local => None,
            Profile::GithubCopilot => {
                if !config.ai.github_copilot.enabled {
                    bail!("GitHub Copilotへの送信は無効です。config.toml の [ai.github_copilot] で送信項目を確認し enabled=true を明示してください。クラウドチャットへ直接貼り付けた情報はdayloopでは制御できません");
                }
                Some(config.ai.github_copilot.clone())
            }
        };
        Ok(Self { cloud })
    }

    pub fn filter(&self, value: Value) -> Value {
        match &self.cloud {
            None => value,
            Some(cloud) => filter_value(value, cloud),
        }
    }
}

fn filter_value(value: Value, cloud: &CloudProfile) -> Value {
    match value {
        Value::Object(object) => {
            let mut out = serde_json::Map::new();
            for (key, value) in object {
                let allowed = match key.as_str() {
                    "title" | "subject" => cloud.allow_titles,
                    "body" | "excerpt" => cloud.allow_bodies,
                    "evidence" => cloud.allow_evidence,
                    "source_ref" | "path" | "organizer" | "location" => cloud.allow_source_refs,
                    "reason" | "state_reason" | "note" | "notes" | "retro_note" | "scope"
                    | "detail" => cloud.allow_notes,
                    "question" if value.is_string() => cloud.allow_titles,
                    "error" => {
                        if value.is_null() {
                            out.insert(key, value);
                            continue;
                        }
                        let safe = match value.as_str() {
                            Some("carry_blocked") => "carry_blocked",
                            Some("reviews_pending") => "reviews_pending",
                            _ => "操作を完了できませんでした。ローカルのdayloopで詳細を確認してください",
                        };
                        out.insert(key, Value::String(safe.into()));
                        continue;
                    }
                    // Protocol structure, IDs, status, counts and dates are shared by opt-in.
                    "id"
                    | "target_id"
                    | "task_id"
                    | "candidate_id"
                    | "observation_id"
                    | "routine_id"
                    | "meeting_id"
                    | "categories"
                    | "carry_limit"
                    | "blocked"
                    | "warnings"
                    | "date"
                    | "plan_date"
                    | "due"
                    | "from"
                    | "to"
                    | "start"
                    | "end"
                    | "starts_on"
                    | "created_at"
                    | "closed_at"
                    | "updated_at"
                    | "observed_at"
                    | "synced_at"
                    | "started_at"
                    | "finished_at"
                    | "plan_confirmed_at"
                    | "kind"
                    | "name"
                    | "category"
                    | "state"
                    | "status"
                    | "source"
                    | "outcome"
                    | "action"
                    | "ok"
                    | "closed"
                    | "reopened"
                    | "required"
                    | "enabled"
                    | "stale"
                    | "is_organizer"
                    | "default"
                    | "saved"
                    | "added"
                    | "completed"
                    | "skipped"
                    | "count"
                    | "item_count"
                    | "estimate_min"
                    | "carried_count"
                    | "age_days"
                    | "open_candidate_count"
                    | "candidates_added"
                    | "candidates_skipped"
                    | "done"
                    | "not_done"
                    | "carried"
                    | "dropped"
                    | "done_rate_pct"
                    | "label"
                    | "tool"
                    | "args"
                    | "needs"
                    | "question"
                    | "questions"
                    | "options"
                    | "day"
                    | "tasks"
                    | "task"
                    | "events"
                    | "open"
                    | "backlog"
                    | "planned"
                    | "in_progress"
                    | "untouched"
                    | "candidate"
                    | "candidates"
                    | "unclosed_days"
                    | "totals"
                    | "by_day"
                    | "repeat_offenders"
                    | "sources"
                    | "errors"
                    | "issues"
                    | "code"
                    | "reviews"
                    | "review"
                    | "history"
                    | "generated_tasks"
                    | "missed_routines"
                    | "routine_gap_limit_days"
                    | "routine"
                    | "routines"
                    | "weekdays"
                    | "observation"
                    | "observations"
                    | "provenance"
                    | "fetch_reports" => true,
                    _ => false,
                };
                if allowed {
                    out.insert(key, filter_value(value, cloud));
                }
            }
            Value::Object(out)
        }
        Value::Array(values) => {
            Value::Array(values.into_iter().map(|v| filter_value(v, cloud)).collect())
        }
        value => value,
    }
}
