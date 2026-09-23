pub mod chat_live;
pub mod fixture;
pub mod github_intake;
pub mod jira_live;
pub mod minutes;
pub mod model;
pub mod outlook_com;
pub mod rules;
pub mod teams;
pub mod tickets;

use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDate};
use serde_json::{json, Value};

use crate::config::{Config, IntakeConfig};
use crate::markdown;
use crate::model::Event;
use crate::store::Store;
use crate::util;

use fixture::FixtureSource;
use model::{CalendarItem, MailItem};
use outlook_com::OutlookCom;
use rules::{event_prep_hit, mail_hit};

pub trait Source {
    fn name(&self) -> &'static str;
    fn mails(&self, since: DateTime<Local>) -> Result<Vec<MailItem>>;
    fn events(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<CalendarItem>>;
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub name: String,
    pub ok: bool,
    pub candidates_added: usize,
    pub candidates_skipped: usize,
    pub events: usize,
    pub error: Option<String>,
}

impl SyncResult {
    pub fn summary_line(&self) -> String {
        if !self.ok {
            return format!(
                "Outlook デスクトップが利用できません: {}",
                self.error.as_deref().unwrap_or("unknown")
            );
        }
        format!(
            "候補 +{}（重複 {}）、予定 {} 件",
            self.candidates_added, self.candidates_skipped, self.events
        )
    }

    pub fn serve_line(&self) -> String {
        if self.ok {
            format!(
                "intake {} ok +{} cand, {} events",
                self.name, self.candidates_added, self.events
            )
        } else {
            format!(
                "intake {} skipped: {}",
                self.name,
                self.error.as_deref().unwrap_or("unknown")
            )
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
    let mails = src.mails(since)?;
    let from = now.date_naive();
    let to = from + Duration::days(1);
    let cals = src.events(from, to)?;

    let mut added = 0usize;
    let mut skipped = 0usize;
    for m in &mails {
        let Some(hit) = mail_hit(m, cfg) else { continue };
        if dry_run {
            println!("  [{}] {}", hit.reason, hit.title);
            added += 1;
            continue;
        }
        match store.add_candidate(&hit.title, "outlook", Some(&hit.source_ref))? {
            Some(_) => added += 1,
            None => skipped += 1,
        }
    }
    for c in &cals {
        let Some(hit) = event_prep_hit(c, now, cfg) else { continue };
        if dry_run {
            println!("  [{}] {}", hit.reason, hit.title);
            added += 1;
            continue;
        }
        match store.add_candidate(&hit.title, "outlook", Some(&hit.source_ref))? {
            Some(_) => added += 1,
            None => skipped += 1,
        }
    }

    let mut event_count = 0usize;
    for day in [from, to] {
        let ds = day.format("%Y-%m-%d").to_string();
        let evs: Vec<Event> = cals
            .iter()
            .filter(|c| c.start.date_naive() == day)
            .map(|c| calendar_to_event(c, src.name(), &ds))
            .collect();
        event_count += evs.len();
        if dry_run {
            for e in &evs {
                println!("  [event] {} {}", e.start, e.subject);
            }
            continue;
        }
        store.upsert_events(&ds, evs)?;
        markdown::export(store, &ds)?;
    }

    if !dry_run {
        let today = now.format("%Y-%m-%d").to_string();
        crate::graph::apply_known_candidates(store, &today)?;
        crate::jev::grow_if_configured(store, &today)?;
    }

    Ok(SyncResult {
        name: src.name().to_string(),
        ok: true,
        candidates_added: added,
        candidates_skipped: skipped,
        events: event_count,
        error: None,
    })
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
    let since_dt = match parse_since(since, cfg.intake.lookback_days) {
        Ok(d) => d,
        Err(e) => {
            return SyncResult {
                name: "outlook".into(),
                ok: false,
                candidates_added: 0,
                candidates_skipped: 0,
                events: 0,
                error: Some(e.to_string()),
            };
        }
    };
    let src = OutlookCom::new(cfg.intake.read_body);
    match ingest(store, &src, &cfg.intake, since_dt, Local::now(), dry_run) {
        Ok(r) => r,
        Err(e) => SyncResult {
            name: "outlook".into(),
            ok: false,
            candidates_added: 0,
            candidates_skipped: 0,
            events: 0,
            error: Some(e.to_string()),
        },
    }
}

pub fn run_fixture(store: &Store, dir: &std::path::Path, cfg: &Config) -> Result<SyncResult> {
    let mut intake = cfg.intake.clone();
    let overlay = dir.join("intake.toml");
    if overlay.exists() {
        if let Ok(text) = std::fs::read_to_string(&overlay) {
            if let Ok(over) = toml::from_str::<IntakeConfig>(&text) {
                intake = over;
            }
        }
    }
    let src = FixtureSource::load(dir)?;
    let since = Local::now() - Duration::days(intake.lookback_days.max(30));
    ingest(store, &src, &intake, since, Local::now(), false)
}

pub fn sync_all(store: &Store, cfg: &Config) -> Vec<SyncResult> {
    let mut out = Vec::new();
    if cfg.intake.outlook {
        out.push(run_outlook(store, cfg, None, false));
    }
    out.extend([run_github(store), run_jira(store), run_teams(store)].into_iter().flatten());
    out
}

fn counted(name: &str, added: usize) -> SyncResult {
    SyncResult {
        name: name.into(),
        ok: true,
        candidates_added: added,
        candidates_skipped: 0,
        events: 0,
        error: None,
    }
}

fn run_github(store: &Store) -> Option<SyncResult> {
    if crate::github::token().is_none() {
        return None;
    }
    match crate::github::fetch_assigned() {
        Ok(items) => github_intake::ingest(store, &items).ok().map(|n| counted("github", n)),
        Err(e) => Some(SyncResult {
            name: "github".into(),
            ok: false,
            candidates_added: 0,
            candidates_skipped: 0,
            events: 0,
            error: Some(e.to_string()),
        }),
    }
}

fn run_jira(store: &Store) -> Option<SyncResult> {
    if crate::jira::creds().is_none() {
        return None;
    }
    match crate::jira::fetch_assigned() {
        Ok(items) => jira_live::ingest(store, &items).ok().map(|n| counted("jira", n)),
        Err(e) => Some(SyncResult {
            name: "jira".into(),
            ok: false,
            candidates_added: 0,
            candidates_skipped: 0,
            events: 0,
            error: Some(e.to_string()),
        }),
    }
}

fn run_teams(store: &Store) -> Option<SyncResult> {
    if crate::chat::token().is_none() {
        return None;
    }
    match crate::chat::fetch_recent() {
        Ok(items) => chat_live::ingest(store, &items).ok().map(|n| counted("teams", n)),
        Err(e) => Some(SyncResult {
            name: "teams".into(),
            ok: false,
            candidates_added: 0,
            candidates_skipped: 0,
            events: 0,
            error: Some(e.to_string()),
        }),
    }
}

pub fn sync_all_json(store: &Store, cfg: &Config) -> Value {
    let results = sync_all(store, cfg);
    let sources: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "ok": r.ok,
                "candidates_added": r.candidates_added,
                "events": r.events,
            })
        })
        .collect();
    let errors: Vec<Value> = results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| {
            json!({
                "name": r.name,
                "error": r.error,
            })
        })
        .collect();
    json!({ "sources": sources, "errors": errors })
}

fn parse_since(spec: Option<&str>, lookback_days: i64) -> Result<DateTime<Local>> {
    match spec {
        None => Ok(Local::now() - Duration::days(lookback_days)),
        Some(s) => {
            if let Some(rest) = s.strip_suffix('d').or_else(|| s.strip_suffix('D')) {
                let n: i64 = rest.parse()?;
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
