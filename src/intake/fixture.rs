use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, NaiveDate};
use serde::Deserialize;

use super::model::{CalendarItem, MailItem};
use super::Source;

pub struct FixtureSource {
    pub mails: Vec<MailItem>,
    pub events: Vec<CalendarItem>,
}

impl FixtureSource {
    pub fn load(dir: &Path) -> Result<Self> {
        Self::load_at(dir, Local::now())
    }

    pub fn load_at(dir: &Path, now: DateTime<Local>) -> Result<Self> {
        let mails: Vec<MailJson> = read_json(&dir.join("mails.json"))?;
        let events: Vec<EventJson> = read_json(&dir.join("events.json"))?;
        Ok(Self {
            mails: mails.into_iter().map(|m| m.into_item()).collect::<Result<Vec<_>>>()?,
            events: events
                .into_iter()
                .map(|e| e.into_item(now))
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

impl Source for FixtureSource {
    fn name(&self) -> &'static str {
        "fixture"
    }

    fn mails(&self, since: DateTime<Local>) -> Result<Vec<MailItem>> {
        Ok(self
            .mails
            .iter()
            .filter(|m| m.received_at >= since)
            .cloned()
            .collect())
    }

    fn events(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<CalendarItem>> {
        Ok(self
            .events
            .iter()
            .filter(|e| {
                let d = e.start.date_naive();
                d >= from && d <= to
            })
            .cloned()
            .collect())
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path).with_context(|| format!("読めません: {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("JSON が不正です: {}", path.display()))
}

#[derive(Deserialize)]
struct MailJson {
    entry_id: String,
    subject: String,
    sender_name: String,
    received_at: String,
    #[serde(default)]
    unread: bool,
    #[serde(default)]
    flagged: bool,
    #[serde(default)]
    flag_request: Option<String>,
    #[serde(default)]
    body_excerpt: Option<String>,
    #[serde(default)]
    conversation_topic: Option<String>,
}

impl MailJson {
    fn into_item(self) -> Result<MailItem> {
        Ok(MailItem {
            entry_id: self.entry_id,
            subject: self.subject,
            sender_name: self.sender_name,
            received_at: parse_dt(&self.received_at)?,
            unread: self.unread,
            flagged: self.flagged,
            flag_request: self.flag_request,
            body_excerpt: self.body_excerpt,
            conversation_topic: self.conversation_topic,
        })
    }
}

#[derive(Deserialize)]
struct EventJson {
    entry_id: String,
    subject: String,
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    start_offset_minutes: Option<i64>,
    #[serde(default)]
    duration_minutes: Option<i64>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    organizer: Option<String>,
    #[serde(default)]
    is_organizer: bool,
    #[serde(default)]
    response_required: bool,
    #[serde(default)]
    all_day: bool,
}

impl EventJson {
    fn into_item(self, now: DateTime<Local>) -> Result<CalendarItem> {
        let (start, end) = if let Some(off) = self.start_offset_minutes {
            let start = now + Duration::minutes(off);
            let dur = self.duration_minutes.unwrap_or(60);
            (start, start + Duration::minutes(dur))
        } else {
            let start = parse_dt(self.start.as_deref().context("start が必要です")?)?;
            let end = match &self.end {
                Some(s) => parse_dt(s)?,
                None => start + Duration::minutes(self.duration_minutes.unwrap_or(60)),
            };
            (start, end)
        };
        Ok(CalendarItem {
            entry_id: self.entry_id,
            subject: self.subject,
            start,
            end,
            location: self.location,
            organizer: self.organizer,
            is_organizer: self.is_organizer,
            response_required: self.response_required,
            all_day: self.all_day,
        })
    }
}

fn parse_dt(s: &str) -> Result<DateTime<Local>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Local));
    }
    anyhow::bail!("日時を RFC3339 で指定してください: {s}")
}
