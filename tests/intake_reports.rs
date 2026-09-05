use std::path::PathBuf;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use dayloop::business::FetchStatus;
use dayloop::config::{Config, IntakeConfig};
use dayloop::intake::model::{CalendarItem, MailItem};
use dayloop::intake::{self, Source, SyncResult};
use dayloop::model::Event;
use dayloop::store::Store;

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("dayloop-intake-reports-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn store(&self) -> Store {
        Store::open_at(self.0.join("dayloop.db")).unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct MockSource {
    mails: Result<Vec<MailItem>, String>,
    events: Result<Vec<CalendarItem>, String>,
}

impl MockSource {
    fn new(
        mails: Result<Vec<MailItem>, String>,
        events: Result<Vec<CalendarItem>, String>,
    ) -> Self {
        Self { mails, events }
    }
}

impl Source for MockSource {
    fn name(&self) -> &'static str {
        "mock-outlook"
    }

    fn mails(&self, _since: DateTime<Local>) -> Result<Vec<MailItem>> {
        self.mails.clone().map_err(|error| anyhow!(error))
    }

    fn events(&self, _from: NaiveDate, _to: NaiveDate) -> Result<Vec<CalendarItem>> {
        self.events.clone().map_err(|error| anyhow!(error))
    }
}

fn now() -> DateTime<Local> {
    Local
        .with_ymd_and_hms(2026, 9, 7, 9, 0, 0)
        .single()
        .unwrap()
}

fn config() -> IntakeConfig {
    IntakeConfig {
        keywords: vec!["対応".into()],
        important_senders: vec!["田中".into()],
        lookback_days: 30,
        ..IntakeConfig::default()
    }
}

fn mail(entry_id: &str, body_excerpt: Option<&str>) -> MailItem {
    MailItem {
        entry_id: entry_id.into(),
        subject: "対応してください".into(),
        sender_name: "田中".into(),
        received_at: now(),
        unread: true,
        flagged: false,
        flag_request: None,
        body_excerpt: body_excerpt.map(str::to_owned),
        conversation_topic: None,
    }
}

fn event(entry_id: &str) -> CalendarItem {
    CalendarItem {
        entry_id: entry_id.into(),
        subject: "週次会議".into(),
        start: now() + chrono::Duration::hours(2),
        end: now() + chrono::Duration::hours(3),
        location: Some("会議室".into()),
        organizer: Some("田中".into()),
        is_organizer: true,
        response_required: true,
        all_day: false,
    }
}

fn stored_event(entry_id: &str, source: &str) -> Event {
    Event {
        entry_id: entry_id.into(),
        date: "2026-09-07".into(),
        start: (now() + chrono::Duration::hours(4)).to_rfc3339(),
        end: (now() + chrono::Duration::hours(5)).to_rfc3339(),
        subject: format!("{source} event"),
        location: None,
        organizer: None,
        is_organizer: false,
        source: source.into(),
        synced_at: now().to_rfc3339(),
    }
}

fn run(store: &Store, source: &MockSource, cfg: &IntakeConfig, dry_run: bool) -> SyncResult {
    intake::ingest(
        store,
        source,
        cfg,
        now() - chrono::Duration::days(30),
        now(),
        dry_run,
    )
    .unwrap()
}

#[test]
fn mail_success_calendar_failure_is_partial_and_keeps_mail_data() {
    let home = Home::new();
    let store = home.store();
    let source = MockSource::new(
        Ok(vec![mail("mail-1", None)]),
        Err("calendar secret=do-not-echo".into()),
    );

    let result = run(&store, &source, &config(), false);

    assert_eq!(result.status, FetchStatus::Partial);
    assert!(!result.ok);
    assert_eq!(result.candidates_added, 1);
    assert_eq!(store.open_candidates().unwrap().len(), 1);
    assert!(store.events_for_day("2026-09-07").unwrap().is_empty());
    assert!(!result
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("secret"));
    assert_eq!(result.fetch_reports.len(), 2);
    assert_eq!(result.fetch_reports[0].status, FetchStatus::Success);
    assert_eq!(result.fetch_reports[0].item_count, 1);
    assert_eq!(result.fetch_reports[1].status, FetchStatus::Failed);
    assert_eq!(store.fetch_reports(None).unwrap().len(), 2);
    assert!(result.fetch_reports[0].scope.contains("since="));
    assert!(result.fetch_reports[1].scope.contains("2026-09-07"));
    assert!(result.fetch_reports[1].scope.contains("2026-09-08"));
    for report in &result.fetch_reports {
        let started = DateTime::parse_from_rfc3339(&report.started_at).unwrap();
        let finished = DateTime::parse_from_rfc3339(&report.finished_at).unwrap();
        assert!(started <= finished);
    }
}

#[test]
fn calendar_success_mail_failure_is_partial_and_keeps_calendar_data() {
    let home = Home::new();
    let store = home.store();
    let source = MockSource::new(
        Err("mail secret=do-not-echo".into()),
        Ok(vec![event("cal-1")]),
    );

    let result = run(&store, &source, &config(), false);

    assert_eq!(result.status, FetchStatus::Partial);
    assert_eq!(result.candidates_added, 1);
    assert_eq!(store.open_candidates().unwrap().len(), 1);
    assert_eq!(store.events_for_day("2026-09-07").unwrap().len(), 1);
    assert!(!result
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("secret"));
    assert_eq!(result.fetch_reports[0].status, FetchStatus::Failed);
    assert_eq!(result.fetch_reports[1].status, FetchStatus::Success);
}

#[test]
fn empty_success_is_success_with_zero_item_reports() {
    let home = Home::new();
    let store = home.store();
    let source = MockSource::new(Ok(Vec::new()), Ok(Vec::new()));

    let result = run(&store, &source, &config(), false);

    assert_eq!(result.status, FetchStatus::Success);
    assert!(result.ok);
    assert!(result.error.is_none());
    assert!(result
        .fetch_reports
        .iter()
        .all(|report| { report.status == FetchStatus::Success && report.item_count == 0 }));
}

#[test]
fn unavailable_branches_are_labeled_without_echoing_source_error() {
    let home = Home::new();
    let store = home.store();
    let source = MockSource::new(
        Err("new_outlook_or_missing: token=secret".into()),
        Err("unavailable: token=secret".into()),
    );

    let result = run(&store, &source, &config(), false);

    assert_eq!(result.status, FetchStatus::Unavailable);
    assert_eq!(result.fetch_reports[0].status, FetchStatus::Unavailable);
    assert_eq!(result.fetch_reports[1].status, FetchStatus::Unavailable);
    assert!(!result
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("secret"));
}

#[test]
fn failed_calendar_retry_preserves_prior_events() {
    let home = Home::new();
    let store = home.store();
    let first = MockSource::new(Ok(Vec::new()), Ok(vec![event("cal-1")]));
    run(&store, &first, &config(), false);
    let second = MockSource::new(Ok(Vec::new()), Err("calendar unavailable".into()));

    let result = run(&store, &second, &config(), false);

    assert_eq!(result.status, FetchStatus::Partial);
    let events = store.events_for_day("2026-09-07").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].entry_id, "cal-1");
}

#[test]
fn successful_empty_calendar_clears_only_its_source_snapshot() {
    let home = Home::new();
    let store = home.store();
    let first = MockSource::new(Ok(Vec::new()), Ok(vec![event("cal-1")]));
    run(&store, &first, &config(), false);
    store
        .upsert_events_from_source(
            "2026-09-07",
            "other-source",
            vec![stored_event("other-1", "other-source")],
        )
        .unwrap();

    let empty = MockSource::new(Ok(Vec::new()), Ok(Vec::new()));
    let result = run(&store, &empty, &config(), false);

    assert_eq!(result.status, FetchStatus::Success);
    let events = store.events_for_day("2026-09-07").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].entry_id, "other-1");
    assert_eq!(events[0].source, "other-source");
}

#[test]
fn event_snapshot_failure_rolls_back_observations_candidates_and_reports() {
    let home = Home::new();
    let store = home.store();
    store
        .upsert_events_from_source(
            "2026-09-07",
            "other-source",
            vec![stored_event("collision", "other-source")],
        )
        .unwrap();
    let source = MockSource::new(
        Ok(vec![mail("mail-rollback", None)]),
        Ok(vec![event("collision")]),
    );

    let result = intake::ingest(
        &store,
        &source,
        &config(),
        now() - chrono::Duration::days(30),
        now(),
        false,
    );

    assert!(result.is_err());
    assert!(store.open_candidates().unwrap().is_empty());
    assert!(store.observations_for_day("2026-09-07").unwrap().is_empty());
    assert!(store.fetch_reports(None).unwrap().is_empty());
    let events = store.events_for_day("2026-09-07").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, "other-source");
}

#[test]
fn read_body_false_excludes_fixture_body_from_rules_and_observation() {
    let home = Home::new();
    let store = home.store();
    let mut cfg = config();
    cfg.read_body = false;
    let source = MockSource::new(Ok(vec![mail("mail-body", Some("対応"))]), Ok(Vec::new()));

    let result = run(&store, &source, &cfg, false);

    assert_eq!(result.candidates_added, 1);
    let observations = store.observations_for_day("2026-09-07").unwrap();
    assert_eq!(observations.len(), 1);
    assert!(observations[0].body.is_none());
}

#[test]
fn successful_non_candidate_items_are_still_recorded_as_observations() {
    let home = Home::new();
    let store = home.store();
    let mut quiet_mail = mail("mail-quiet", None);
    quiet_mail.subject = "参考情報".into();
    quiet_mail.sender_name = "未登録の送信者".into();
    quiet_mail.unread = false;
    let mut quiet_event = event("cal-quiet");
    quiet_event.is_organizer = false;
    quiet_event.response_required = false;
    let mut cfg = config();
    cfg.meeting_prep_only_required = true;
    let source = MockSource::new(Ok(vec![quiet_mail]), Ok(vec![quiet_event]));

    let result = run(&store, &source, &cfg, false);

    assert_eq!(result.candidates_added, 0);
    let observations = store.observations_for_day("2026-09-07").unwrap();
    assert_eq!(observations.len(), 2);
    assert!(observations
        .iter()
        .any(|observation| observation.source_ref == "outlook:mail:mail-quiet"));
    assert!(observations
        .iter()
        .any(|observation| observation.source_ref == "outlook:cal:cal-quiet:prep"));
}

#[test]
fn rejected_mail_candidate_does_not_reappear_on_retry() {
    let home = Home::new();
    let store = home.store();
    let source = MockSource::new(Ok(vec![mail("mail-rejected", None)]), Ok(Vec::new()));
    run(&store, &source, &config(), false);
    let candidate = store.open_candidates().unwrap().pop().unwrap();
    store.reject_candidate(&candidate.id).unwrap();

    let result = run(&store, &source, &config(), false);

    assert_eq!(result.candidates_added, 0);
    assert_eq!(result.candidates_skipped, 1);
    assert!(store.open_candidates().unwrap().is_empty());
    assert_eq!(store.observations_for_day("2026-09-07").unwrap().len(), 1);
}

#[test]
fn dry_run_writes_no_candidates_events_or_reports() {
    let home = Home::new();
    let store = home.store();
    let source = MockSource::new(Ok(vec![mail("mail-dry", None)]), Ok(vec![event("cal-dry")]));

    let result = run(&store, &source, &config(), true);

    assert_eq!(result.status, FetchStatus::Success);
    assert!(store.open_candidates().unwrap().is_empty());
    assert!(store.events_for_day("2026-09-07").unwrap().is_empty());
    assert!(store.fetch_reports(None).unwrap().is_empty());
}

#[test]
fn fixture_invalid_overlay_is_an_error_without_silent_fallback() {
    let home = Home::new();
    let fixture = home.0.join("fixture");
    std::fs::create_dir_all(&fixture).unwrap();
    std::fs::write(fixture.join("mails.json"), "[]").unwrap();
    std::fs::write(fixture.join("events.json"), "[]").unwrap();
    std::fs::write(fixture.join("intake.toml"), "lookback_days = [").unwrap();

    let store = home.store();
    assert!(intake::run_fixture(&store, &fixture, &Config::default()).is_err());
    assert!(store.fetch_reports(None).unwrap().is_empty());
}

#[test]
fn a_markdown_mirror_failure_is_reported_without_relabeling_a_saved_fetch() {
    let home = Home::new();
    let store = home.store();
    std::fs::create_dir_all(home.0.join("days/2026-09-07.md")).unwrap();
    let result = run(
        &store,
        &MockSource::new(Ok(Vec::new()), Ok(vec![event("mirror-event")])),
        &config(),
        false,
    );
    assert!(!result.ok);
    assert_eq!(result.status, FetchStatus::Success);
    assert_eq!(result.error.as_deref(), Some("markdown_export_failed"));
    assert!(result.summary_line().contains("markdown_export_failed"));
    assert_eq!(store.events_for_day("2026-09-07").unwrap().len(), 1);
    assert_eq!(store.fetch_reports(None).unwrap().len(), 2);
}
