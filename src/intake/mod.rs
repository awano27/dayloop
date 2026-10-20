pub mod chat_live;
pub mod devops_live;
pub mod fixture;
pub mod github_intake;
pub mod jira_live;
pub mod minutes;
pub mod model;
pub mod outlook_com;
pub mod recap;
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
    pub files_processed: usize,
    pub files_failed: usize,
    pub error: Option<String>,
}

impl SyncResult {
    /// Operational outcome; source coverage remains independently conservative.
    pub fn status(&self) -> &'static str {
        if self.ok {
            "success"
        } else if self.candidates_added
            + self.candidates_skipped
            + self.events
            + self.files_processed
            > 0
        {
            "partial"
        } else {
            "failed"
        }
    }

    pub fn summary_line(&self) -> String {
        format!(
            "{} {}: 候補 +{}（重複 {}）、予定 {} 件、ファイル成功 {} / 失敗 {}{}",
            self.name,
            self.status(),
            self.candidates_added,
            self.candidates_skipped,
            self.events,
            self.files_processed,
            self.files_failed,
            self.error
                .as_ref()
                .map(|e| format!("; {e}"))
                .unwrap_or_default()
        )
    }

    pub fn serve_line(&self) -> String {
        format!("intake {}", self.summary_line())
    }

    fn fail(&mut self, stage: &str, detail: &str) {
        self.ok = false;
        let message = format!("{stage}: {detail}");
        match &mut self.error {
            Some(error) => {
                error.push_str("; ");
                error.push_str(&message);
            }
            None => self.error = Some(message),
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
        let Some(hit) = mail_hit(m, cfg) else {
            continue;
        };
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
        let source_name = if src.name() == "graph" {
            "mail"
        } else {
            "outlook"
        };
        match store.add_candidate(&hit.title, source_name, Some(&source_ref))? {
            Some(_) => added += 1,
            None => skipped += 1,
        }
    }
    for c in &cals {
        let Some(hit) = event_prep_hit(c, now, cfg) else {
            continue;
        };
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
        let source_name = if src.name() == "graph" {
            "meeting"
        } else {
            "outlook"
        };
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
        markdown::export_if_open(store, &ds)?;
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
        files_processed: 0,
        files_failed: 0,
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
                files_processed: 0,
                files_failed: 0,
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
            files_processed: 0,
            files_failed: 0,
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
    let src = match FixtureSource::load(dir) {
        Ok(source) => source,
        Err(error) => {
            let failed = SyncResult {
                name: "fixture".into(),
                ok: false,
                candidates_added: 0,
                candidates_skipped: 0,
                events: 0,
                files_processed: 0,
                files_failed: 0,
                error: Some(error.to_string()),
            };
            record_sync(store, &failed, "failed")?;
            return Err(error);
        }
    };
    let since = Local::now() - Duration::days(intake.lookback_days.max(30));
    let mut result = match ingest(store, &src, &intake, since, Local::now(), false) {
        Ok(result) => result,
        Err(error) => {
            let failed = SyncResult {
                name: "fixture".into(),
                ok: false,
                candidates_added: 0,
                candidates_skipped: 0,
                events: 0,
                files_processed: 0,
                files_failed: 0,
                error: Some(error.to_string()),
            };
            record_sync(store, &failed, "partial")?;
            return Err(error);
        }
    };
    let updates = dir.join("commitments.json");
    if updates.exists() {
        let imported = (|| -> Result<()> {
            let text = std::fs::read_to_string(&updates)?;
            let rows: Vec<Value> = serde_json::from_str(&text)?;
            for row in rows {
                crate::commitment::dispatch(store, "commitment_ingest", &row)?;
            }
            Ok(())
        })();
        if let Err(error) = imported {
            result.ok = false;
            result.error = Some(format!("約束fixtureは部分取得/取込失敗: {error}"));
        }
    }
    record_sync(
        store,
        &result,
        if result.ok { "success" } else { "partial" },
    )?;
    Ok(result)
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
        [
            run_github(store),
            run_jira(store),
            run_teams(store),
            run_devops(store),
        ]
        .into_iter()
        .flatten(),
    );
    if let Some(inbox) = run_inbox(store) {
        out.push(inbox);
    }
    for result in &mut out {
        // Existing adapters may cap their result set. Success does not prove
        // complete coverage of replies or changes.
        record_result(store, result);
    }
    out
}

fn record_sync(store: &Store, result: &SyncResult, status: &str) -> Result<()> {
    crate::commitment::dispatch(
        store,
        "commitment_sync",
        &json!({
            "op_id": ulid::Ulid::new().to_string(),
            "source": result.name,
            "status": status,
            "at": chrono::Utc::now().to_rfc3339(),
            "evidence": format!("候補追加={},重複={},予定={},ファイル成功={},ファイル失敗={},取得範囲の完全性={},詳細={}",
                result.candidates_added, result.candidates_skipped, result.events,
                result.files_processed, result.files_failed,
                if status == "success" { "fixture範囲内" } else { "未確認" },
                result.error.as_deref().unwrap_or("adapter returned")),
        }),
    )?;
    Ok(())
}

#[derive(Default)]
pub(super) struct BatchCounts {
    added: usize,
    skipped: usize,
}

/// Each fetched batch commits completely or leaves no candidate writes.
pub(super) fn ingest_candidates<'a>(
    store: &Store,
    source: &str,
    items: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<BatchCounts> {
    let tx = store.connection().unchecked_transaction()?;
    let mut counts = BatchCounts::default();
    for (title, source_ref) in items {
        if store
            .add_candidate(title, source, Some(source_ref))?
            .is_some()
        {
            counts.added += 1;
        } else {
            counts.skipped += 1;
        }
    }
    tx.commit()?;
    Ok(counts)
}

fn run_adapter<T>(
    name: &str,
    configured: bool,
    fetch: impl FnOnce() -> Result<T>,
    ingest: impl FnOnce(T) -> Result<BatchCounts>,
) -> Option<SyncResult> {
    if !configured {
        return None;
    }
    let mut result = counted(name, 0);
    match fetch() {
        Ok(items) => match ingest(items) {
            Ok(counts) => {
                result.candidates_added = counts.added;
                result.candidates_skipped = counts.skipped;
            }
            Err(error) => result.fail(
                "store",
                &format!(
                    "candidate batch failed; rolled back ({})",
                    safe_error_kind(&error)
                ),
            ),
        },
        // Provider errors may contain tokens, response bodies or URLs.
        Err(_) => result.fail("fetch", "source retrieval failed"),
    }
    Some(result)
}

/// Include only machine error categories, never SQL, paths, response bodies or credentials.
fn safe_error_kind(error: &anyhow::Error) -> String {
    for cause in error.chain() {
        if let Some(rusqlite::Error::SqliteFailure(code, _)) =
            cause.downcast_ref::<rusqlite::Error>()
        {
            return format!("sqlite:{:?}", code.code);
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return format!("io:{:?}", io.kind());
        }
    }
    "validation_or_storage_error".into()
}

fn record_result(store: &Store, result: &mut SyncResult) {
    persist_result(result, |result, status| record_sync(store, result, status));
}

fn persist_result(result: &mut SyncResult, save: impl FnOnce(&SyncResult, &str) -> Result<()>) {
    let status = if result.ok || result.status() == "partial" {
        "partial"
    } else {
        "failed"
    };
    if let Err(error) = save(result, status) {
        result.fail(
            "sync_record",
            &format!(
                "synchronization record could not be saved ({})",
                safe_error_kind(&error)
            ),
        );
    }
}

fn run_devops(store: &Store) -> Option<SyncResult> {
    run_adapter(
        "devops",
        crate::devops::org().is_some(),
        crate::devops::fetch_assigned,
        |items| devops_live::ingest_report(store, &items),
    )
}

/// Read the configured inbox without contacting any external service.
pub fn run_inbox(store: &Store) -> Option<SyncResult> {
    let dir = crate::paths::inbox_dir();
    match std::fs::symlink_metadata(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        _ => {}
    }
    let mut result = counted("minutes", 0);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) => {
            result.fail(
                "enumerate",
                &format!("inbox directory unavailable ({:?})", e.kind()),
            );
            return Some(result);
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                result.fail(
                    "enumerate",
                    &format!("inbox entry unavailable ({:?})", e.kind()),
                );
                continue;
            }
        };
        let path = entry.path();
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => continue,
            Err(e) => {
                result.files_failed += 1;
                result.fail(
                    "read",
                    &format!("file metadata unavailable ({:?})", e.kind()),
                );
                continue;
            }
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !matches!(ext, "txt" | "md" | "html" | "htm") {
            continue;
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) => {
                result.files_failed += 1;
                result.fail(
                    "read",
                    &format!("file could not be read as UTF-8 ({:?})", e.kind()),
                );
                continue;
            }
        };
        let text = if ext.starts_with("htm") {
            strip_tags(&raw)
        } else {
            raw
        };
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let actions = recap::actions(&text);
        let saved = if actions.is_empty() {
            minutes::ingest(store, &text).map(|report| BatchCounts {
                added: report.actions,
                skipped: report.skipped,
            })
        } else {
            let refs: Vec<String> = actions
                .iter()
                .map(|title| format!("minutes:{name}:{title}"))
                .collect();
            ingest_candidates(
                store,
                "meeting",
                actions
                    .iter()
                    .zip(&refs)
                    .map(|(title, reference)| (title.as_str(), reference.as_str())),
            )
        };
        match saved {
            Ok(counts) => {
                result.files_processed += 1;
                result.candidates_added += counts.added;
                result.candidates_skipped += counts.skipped;
            }
            Err(error) => {
                result.files_failed += 1;
                result.fail(
                    "store",
                    &format!(
                        "file candidate batch failed; rolled back ({})",
                        safe_error_kind(&error)
                    ),
                );
            }
        }
    }
    Some(result)
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
                    files_processed: 0,
                    files_failed: 0,
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
            files_processed: 0,
            files_failed: 0,
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
        files_processed: 0,
        files_failed: 0,
        error: None,
    }
}

fn run_github(store: &Store) -> Option<SyncResult> {
    run_adapter(
        "github",
        crate::github::token().is_some(),
        crate::github::fetch_assigned,
        |items| github_intake::ingest_report(store, &items),
    )
}

fn run_jira(store: &Store) -> Option<SyncResult> {
    run_adapter(
        "jira",
        crate::jira::creds().is_some(),
        crate::jira::fetch_assigned,
        |items| jira_live::ingest_report(store, &items),
    )
}

fn run_teams(store: &Store) -> Option<SyncResult> {
    run_adapter(
        "teams",
        crate::chat::token().is_some(),
        crate::chat::fetch_recent,
        |items| chat_live::ingest_report(store, &items),
    )
}

pub fn sync_all_json(store: &Store, cfg: &Config) -> Value {
    results_json(&sync_all(store, cfg))
}

pub fn results_json(results: &[SyncResult]) -> Value {
    let sources: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "ok": r.ok,
                "status": r.status(),
                "error": r.error,
                "candidates_added": r.candidates_added,
                "candidates_skipped": r.candidates_skipped,
                "events": r.events,
                "files_processed": r.files_processed,
                "files_failed": r.files_failed,
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

#[cfg(test)]
mod sync_result_tests {
    use super::*;

    #[test]
    fn fetched_adapter_failure_is_retained_and_sanitized_for_every_source() {
        for name in ["github", "devops", "jira", "teams"] {
            let failed = run_adapter(
                name,
                true,
                || Ok(vec!["synthetic"]),
                |_| {
                    Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(19),
                        Some("secret-body".into()),
                    )
                    .into())
                },
            )
            .unwrap();
            assert_eq!(failed.name, name);
            assert_eq!(failed.status(), "failed");
            assert_eq!(failed.candidates_added, 0);
            assert!(failed.error.as_deref().unwrap().contains("store"));
            assert!(failed
                .error
                .as_deref()
                .unwrap()
                .contains("ConstraintViolation"));
            assert!(!failed.error.as_deref().unwrap().contains("secret-body"));
            let json = results_json(&[failed]);
            assert_eq!(json["sources"].as_array().unwrap().len(), 1);
            assert_eq!(json["errors"].as_array().unwrap().len(), 1);
            assert_eq!(json["sources"][0]["status"], "failed");
        }
    }

    #[test]
    fn unconfigured_zero_and_fetch_failure_are_distinct() {
        let skipped = run_adapter::<()>(
            "github",
            false,
            || panic!("must not fetch"),
            |_| panic!("must not ingest"),
        );
        assert!(skipped.is_none());
        let empty = run_adapter("github", true, || Ok(()), |_| Ok(BatchCounts::default())).unwrap();
        assert_eq!(empty.status(), "success");
        assert_eq!(empty.candidates_added, 0);
        let failed = run_adapter::<()>(
            "github",
            true,
            || anyhow::bail!("secret token"),
            |_| panic!("must not ingest"),
        )
        .unwrap();
        assert_eq!(failed.status(), "failed");
        assert_eq!(
            failed.error.as_deref(),
            Some("fetch: source retrieval failed")
        );
    }

    #[test]
    fn ledger_failure_appends_to_import_failure_and_preserves_counts() {
        let mut result = counted("minutes", 2);
        result.fail("read", "invalid UTF-8");
        persist_result(&mut result, |r, status| {
            assert_eq!(r.candidates_added, 2);
            assert_eq!(status, "partial");
            Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(8),
                Some("secret path".into()),
            )
            .into())
        });
        assert_eq!(result.status(), "partial");
        let error = result.error.as_deref().unwrap();
        assert!(error.contains("read:"));
        assert!(error.contains("sync_record:"));
        assert!(error.contains("ReadOnly"));
        assert!(!error.contains("secret"));
        assert!(result.serve_line().contains("候補 +2"));
    }

    #[test]
    fn operational_zero_keeps_conservative_coverage_and_failure_is_recorded() {
        for (ok, expected) in [(true, "partial"), (false, "failed")] {
            let mut result = counted("github", 0);
            result.ok = ok;
            persist_result(&mut result, |_, status| {
                assert_eq!(status, expected);
                Ok(())
            });
            assert_eq!(result.ok, ok);
        }
    }
}
