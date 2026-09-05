use std::collections::HashSet;
use std::fmt;

use anyhow::{bail, Result};
use chrono::{DateTime, Datelike, Duration, FixedOffset};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

use crate::model::{Candidate, State, Task};
use crate::store::Store;
use crate::util::{now, parse_date};

pub const ROUTINE_GAP_LOOKBACK_DAYS: i64 = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Teams,
    Outlook,
    Attendance,
    Tasks,
    Alerts,
    MeetingPrep,
    MeetingResults,
}

impl Category {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "teams" => Ok(Self::Teams),
            "outlook" => Ok(Self::Outlook),
            "attendance" => Ok(Self::Attendance),
            "tasks" => Ok(Self::Tasks),
            "alerts" => Ok(Self::Alerts),
            "meeting_prep" => Ok(Self::MeetingPrep),
            "meeting_results" => Ok(Self::MeetingResults),
            _ => bail!("未知の確認カテゴリです: {value}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Teams => "teams",
            Self::Outlook => "outlook",
            Self::Attendance => "attendance",
            Self::Tasks => "tasks",
            Self::Alerts => "alerts",
            Self::MeetingPrep => "meeting_prep",
            Self::MeetingResults => "meeting_results",
        }
    }

    pub fn label_ja(self) -> &'static str {
        match self {
            Self::Teams => "Teams",
            Self::Outlook => "Outlook",
            Self::Attendance => "勤怠",
            Self::Tasks => "タスク",
            Self::Alerts => "アラート",
            Self::MeetingPrep => "会議準備",
            Self::MeetingResults => "会議結果",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Self::Teams,
            Self::Outlook,
            Self::Attendance,
            Self::Tasks,
            Self::Alerts,
            Self::MeetingPrep,
            Self::MeetingResults,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    Pending,
    Confirmed,
    NeedsAction,
    NotChecked,
}

impl ReviewOutcome {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "confirmed" => Ok(Self::Confirmed),
            "needs_action" => Ok(Self::NeedsAction),
            "not_checked" => Ok(Self::NotChecked),
            _ => bail!("未知の確認結果です: {value}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::NeedsAction => "needs_action",
            Self::NotChecked => "not_checked",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Review {
    pub date: String,
    pub category: Category,
    pub required: bool,
    pub outcome: ReviewOutcome,
    pub reason: Option<String>,
    pub task_id: Option<String>,
    pub candidate_id: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug)]
pub struct ReviewPending {
    pub reviews: Vec<Review>,
}

impl fmt::Display for ReviewPending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let categories = self
            .reviews
            .iter()
            .map(|review| review.category.label_ja())
            .collect::<Vec<_>>()
            .join("、");
        write!(f, "未確認の必須チェックがあります: {categories}")
    }
}

impl std::error::Error for ReviewPending {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchStatus {
    Success,
    Partial,
    Failed,
    Unavailable,
    Disabled,
    Stale,
}

impl FetchStatus {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "success" => Ok(Self::Success),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            "unavailable" => Ok(Self::Unavailable),
            "disabled" => Ok(Self::Disabled),
            "stale" => Ok(Self::Stale),
            _ => bail!("未知の取得状態です: {value}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
            Self::Disabled => "disabled",
            Self::Stale => "stale",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FetchReportInput {
    pub category: Category,
    pub source: String,
    pub scope: String,
    pub status: FetchStatus,
    pub item_count: i64,
    pub started_at: String,
    pub finished_at: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FetchReport {
    pub id: String,
    pub category: Category,
    pub source: String,
    pub scope: String,
    pub status: FetchStatus,
    pub item_count: i64,
    pub started_at: String,
    pub finished_at: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Observation {
    pub id: String,
    pub category: Category,
    pub source_ref: String,
    pub meeting_id: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub observed_at: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Routine {
    pub id: String,
    pub title: String,
    pub weekdays: Vec<String>,
    pub enabled: bool,
    pub starts_on: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RoutineGap {
    pub routine_id: String,
    pub title: String,
    pub date: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreparedDay {
    pub reviews: Vec<Review>,
    pub generated_tasks: Vec<Task>,
    pub missed_routines: Vec<RoutineGap>,
}

fn new_id() -> String {
    ulid::Ulid::new().to_string()
}

fn validate_timestamp(value: &str, field: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value)
        .map_err(|_| anyhow::anyhow!("{field} は RFC3339 形式で指定してください: {value}"))
}

fn row_to_review(row: &Row<'_>) -> rusqlite::Result<Review> {
    let category: String = row.get("category")?;
    let outcome: String = row.get("outcome")?;
    let parse = |error: anyhow::Error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()).into(),
        )
    };
    Ok(Review {
        date: row.get("date")?,
        category: Category::parse(&category).map_err(parse)?,
        required: row.get::<_, i64>("required")? != 0,
        outcome: ReviewOutcome::parse(&outcome).map_err(parse)?,
        reason: row.get("reason")?,
        task_id: row.get("task_id")?,
        candidate_id: row.get("candidate_id")?,
        updated_at: row.get("updated_at")?,
    })
}

fn row_to_fetch_report(row: &Row<'_>) -> rusqlite::Result<FetchReport> {
    let category: String = row.get("category")?;
    let status: String = row.get("status")?;
    let parse = |error: anyhow::Error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()).into(),
        )
    };
    Ok(FetchReport {
        id: row.get("id")?,
        category: Category::parse(&category).map_err(parse)?,
        source: row.get("source")?,
        scope: row.get("scope")?,
        status: FetchStatus::parse(&status).map_err(parse)?,
        item_count: row.get("item_count")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        reason: row.get("reason")?,
    })
}

fn row_to_observation(row: &Row<'_>) -> rusqlite::Result<Observation> {
    let category: String = row.get("category")?;
    let category = Category::parse(&category).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()).into(),
        )
    })?;
    Ok(Observation {
        id: row.get("id")?,
        category,
        source_ref: row.get("source_ref")?,
        meeting_id: row.get("meeting_id")?,
        title: row.get("title")?,
        body: row.get("body")?,
        observed_at: row.get("observed_at")?,
        created_at: row.get("created_at")?,
    })
}

fn row_to_routine(row: &Row<'_>) -> rusqlite::Result<Routine> {
    let weekdays_text: String = row.get("weekdays")?;
    let weekdays = serde_json::from_str(&weekdays_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()).into(),
        )
    })?;
    Ok(Routine {
        id: row.get("id")?,
        title: row.get("title")?,
        weekdays,
        enabled: row.get::<_, i64>("enabled")? != 0,
        starts_on: row.get("starts_on")?,
        created_at: row.get("created_at")?,
    })
}

fn weekday_name(date: chrono::NaiveDate) -> &'static str {
    match date.weekday() {
        chrono::Weekday::Mon => "Mon",
        chrono::Weekday::Tue => "Tue",
        chrono::Weekday::Wed => "Wed",
        chrono::Weekday::Thu => "Thu",
        chrono::Weekday::Fri => "Fri",
        chrono::Weekday::Sat => "Sat",
        chrono::Weekday::Sun => "Sun",
    }
}

impl Store {
    /// Called while the Store write transaction is active. Existing ledger dates are
    /// marked during migration, so configuration never retroactively changes them.
    pub(crate) fn ensure_review_set(&self, date: &str) -> Result<()> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO review_sets(date,prepared_at) VALUES(?1,?2)",
            params![date, now()],
        )?;
        if inserted == 0 {
            return Ok(());
        }
        for category in self.required_categories()? {
            self.conn.execute(
                "INSERT INTO reviews(date,category,required,outcome,updated_at) VALUES(?1,?2,1,'pending',?3)",
                params![date, category.as_str(), now()],
            )?;
        }
        Ok(())
    }

    pub fn required_categories(&self) -> Result<Vec<Category>> {
        let mut statement = self
            .conn
            .prepare("SELECT category FROM required_categories ORDER BY category")?;
        let configured = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|value| Category::parse(&value).map(|category| category.as_str()))
            .collect::<Result<HashSet<_>>>()?;
        Ok(Category::all()
            .into_iter()
            .filter(|category| configured.contains(category.as_str()))
            .collect())
    }

    pub fn set_required_categories(&self, categories: &[Category]) -> Result<()> {
        let mut seen = HashSet::new();
        for category in categories {
            if !seen.insert(category.as_str()) {
                bail!("確認カテゴリが重複しています: {}", category.as_str());
            }
        }
        self.atomic(|| {
            self.conn.execute("DELETE FROM required_categories", [])?;
            for category in categories {
                self.conn.execute(
                    "INSERT INTO required_categories(category) VALUES(?1)",
                    [category.as_str()],
                )?;
            }
            Ok(())
        })
    }

    pub fn prepare_day(&self, date: &str) -> Result<PreparedDay> {
        parse_date(date)?;
        self.atomic(|| {
            if self
                .get_day(date)?
                .is_some_and(|day| day.closed_at.is_some())
            {
                return Ok(PreparedDay {
                    reviews: self.reviews_for_day(date)?,
                    generated_tasks: Vec::new(),
                    missed_routines: self.routine_gaps(date)?,
                });
            }
            self.ensure_day(date)?;
            let generated_tasks = self.generate_routines_for_day(date)?;
            Ok(PreparedDay {
                reviews: self.reviews_for_day(date)?,
                generated_tasks,
                missed_routines: self.routine_gaps(date)?,
            })
        })
    }

    pub fn reviews_for_day(&self, date: &str) -> Result<Vec<Review>> {
        parse_date(date)?;
        let mut statement = self.conn.prepare(
            "SELECT r.date,r.category,r.required,r.outcome,r.reason,r.task_id,r.candidate_id,r.updated_at
             FROM reviews r JOIN (
               SELECT category, MAX(id) AS id FROM reviews WHERE date=?1 GROUP BY category
             ) latest ON latest.id=r.id ORDER BY r.category",
        )?;
        let rows = statement.query_map([date], row_to_review)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn pending_reviews(&self, date: &str) -> Result<Vec<Review>> {
        let mut pending = Vec::new();
        for review in self.reviews_for_day(date)? {
            if review.required
                && (review.outcome == ReviewOutcome::Pending
                    || (review.outcome == ReviewOutcome::NeedsAction
                        && !self.needs_action_link_is_resolved(&review)?))
            {
                pending.push(review);
            }
        }
        Ok(pending)
    }

    pub fn review_history(&self, date: &str, category: Category) -> Result<Vec<Review>> {
        parse_date(date)?;
        let mut statement = self.conn.prepare(
            "SELECT date,category,required,outcome,reason,task_id,candidate_id,updated_at
             FROM reviews WHERE date=?1 AND category=?2 ORDER BY id",
        )?;
        let rows = statement.query_map(params![date, category.as_str()], row_to_review)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn record_review(
        &self,
        date: &str,
        category: Category,
        outcome: ReviewOutcome,
        reason: Option<&str>,
        task_id: Option<&str>,
        candidate_id: Option<&str>,
    ) -> Result<Review> {
        parse_date(date)?;
        let reason = reason.map(str::trim).filter(|value| !value.is_empty());
        self.atomic(|| {
            self.require_open_day(date)?;
            self.ensure_day(date)?;
            let required: Option<i64> = self.conn.query_row(
                "SELECT required FROM reviews WHERE date=?1 AND category=?2 ORDER BY id LIMIT 1",
                params![date, category.as_str()],
                |row| row.get(0),
            ).optional()?;
            let Some(required) = required else {
                bail!("{} はこの日の必須確認に設定されていません", category.label_ja());
            };
            if outcome == ReviewOutcome::NotChecked && reason.is_none() {
                bail!("not_checked には理由が必須です");
            }
            let valid_task = if let Some(id) = task_id {
                if !self.task_exists_exact(id)? {
                    bail!("関連付けるタスクが見つかりません: {id}");
                }
                true
            } else {
                false
            };
            let valid_candidate = if let Some(id) = candidate_id {
                if !self.candidate_is_actionable_exact(id)? {
                    bail!("関連付ける候補が見つからないか、却下済みです: {id}");
                }
                true
            } else {
                false
            };
            if outcome == ReviewOutcome::NeedsAction && !valid_task && !valid_candidate {
                bail!("needs_action には存在するタスクまたは未却下候補を関連付けてください");
            }
            self.conn.execute(
                "INSERT INTO reviews(date,category,required,outcome,reason,task_id,candidate_id,updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![date, category.as_str(), required, outcome.as_str(), reason, task_id, candidate_id, now()],
            )?;
            self.review_history(date, category)?
                .pop()
                .ok_or_else(|| anyhow::anyhow!("確認記録の保存に失敗しました"))
        })
    }

    pub fn require_reviews_resolved(&self, date: &str) -> Result<()> {
        let reviews = self.pending_reviews(date)?;
        if reviews.is_empty() {
            Ok(())
        } else {
            Err(ReviewPending { reviews }.into())
        }
    }

    fn task_exists_exact(&self, id: &str) -> Result<bool> {
        if id.trim().is_empty() || id != id.trim() {
            bail!("タスク ID は完全な ID を指定してください");
        }
        let state: Option<String> = self
            .conn
            .query_row("SELECT state FROM tasks WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        match state {
            None => Ok(false),
            Some(state) => {
                State::parse(&state)
                    .ok_or_else(|| anyhow::anyhow!("関連タスクの状態が不正です: {id}"))?;
                Ok(true)
            }
        }
    }

    fn candidate_is_actionable_exact(&self, id: &str) -> Result<bool> {
        if id.trim().is_empty() || id != id.trim() {
            bail!("候補 ID は完全な ID を指定してください");
        }
        let status: Option<String> = self
            .conn
            .query_row("SELECT status FROM candidates WHERE id=?1", [id], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        match status.as_deref() {
            None => Ok(false),
            Some("open" | "accepted") => Ok(true),
            Some("rejected") => Ok(false),
            Some(_) => bail!("関連候補の状態が不正です: {id}"),
        }
    }

    fn needs_action_link_is_resolved(&self, review: &Review) -> Result<bool> {
        let task_is_valid = review
            .task_id
            .as_deref()
            .map(|id| self.task_exists_exact(id))
            .transpose()?
            .unwrap_or(false);
        let candidate_is_valid = review
            .candidate_id
            .as_deref()
            .map(|id| self.candidate_is_actionable_exact(id))
            .transpose()?
            .unwrap_or(false);
        Ok(task_is_valid || candidate_is_valid)
    }

    pub fn record_fetch_report(&self, report: FetchReportInput) -> Result<FetchReport> {
        let source = report.source.trim();
        let scope = report.scope.trim();
        if source.is_empty() || scope.is_empty() {
            bail!("取得元と対象範囲は必須です");
        }
        if report.item_count < 0 {
            bail!("取得件数は0以上にしてください");
        }
        let started_at = validate_timestamp(&report.started_at, "started_at")?;
        let finished_at = validate_timestamp(&report.finished_at, "finished_at")?;
        if finished_at < started_at {
            bail!("finished_at は started_at 以降にしてください");
        }
        let reason = report
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let reason = match (report.status, reason) {
            (FetchStatus::Success, value) => value.map(str::to_owned),
            (FetchStatus::Disabled, Some(value)) => Some(value.to_owned()),
            (FetchStatus::Disabled, None) => Some("disabled".to_string()),
            (_, Some(value)) => Some(value.to_owned()),
            _ => bail!("success 以外の取得状態には理由が必須です"),
        };
        let result = FetchReport {
            id: new_id(),
            category: report.category,
            source: source.to_owned(),
            scope: scope.to_owned(),
            status: report.status,
            item_count: report.item_count,
            started_at: report.started_at,
            finished_at: report.finished_at,
            reason,
        };
        self.conn.execute(
            "INSERT INTO fetch_reports(id,category,source,scope,status,item_count,started_at,finished_at,reason)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![result.id, result.category.as_str(), result.source, result.scope, result.status.as_str(), result.item_count, result.started_at, result.finished_at, result.reason],
        )?;
        Ok(result)
    }

    pub fn fetch_reports(&self, date: Option<&str>) -> Result<Vec<FetchReport>> {
        if let Some(date) = date {
            parse_date(date)?;
        }
        let mut statement = self.conn.prepare(
            "SELECT id,category,source,scope,status,item_count,started_at,finished_at,reason
             FROM fetch_reports WHERE (?1 IS NULL OR substr(started_at,1,10)=?1)
             ORDER BY started_at,id",
        )?;
        let rows = statement.query_map([date], row_to_fetch_report)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_observation(
        &self,
        category: Category,
        source_ref: &str,
        meeting_id: Option<&str>,
        title: &str,
        body: Option<&str>,
        observed_at: &str,
    ) -> Result<Observation> {
        let source_ref = source_ref.trim();
        let title = title.trim();
        if source_ref.is_empty() || title.is_empty() {
            bail!("観測の source_ref と title は必須です");
        }
        validate_timestamp(observed_at, "observed_at")?;
        self.atomic(|| {
            if let Some(existing) = self
                .conn
                .query_row(
                    "SELECT id,category,source_ref,meeting_id,title,body,observed_at,created_at
                     FROM observations WHERE category=?1 AND source_ref=?2",
                    params![category.as_str(), source_ref],
                    row_to_observation,
                )
                .optional()?
            {
                return Ok(existing);
            }
            let observation = Observation {
                id: new_id(),
                category,
                source_ref: source_ref.to_owned(),
                meeting_id: meeting_id
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
                title: title.to_owned(),
                body: body
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
                observed_at: observed_at.to_owned(),
                created_at: now(),
            };
            self.conn.execute(
                "INSERT INTO observations(id,category,source_ref,meeting_id,title,body,observed_at,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![observation.id, observation.category.as_str(), observation.source_ref, observation.meeting_id, observation.title, observation.body, observation.observed_at, observation.created_at],
            )?;
            Ok(observation)
        })
    }

    pub fn get_observation(&self, id: &str) -> Result<Observation> {
        if id.trim().is_empty() || id != id.trim() {
            bail!("観測 ID は完全な ID を指定してください");
        }
        self.conn.query_row(
            "SELECT id,category,source_ref,meeting_id,title,body,observed_at,created_at FROM observations WHERE id=?1",
            [id],
            row_to_observation,
        ).optional()?.ok_or_else(|| anyhow::anyhow!("観測が見つかりません: {id}"))
    }

    pub fn observations_for_day(&self, date: &str) -> Result<Vec<Observation>> {
        parse_date(date)?;
        let mut statement = self.conn.prepare(
            "SELECT id,category,source_ref,meeting_id,title,body,observed_at,created_at
             FROM observations WHERE substr(observed_at,1,10)=?1 ORDER BY observed_at,id",
        )?;
        let rows = statement.query_map([date], row_to_observation)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn propose_action(
        &self,
        observation_id: &str,
        action_key: &str,
        title: &str,
    ) -> Result<Option<Candidate>> {
        let action_key = action_key.trim();
        let title = title.trim();
        if action_key.is_empty() || title.is_empty() {
            bail!("action_key とタイトルは必須です");
        }
        self.atomic(|| {
            let observation = self.get_observation(observation_id)?;
            let source_ref = format!("observation:{}:action:{}", observation.id, action_key);
            let existing = self.conn.query_row(
                "SELECT id,title,source,source_ref,created_at,status FROM candidates WHERE source_ref=?1",
                [source_ref.as_str()],
                |row| Ok(Candidate { id: row.get(0)?, title: row.get(1)?, source: row.get(2)?, source_ref: row.get(3)?, created_at: row.get(4)?, status: row.get(5)? }),
            ).optional()?;
            if let Some(candidate) = existing {
                return if candidate.status == "rejected" { Ok(None) } else { Ok(Some(candidate)) };
            }
            if self.conn.query_row("SELECT 1 FROM rejected_refs WHERE source_ref=?1", [source_ref.as_str()], |_| Ok(())).optional()?.is_some() {
                return Ok(None);
            }
            let candidate = self.add_candidate(title, "observation", Some(&source_ref))?
                .ok_or_else(|| anyhow::anyhow!("候補の保存に失敗しました"))?;
            self.link_source_observation(&source_ref, &observation.id)?;
            Ok(Some(candidate))
        })
    }

    pub(crate) fn link_source_observation(
        &self,
        source_ref: &str,
        observation_id: &str,
    ) -> Result<()> {
        let source_ref = source_ref.trim();
        if source_ref.is_empty() {
            bail!("source_ref は必須です");
        }
        self.atomic(|| {
            self.get_observation(observation_id)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO source_observations(source_ref,observation_id) VALUES(?1,?2)",
                params![source_ref, observation_id],
            )?;
            Ok(())
        })
    }

    pub fn task_provenance(&self, task_id: &str) -> Result<Vec<Observation>> {
        if task_id.trim().is_empty() || task_id != task_id.trim() {
            bail!("タスク ID は完全な ID を指定してください");
        }
        let source_ref: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT source_ref FROM tasks WHERE id=?1",
                [task_id],
                |row| row.get(0),
            )
            .optional()?;
        match source_ref {
            None => bail!("タスクが見つかりません: {task_id}"),
            Some(None) => Ok(Vec::new()),
            Some(Some(source_ref)) => self.provenance_for_source_ref(Some(&source_ref)),
        }
    }

    pub fn candidate_provenance(&self, candidate_id: &str) -> Result<Vec<Observation>> {
        if candidate_id.trim().is_empty() || candidate_id != candidate_id.trim() {
            bail!("候補 ID は完全な ID を指定してください");
        }
        let source_ref: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT source_ref FROM candidates WHERE id=?1",
                [candidate_id],
                |row| row.get(0),
            )
            .optional()?;
        match source_ref {
            None => bail!("候補が見つかりません: {candidate_id}"),
            Some(None) => Ok(Vec::new()),
            Some(Some(source_ref)) => self.provenance_for_source_ref(Some(&source_ref)),
        }
    }

    fn provenance_for_source_ref(&self, source_ref: Option<&str>) -> Result<Vec<Observation>> {
        let Some(source_ref) = source_ref else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            "SELECT o.id,o.category,o.source_ref,o.meeting_id,o.title,o.body,o.observed_at,o.created_at
             FROM source_observations so JOIN observations o ON o.id=so.observation_id
             WHERE so.source_ref=?1 ORDER BY o.observed_at,o.id",
        )?;
        let rows = statement.query_map([source_ref], row_to_observation)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_routine(
        &self,
        title: &str,
        weekdays: &[String],
        starts_on: &str,
    ) -> Result<Routine> {
        let title = title.trim();
        if title.is_empty() || weekdays.is_empty() {
            bail!("定期タスクのタイトルと曜日は必須です");
        }
        parse_date(starts_on)?;
        let allowed: HashSet<&str> = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            .into_iter()
            .collect();
        let mut unique = HashSet::new();
        for weekday in weekdays {
            if !allowed.contains(weekday.as_str()) || !unique.insert(weekday.as_str()) {
                bail!("曜日は Mon..Sun を重複なく指定してください");
            }
        }
        let routine = Routine {
            id: new_id(),
            title: title.to_owned(),
            weekdays: weekdays.to_vec(),
            enabled: true,
            starts_on: starts_on.to_owned(),
            created_at: now(),
        };
        self.conn.execute(
            "INSERT INTO routines(id,title,weekdays,enabled,starts_on,created_at) VALUES(?1,?2,?3,1,?4,?5)",
            params![routine.id, routine.title, serde_json::to_string(&routine.weekdays)?, routine.starts_on, routine.created_at],
        )?;
        Ok(routine)
    }

    pub fn routines(&self) -> Result<Vec<Routine>> {
        let mut statement = self.conn.prepare("SELECT id,title,weekdays,enabled,starts_on,created_at FROM routines ORDER BY created_at,id")?;
        let rows = statement.query_map([], row_to_routine)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_routine_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        if id.trim().is_empty() || id != id.trim() {
            bail!("定期タスク ID は完全な ID を指定してください");
        }
        if self.conn.execute(
            "UPDATE routines SET enabled=?2 WHERE id=?1",
            params![id, enabled as i64],
        )? == 0
        {
            bail!("定期タスクが見つかりません: {id}");
        }
        Ok(())
    }

    fn generate_routines_for_day(&self, date: &str) -> Result<Vec<Task>> {
        let date_value = parse_date(date)?;
        let mut generated = Vec::new();
        for routine in self
            .routines()?
            .into_iter()
            .filter(|routine| routine.enabled)
        {
            if parse_date(&routine.starts_on)? > date_value
                || !routine
                    .weekdays
                    .iter()
                    .any(|day| day == weekday_name(date_value))
            {
                continue;
            }
            let existing: Option<String> = self
                .conn
                .query_row(
                    "SELECT task_id FROM routine_occurrences WHERE routine_id=?1 AND date=?2",
                    params![routine.id, date],
                    |row| row.get(0),
                )
                .optional()?;
            if existing.is_some() {
                continue;
            }
            let task_id = new_id();
            let source_ref = format!("routine:{}:{}", routine.id, date);
            self.conn.execute(
                "INSERT INTO tasks(id,title,source,source_ref,plan_date,state,created_at) VALUES(?1,?2,'routine',?3,?4,'planned',?5)",
                params![task_id, routine.title, source_ref, date, now()],
            )?;
            self.conn.execute(
                "INSERT INTO routine_occurrences(routine_id,date,task_id) VALUES(?1,?2,?3)",
                params![routine.id, date, task_id],
            )?;
            generated.push(self.get_task(&task_id)?);
        }
        Ok(generated)
    }

    fn routine_gaps(&self, date: &str) -> Result<Vec<RoutineGap>> {
        let target = parse_date(date)?;
        let lower = target - Duration::days(ROUTINE_GAP_LOOKBACK_DAYS);
        let mut gaps = Vec::new();
        for routine in self
            .routines()?
            .into_iter()
            .filter(|routine| routine.enabled)
        {
            let mut day = std::cmp::max(parse_date(&routine.starts_on)?, lower);
            while day < target {
                let formatted = day.format("%Y-%m-%d").to_string();
                if routine
                    .weekdays
                    .iter()
                    .any(|weekday| weekday == weekday_name(day))
                {
                    let exists = self
                        .conn
                        .query_row(
                            "SELECT 1 FROM routine_occurrences WHERE routine_id=?1 AND date=?2",
                            params![routine.id, formatted],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some();
                    if !exists {
                        gaps.push(RoutineGap {
                            routine_id: routine.id.clone(),
                            title: routine.title.clone(),
                            date: formatted,
                        });
                    }
                }
                day += Duration::days(1);
            }
        }
        Ok(gaps)
    }
}
