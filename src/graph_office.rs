//! Mail and calendar through Microsoft Graph when Outlook COM is unavailable.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, NaiveDate};

use crate::chat;
use crate::intake::model::{CalendarItem, MailItem};
use crate::intake::Source;

pub struct Loaded {
    pub mails: Vec<MailItem>,
    pub events: Vec<CalendarItem>,
}

impl Source for Loaded {
    fn name(&self) -> &'static str {
        "graph"
    }

    fn mails(&self, since: DateTime<Local>) -> Result<Vec<MailItem>> {
        Ok(self
            .mails
            .iter()
            .filter(|mail| mail.received_at >= since)
            .cloned()
            .collect())
    }

    fn events(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<CalendarItem>> {
        Ok(self
            .events
            .iter()
            .filter(|event| {
                let day = event.start.date_naive();
                day >= from && day <= to
            })
            .cloned()
            .collect())
    }
}

pub fn parse_messages(body: &str) -> Result<Vec<MailItem>> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let rows = value.get("value").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let subject = row.get("subject").and_then(|v| v.as_str()).unwrap_or("").trim();
        if subject.is_empty() {
            continue;
        }
        let Some(received_at) = row
            .get("receivedDateTime")
            .and_then(|v| v.as_str())
            .and_then(parse_time)
        else {
            continue;
        };
        let sender_name = row
            .pointer("/from/emailAddress/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let unread = !row.get("isRead").and_then(|v| v.as_bool()).unwrap_or(true);
        let flagged = row
            .pointer("/flag/flagStatus")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("flagged"));
        out.push(MailItem {
            entry_id: id.to_string(),
            subject: subject.to_string(),
            sender_name,
            received_at,
            unread,
            flagged,
            flag_request: None,
            body_excerpt: None,
            conversation_topic: None,
        });
    }
    Ok(out)
}

pub fn parse_events(body: &str) -> Result<Vec<CalendarItem>> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let rows = value.get("value").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let subject = row.get("subject").and_then(|v| v.as_str()).unwrap_or("").trim();
        if subject.is_empty() {
            continue;
        }
        let Some(start) = row.pointer("/start/dateTime").and_then(|v| v.as_str()).and_then(parse_time) else {
            continue;
        };
        let Some(end) = row.pointer("/end/dateTime").and_then(|v| v.as_str()).and_then(parse_time) else {
            continue;
        };
        out.push(CalendarItem {
            entry_id: id.to_string(),
            subject: subject.to_string(),
            start,
            end,
            location: row
                .pointer("/location/displayName")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            organizer: row
                .pointer("/organizer/emailAddress/name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            is_organizer: row.get("isOrganizer").and_then(|v| v.as_bool()).unwrap_or(false),
            response_required: row
                .get("responseRequested")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            all_day: row.get("isAllDay").and_then(|v| v.as_bool()).unwrap_or(false),
        });
    }
    Ok(out)
}

pub fn fetch(since: DateTime<Local>, from: NaiveDate, to: NaiveDate) -> Result<(Vec<MailItem>, Vec<CalendarItem>)> {
    let token = chat::token().ok_or_else(|| anyhow!("Graph のトークンがありません"))?;
    let mail_url = "https://graph.microsoft.com/v1.0/me/messages?$top=25&$orderby=receivedDateTime%20desc&$select=id,subject,from,receivedDateTime,isRead,flag";
    let start = from.format("%Y-%m-%dT00:00:00");
    let end = (to + chrono::Duration::days(1)).format("%Y-%m-%dT00:00:00");
    let cal_url = format!(
        "https://graph.microsoft.com/v1.0/me/calendarView?startDateTime={start}&endDateTime={end}&$select=id,subject,start,end,location,organizer,isOrganizer,responseRequested,isAllDay"
    );
    let mails = parse_messages(&get(mail_url, &token)?)?
        .into_iter()
        .filter(|mail| mail.received_at >= since)
        .collect();
    let events = parse_events(&get(&cal_url, &token)?)?;
    Ok((mails, events))
}

fn get(url: &str, token: &str) -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(8))
        .build();
    let response = agent
        .get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .set("User-Agent", "dayloop")
        .call()
        .map_err(|e| anyhow!("メールに聞けません: {e}"))?;
    Ok(response.into_string()?)
}

fn parse_time(text: &str) -> Option<DateTime<Local>> {
    let text = text.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.with_timezone(&Local));
    }
    let bare = text.trim_end_matches('Z').trim_end_matches(".0000000");
    chrono::NaiveDateTime::parse_from_str(bare, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .and_then(|ndt| ndt.and_local_timezone(Local).single())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flagged_mail_and_meeting_parse() {
        let mails = parse_messages(
            r#"{"value":[{"id":"m1","subject":"お願い","isRead":false,"receivedDateTime":"2026-09-23T01:00:00Z","from":{"emailAddress":{"name":"田中"}},"flag":{"flagStatus":"flagged"}}]}"#,
        )
        .unwrap();
        assert_eq!(mails[0].sender_name, "田中");
        assert!(mails[0].flagged);
        let events = parse_events(
            r#"{"value":[{"id":"c1","subject":"定例","start":{"dateTime":"2026-09-23T10:00:00Z"},"end":{"dateTime":"2026-09-23T11:00:00Z"},"isOrganizer":true,"responseRequested":true}]}"#,
        )
        .unwrap();
        assert!(events[0].is_organizer);
        assert_eq!(events[0].subject, "定例");
    }
}
