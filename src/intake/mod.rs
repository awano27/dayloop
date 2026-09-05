pub mod fixture;
pub mod model;
pub mod outlook_com;
pub mod rules;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Local, NaiveDate};
use serde::Serialize;
use serde_json::{json, Value};

use crate::business::{Category, FetchReport, FetchReportInput, FetchStatus};
use crate::config::{Config, IntakeConfig};
use crate::markdown;
use crate::model::Event;
use crate::store::Store;
use crate::util;

use fixture::FixtureSource;
use model::{CalendarItem, MailItem};
use outlook_com::OutlookCom;
use rules::{event_prep_hit, mail_hit};

const MAX_LOOKBACK_DAYS: i64 = 365;

struct ReportDetails {
    category: Category,
    source: String,
    scope: String,
    status: FetchStatus,
    item_count: usize,
    started: DateTime<Local>,
    finished: DateTime<Local>,
    reason: Option<String>,
}

pub trait Source {
    fn name(&self) -> &'static str;
    fn mails(&self, since: DateTime<Local>) -> Result<Vec<MailItem>>;
    fn events(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<CalendarItem>>;
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub name: String,
    pub ok: bool,
    pub status: FetchStatus,
    pub candidates_added: usize,
    pub candidates_skipped: usize,
    pub events: usize,
    pub fetch_reports: Vec<FetchReport>,
    pub error: Option<String>,
}

impl SyncResult {
    pub fn summary_line(&self) -> String {
        match self.status {
            FetchStatus::Success => {
                let mut summary = format!(
                    "候補 +{}（重複 {}）、予定 {} 件",
                    self.candidates_added, self.candidates_skipped, self.events
                );
                if let Some(error) = &self.error {
                    summary.push_str(&format!("（取得後の処理: {error}）"));
                }
                summary
            }
            FetchStatus::Partial => format!(
                "一部取得: 候補 +{}（重複 {}）、予定 {} 件（{}）",
                self.candidates_added,
                self.candidates_skipped,
                self.events,
                self.error.as_deref().unwrap_or("partial")
            ),
            FetchStatus::Disabled => format!("{} 取り込みは無効です", self.name),
            FetchStatus::Unavailable => format!(
                "{} デスクトップが利用できません: {}",
                self.name,
                self.error.as_deref().unwrap_or("unavailable")
            ),
            FetchStatus::Failed | FetchStatus::Stale => format!(
                "{} 取り込みに失敗しました: {}",
                self.name,
                self.error.as_deref().unwrap_or("failed")
            ),
        }
    }

    pub fn serve_line(&self) -> String {
        match self.status {
            FetchStatus::Success => format!(
                "intake {} ok +{} cand, {} events",
                self.name, self.candidates_added, self.events
            ),
            _ => format!(
                "intake {} {}: {}",
                self.name,
                self.status.as_str(),
                self.error.as_deref().unwrap_or("unknown")
            ),
        }
    }
}

pub fn ingest(
    store: &Store,
    src: &dyn Source,
    cfg: &IntakeConfig,
    since: DateTime<Local>,
    now: DateTime<Local>,
    dry_run: bool,
) -> Result<SyncResult> {
    let mail_started = Local::now();
    let mails = src.mails(since);
    let mail_finished = Local::now();
    let from = now.date_naive();
    let to = from + Duration::days(1);
    let calendar_started = Local::now();
    let calendars = src.events(from, to);
    let calendar_finished = Local::now();

    let mail_status = branch_status(&mails);
    let calendar_status = branch_status(&calendars);
    let status = aggregate_status(mail_status, calendar_status);
    let mail_report = report_input(ReportDetails {
        category: Category::Outlook,
        source: src.name().to_string(),
        scope: format!("mails since={}", since.to_rfc3339()),
        status: mail_status,
        item_count: mails.as_ref().map_or(0, Vec::len),
        started: mail_started,
        finished: mail_finished,
        reason: branch_reason("mails", mail_status),
    });
    let calendar_report = report_input(ReportDetails {
        category: Category::MeetingPrep,
        source: src.name().to_string(),
        scope: format!("calendar {}..{}", from, to),
        status: calendar_status,
        item_count: calendars.as_ref().map_or(0, Vec::len),
        started: calendar_started,
        finished: calendar_finished,
        reason: branch_reason("calendar", calendar_status),
    });

    let mut added = 0usize;
    let mut skipped = 0usize;
    let mut event_count = 0usize;
    let mut fetch_reports = Vec::new();
    let mut export_dates = Vec::new();
    if dry_run {
        if let Ok(mails) = &mails {
            for mail in mails {
                let mail = mail_without_body(mail, cfg.read_body);
                if let Some(hit) = mail_hit(&mail, cfg) {
                    println!("  [{}] {}", hit.reason, hit.title);
                    added += 1;
                }
            }
        }
        if let Ok(calendars) = &calendars {
            for calendar in calendars {
                if let Some(hit) = event_prep_hit(calendar, now, cfg) {
                    println!("  [{}] {}", hit.reason, hit.title);
                    added += 1;
                }
            }
            for calendar in calendars {
                let date = calendar.start.date_naive().format("%Y-%m-%d").to_string();
                println!("  [event] {} {}", calendar.start, calendar.subject);
                if date == from.format("%Y-%m-%d").to_string()
                    || date == to.format("%Y-%m-%d").to_string()
                {
                    event_count += 1;
                }
            }
        }
        fetch_reports.push(report_without_write(mail_report));
        fetch_reports.push(report_without_write(calendar_report));
    } else {
        let observation_time = now.to_rfc3339();
        let day_events: Vec<(String, Vec<Event>)> = calendars
            .as_ref()
            .map(|calendars| {
                [from, to]
                    .into_iter()
                    .map(|day| {
                        let date = day.format("%Y-%m-%d").to_string();
                        let events = calendars
                            .iter()
                            .filter(|calendar| calendar.start.date_naive() == day)
                            .map(|calendar| calendar_to_event(calendar, src.name(), &date))
                            .collect();
                        (date, events)
                    })
                    .collect()
            })
            .unwrap_or_default();
        event_count = day_events.iter().map(|(_, events)| events.len()).sum();
        export_dates = day_events.iter().map(|(date, _)| date.clone()).collect();
        let (mail_report, calendar_report, added_count, skipped_count) = store.atomic(|| {
            let mut added = 0usize;
            let mut skipped = 0usize;
            if let Ok(mails) = &mails {
                for mail in mails {
                    let mail = mail_without_body(mail, cfg.read_body);
                    let hit = mail_hit(&mail, cfg);
                    let source_ref = hit
                        .as_ref()
                        .map(|hit| hit.source_ref.clone())
                        .unwrap_or_else(|| format!("outlook:mail:{}", mail.entry_id));
                    let title = hit
                        .as_ref()
                        .map(|hit| hit.title.clone())
                        .unwrap_or_else(|| {
                            let subject = mail.subject.trim();
                            if subject.is_empty() {
                                mail.entry_id.clone()
                            } else {
                                subject.to_string()
                            }
                        });
                    let body = if cfg.read_body {
                        mail.body_excerpt.as_deref()
                    } else {
                        None
                    };
                    let observation = store.add_observation(
                        Category::Outlook,
                        &source_ref,
                        None,
                        &title,
                        body,
                        &observation_time,
                    )?;
                    store.link_source_observation(&source_ref, &observation.id)?;
                    if let Some(hit) = hit {
                        match store.add_candidate(&hit.title, "outlook", Some(&source_ref))? {
                            Some(_) => added += 1,
                            None => skipped += 1,
                        }
                    }
                }
            }
            if let Ok(calendars) = &calendars {
                for calendar in calendars {
                    let hit = event_prep_hit(calendar, now, cfg);
                    let source_ref = hit
                        .as_ref()
                        .map(|hit| hit.source_ref.clone())
                        .unwrap_or_else(|| format!("outlook:cal:{}:prep", calendar.entry_id));
                    let title = hit
                        .as_ref()
                        .map(|hit| hit.title.clone())
                        .unwrap_or_else(|| {
                            let subject = calendar.subject.trim();
                            if subject.is_empty() {
                                calendar.entry_id.clone()
                            } else {
                                subject.to_string()
                            }
                        });
                    let observation = store.add_observation(
                        Category::MeetingPrep,
                        &source_ref,
                        Some(&calendar.entry_id),
                        &title,
                        None,
                        &observation_time,
                    )?;
                    store.link_source_observation(&source_ref, &observation.id)?;
                    if let Some(hit) = hit {
                        match store.add_candidate(&hit.title, "outlook", Some(&hit.source_ref))? {
                            Some(_) => added += 1,
                            None => skipped += 1,
                        }
                    }
                }
            }
            for (date, events) in &day_events {
                store.upsert_events_from_source(date, src.name(), events.clone())?;
            }
            let mail_report = store.record_fetch_report(mail_report)?;
            let calendar_report = store.record_fetch_report(calendar_report)?;
            Ok((mail_report, calendar_report, added, skipped))
        })?;
        fetch_reports.push(mail_report);
        fetch_reports.push(calendar_report);
        added = added_count;
        skipped = skipped_count;
    }

    let mut export_error = None;
    if !dry_run && calendars.is_ok() {
        for date in &export_dates {
            if markdown::export(store, date).is_err() {
                export_error = Some("markdown_export_failed".to_string());
            }
        }
    }

    Ok(SyncResult {
        name: src.name().to_string(),
        ok: status == FetchStatus::Success && export_error.is_none(),
        status,
        candidates_added: added,
        candidates_skipped: skipped,
        events: event_count,
        fetch_reports,
        error: aggregate_error(mail_status, calendar_status).or(export_error),
    })
}

fn mail_without_body(mail: &MailItem, allow_body: bool) -> MailItem {
    let mut mail = mail.clone();
    if !allow_body {
        mail.body_excerpt = None;
    }
    mail
}

fn calendar_to_event(c: &CalendarItem, source: &str, date: &str) -> Event {
    Event {
        entry_id: c.entry_id.clone(),
        date: date.to_string(),
        start: c.start.to_rfc3339(),
        end: c.end.to_rfc3339(),
        subject: c.subject.clone(),
        location: c.location.clone(),
        organizer: c.organizer.clone(),
        is_organizer: c.is_organizer,
        source: source.to_string(),
        synced_at: util::now(),
    }
}

pub fn run_outlook(store: &Store, cfg: &Config, since: Option<&str>, dry_run: bool) -> SyncResult {
    if !cfg.intake.outlook {
        return disabled_result(store, "outlook", dry_run);
    }
    let since_dt = match parse_since(since, cfg.intake.lookback_days) {
        Ok(d) => d,
        Err(_) => return failed_result("outlook", FetchStatus::Failed, "invalid_since"),
    };
    let src = OutlookCom::new(cfg.intake.read_body);
    match ingest(store, &src, &cfg.intake, since_dt, Local::now(), dry_run) {
        Ok(result) => result,
        Err(_) => failed_result("outlook", FetchStatus::Failed, "ingest_failed"),
    }
}

pub fn run_fixture(store: &Store, dir: &std::path::Path, cfg: &Config) -> Result<SyncResult> {
    let mut intake = cfg.intake.clone();
    let overlay = dir.join("intake.toml");
    if overlay.exists() {
        let text = std::fs::read_to_string(&overlay)?;
        intake = toml::from_str::<IntakeConfig>(&text)?;
    }
    validate_lookback_days(intake.lookback_days)?;
    let src = FixtureSource::load(dir)?;
    let since = Local::now() - Duration::days(intake.lookback_days.max(30));
    ingest(store, &src, &intake, since, Local::now(), false)
}

pub fn sync_all(store: &Store, cfg: &Config) -> Vec<SyncResult> {
    vec![run_outlook(store, cfg, None, false)]
}

pub fn sync_all_json(store: &Store, cfg: &Config) -> Value {
    let results = sync_all(store, cfg);
    let sources: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "ok": r.ok,
                "status": r.status,
                "candidates_added": r.candidates_added,
                "events": r.events,
                "fetch_reports": r.fetch_reports,
            })
        })
        .collect();
    let errors: Vec<Value> = results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| {
            json!({
                "name": r.name,
                "status": r.status,
                "error": r.error,
            })
        })
        .collect();
    json!({ "sources": sources, "errors": errors })
}

fn branch_status<T>(result: &Result<Vec<T>>) -> FetchStatus {
    match result {
        Ok(_) => FetchStatus::Success,
        Err(error) => classify_error(error),
    }
}

fn classify_error(error: &anyhow::Error) -> FetchStatus {
    let lower = error.to_string().to_ascii_lowercase();
    if lower.contains("unavailable")
        || lower.contains("new_outlook_or_missing")
        || lower.contains("not available")
    {
        FetchStatus::Unavailable
    } else {
        FetchStatus::Failed
    }
}

fn aggregate_status(mail: FetchStatus, calendar: FetchStatus) -> FetchStatus {
    if mail == FetchStatus::Success && calendar == FetchStatus::Success {
        FetchStatus::Success
    } else if mail == FetchStatus::Success || calendar == FetchStatus::Success {
        FetchStatus::Partial
    } else if mail == FetchStatus::Disabled && calendar == FetchStatus::Disabled {
        FetchStatus::Disabled
    } else if mail == FetchStatus::Unavailable && calendar == FetchStatus::Unavailable {
        FetchStatus::Unavailable
    } else {
        FetchStatus::Failed
    }
}

fn branch_reason(scope: &str, status: FetchStatus) -> Option<String> {
    match status {
        FetchStatus::Success => None,
        FetchStatus::Disabled => Some("disabled".into()),
        FetchStatus::Unavailable => Some(format!("{scope}_source_unavailable")),
        FetchStatus::Failed | FetchStatus::Partial | FetchStatus::Stale => {
            Some(format!("{scope}_fetch_failed"))
        }
    }
}

fn aggregate_error(mail: FetchStatus, calendar: FetchStatus) -> Option<String> {
    let mut errors = Vec::new();
    if mail != FetchStatus::Success {
        errors.push(format!("mails_{}", mail.as_str()));
    }
    if calendar != FetchStatus::Success {
        errors.push(format!("calendar_{}", calendar.as_str()));
    }
    (!errors.is_empty()).then(|| errors.join(","))
}

fn report_input(details: ReportDetails) -> FetchReportInput {
    FetchReportInput {
        category: details.category,
        source: details.source,
        scope: details.scope,
        status: details.status,
        item_count: details.item_count as i64,
        started_at: details.started.to_rfc3339(),
        finished_at: details.finished.to_rfc3339(),
        reason: details.reason,
    }
}

fn report_without_write(input: FetchReportInput) -> FetchReport {
    FetchReport {
        id: ulid::Ulid::new().to_string(),
        category: input.category,
        source: input.source,
        scope: input.scope,
        status: input.status,
        item_count: input.item_count,
        started_at: input.started_at,
        finished_at: input.finished_at,
        reason: input.reason,
    }
}

fn disabled_result(store: &Store, name: &str, dry_run: bool) -> SyncResult {
    let started = Local::now();
    let finished = Local::now();
    let mail = report_input(ReportDetails {
        category: Category::Outlook,
        source: name.into(),
        scope: "mails disabled".into(),
        status: FetchStatus::Disabled,
        item_count: 0,
        started,
        finished,
        reason: Some("disabled".into()),
    });
    let calendar = report_input(ReportDetails {
        category: Category::MeetingPrep,
        source: name.into(),
        scope: "calendar disabled".into(),
        status: FetchStatus::Disabled,
        item_count: 0,
        started,
        finished,
        reason: Some("disabled".into()),
    });
    let fetch_reports = if dry_run {
        vec![report_without_write(mail), report_without_write(calendar)]
    } else {
        match store.atomic(|| {
            Ok(vec![
                store.record_fetch_report(mail)?,
                store.record_fetch_report(calendar)?,
            ])
        }) {
            Ok(reports) => reports,
            Err(_) => {
                return failed_result(name, FetchStatus::Failed, "fetch_report_save_failed");
            }
        }
    };
    SyncResult {
        name: name.into(),
        ok: false,
        status: FetchStatus::Disabled,
        candidates_added: 0,
        candidates_skipped: 0,
        events: 0,
        fetch_reports,
        error: Some("outlook_disabled".into()),
    }
}

fn failed_result(name: &str, status: FetchStatus, error: &str) -> SyncResult {
    SyncResult {
        name: name.into(),
        ok: false,
        status,
        candidates_added: 0,
        candidates_skipped: 0,
        events: 0,
        fetch_reports: Vec::new(),
        error: Some(error.into()),
    }
}

fn validate_lookback_days(days: i64) -> Result<()> {
    if !(0..=MAX_LOOKBACK_DAYS).contains(&days) {
        return Err(anyhow!(
            "lookback_days は0〜{MAX_LOOKBACK_DAYS}の範囲で指定してください"
        ));
    }
    Ok(())
}

fn parse_since(spec: Option<&str>, lookback_days: i64) -> Result<DateTime<Local>> {
    match spec {
        None => {
            validate_lookback_days(lookback_days)?;
            Ok(Local::now() - Duration::days(lookback_days))
        }
        Some(s) => {
            if let Some(rest) = s.strip_suffix('d').or_else(|| s.strip_suffix('D')) {
                let n: i64 = rest.parse()?;
                validate_lookback_days(n)?;
                Ok(Local::now() - Duration::days(n))
            } else {
                let d = crate::util::parse_date(s)?;
                d.and_hms_opt(0, 0, 0)
                    .and_then(|ndt| ndt.and_local_timezone(Local).earliest())
                    .ok_or_else(|| anyhow::anyhow!("since を解釈できません: {s}"))
            }
        }
    }
}
