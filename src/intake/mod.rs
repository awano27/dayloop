pub mod chat_live;
pub mod devops_live;
pub mod recap;
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
        let source_ref = if src.name() == "graph" {
            format!("graph:mail:{}", m.entry_id)
        } else {
            hit.source_ref.clone()
        };
        let source_name = if src.name() == "graph" { "mail" } else { "outlook" };
        match store.add_candidate(&hit.title, source_name, Some(&source_ref))? {
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
        let source_ref = if src.name() == "graph" {
            format!("graph:cal:{}:prep", c.entry_id)
        } else {
            hit.source_ref.clone()
        };
        let source_name = if src.name() == "graph" { "meeting" } else { "outlook" };
        match store.add_candidate(&hit.title, source_name, Some(&source_ref))? {
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
        let outlook = run_outlook(store, cfg, None, false);
        if outlook.ok {
            out.push(outlook);
        } else if crate::chat::token().is_some() {
            out.push(outlook);
            out.push(run_graph(store, cfg));
        } else {
            out.push(outlook);
        }
    }
    out.extend(
        [run_github(store), run_jira(store), run_teams(store), run_devops(store)]
            .into_iter()
            .flatten(),
    );
    if let Some(inbox) = run_inbox(store) {
        out.push(inbox);
    }
    out
}

fn run_devops(store: &Store) -> Option<SyncResult> {
    if crate::devops::org().is_none() {
        return None;
    }
    match crate::devops::fetch_assigned() {
        Ok(items) => devops_live::ingest(store, &items).ok().map(|n| counted("devops", n)),
        Err(e) => Some(SyncResult {
            name: "devops".into(),
            ok: false,
            candidates_added: 0,
            candidates_skipped: 0,
            events: 0,
            error: Some(e.to_string()),
        }),
    }
}

fn run_inbox(store: &Store) -> Option<SyncResult> {
    let dir = crate::paths::inbox_dir();
    if !dir.is_dir() {
        return None;
    }
    let mut added = 0usize;
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !matches!(ext, "txt" | "md" | "html" | "htm") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let text = if ext.starts_with("htm") { strip_tags(&raw) } else { raw };
        let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let actions = recap::actions(&text);
        if actions.is_empty() {
            if let Ok(report) = minutes::ingest(store, &text) {
                added += report.actions;
            }
            continue;
        }
        for title in actions {
            let source_ref = format!("minutes:{name}:{title}");
            if store
                .add_candidate(&title, "meeting", Some(&source_ref))
                .ok()
                .flatten()
                .is_some()
            {
                added += 1;
            }
        }
    }
    Some(counted("minutes", added))
}

fn strip_tags(text: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for ch in text.chars() {
        match ch {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn run_graph(store: &Store, cfg: &Config) -> SyncResult {
    let now = Local::now();
    let since = now - Duration::days(cfg.intake.lookback_days.max(1));
    let from = now.date_naive();
    let to = from + Duration::days(1);
    match crate::graph_office::fetch(since, from, to) {
        Ok((mails, events)) => {
            let src = crate::graph_office::Loaded { mails, events };
            match ingest(store, &src, &cfg.intake, since, now, false) {
                Ok(mut result) => {
                    result.name = "mail".into();
                    result
                }
                Err(e) => SyncResult {
                    name: "mail".into(),
                    ok: false,
                    candidates_added: 0,
                    candidates_skipped: 0,
                    events: 0,
                    error: Some(e.to_string()),
                },
            }
        }
        Err(e) => SyncResult {
            name: "mail".into(),
            ok: false,
            candidates_added: 0,
            candidates_skipped: 0,
            events: 0,
            error: Some(e.to_string()),
        },
    }
}

pub fn link(store: &Store) -> Vec<SyncResult> {
    if cfg!(test) && std::env::var("DAYLOOP_LINK_LIVE").ok().as_deref() != Some("1") {
        return Vec::new();
    }
    let cfg = crate::config::load();
    let results = sync_all(store, &cfg);
    for result in &results {
        if result.candidates_added > 0 || result.events > 0 || !result.ok {
            println!("{}", result.serve_line());
            crate::serve::log_event(&result.serve_line());
        }
    }
    results
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
