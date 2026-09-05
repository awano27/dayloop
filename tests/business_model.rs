use dayloop::business::{Category, FetchReportInput, FetchStatus, ReviewOutcome};
use dayloop::store::Store;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dayloop-business-{}", ulid::Ulid::new()));
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

fn resolve_all(s: &Store, date: &str) {
    for category in s.required_categories().unwrap() {
        s.record_review(date, category, ReviewOutcome::Confirmed, None, None, None)
            .unwrap();
    }
}

#[test]
fn new_days_default_to_all_categories_and_pending_reviews_block_direct_close() {
    let home = Home::new();
    let s = home.store();
    assert_eq!(s.required_categories().unwrap(), Category::all());
    let day = s.prepare_day("2026-09-07").unwrap();
    assert_eq!(day.reviews.len(), 7);
    let pending = s.close_day("2026-09-07").unwrap_err();
    assert!(pending
        .downcast_ref::<dayloop::business::ReviewPending>()
        .is_some());
    assert!(s
        .record_review(
            "2026-09-07",
            Category::Teams,
            ReviewOutcome::NotChecked,
            Some("  "),
            None,
            None,
        )
        .is_err());
    s.record_review(
        "2026-09-07",
        Category::Teams,
        ReviewOutcome::NotChecked,
        Some("端末が利用できない"),
        None,
        None,
    )
    .unwrap();
    resolve_all(&s, "2026-09-07");
    s.close_day("2026-09-07").unwrap().unwrap();
    assert!(s.pending_reviews("2026-09-07").unwrap().is_empty());
    assert!(s
        .record_review(
            "2026-09-07",
            Category::Teams,
            ReviewOutcome::Confirmed,
            None,
            None,
            None,
        )
        .is_err());
}

#[test]
fn configured_categories_only_affect_future_unprepared_days() {
    let home = Home::new();
    let s = home.store();
    s.prepare_day("2026-09-07").unwrap();
    s.set_required_categories(&[Category::Tasks]).unwrap();
    assert_eq!(s.pending_reviews("2026-09-07").unwrap().len(), 7);
    assert_eq!(s.prepare_day("2026-09-08").unwrap().reviews.len(), 1);
    assert_eq!(
        s.pending_reviews("2026-09-08").unwrap()[0].category,
        Category::Tasks
    );
}

#[test]
fn migration_grandfathers_existing_days_without_creating_pending_reviews() {
    let home = Home::new();
    let db_path = home.0.join("dayloop.db");
    let store = Store::open_at(&db_path).unwrap();
    let task = store
        .add_task(
            "legacy planned",
            None,
            None,
            "manual",
            None,
            Some("2026-09-07"),
        )
        .unwrap();
    drop(store);
    let db = Connection::open(&db_path).unwrap();
    db.execute_batch(
        "DROP TABLE routine_occurrences; DROP TABLE routines; DROP TABLE source_observations;
         DROP TABLE observations; DROP TABLE fetch_reports; DROP TABLE reviews; DROP TABLE review_sets;
         DROP TABLE required_categories; DELETE FROM days; PRAGMA user_version=1;",
    )
    .unwrap();
    drop(db);
    let migrated = Store::open_at(&db_path).unwrap();
    assert!(migrated.get_day("2026-09-07").unwrap().is_none());
    assert!(migrated.reviews_for_day("2026-09-07").unwrap().is_empty());
    assert!(migrated
        .integrity_issues()
        .unwrap()
        .iter()
        .any(|issue| issue.code == "missing_day" && issue.task_id.as_deref() == Some(&task.id)));
    assert_eq!(migrated.prepare_day("2026-09-08").unwrap().reviews.len(), 7);
}

#[test]
fn fetch_results_are_distinct_from_human_reviews() {
    let home = Home::new();
    let s = home.store();
    s.record_fetch_report(FetchReportInput {
        category: Category::Outlook,
        source: "outlook-fixture".into(),
        scope: "inbox".into(),
        status: FetchStatus::Success,
        item_count: 0,
        started_at: "2026-09-07T08:00:00+09:00".into(),
        finished_at: "2026-09-07T08:00:01+09:00".into(),
        reason: None,
    })
    .unwrap();
    assert!(s
        .record_fetch_report(FetchReportInput {
            category: Category::Outlook,
            source: "outlook-fixture".into(),
            scope: "calendar".into(),
            status: FetchStatus::Partial,
            item_count: 2,
            started_at: "2026-09-07T08:01:00+09:00".into(),
            finished_at: "2026-09-07T08:01:01+09:00".into(),
            reason: None,
        })
        .is_err());
    assert_eq!(s.fetch_reports(Some("2026-09-07")).unwrap().len(), 1);
    assert!(s.reviews_for_day("2026-09-07").unwrap().is_empty());
    s.prepare_day("2026-09-07").unwrap();
    assert_eq!(s.pending_reviews("2026-09-07").unwrap().len(), 7);
}

#[test]
fn needs_action_requires_a_real_link_and_observation_provenance_survives_adoption() {
    let home = Home::new();
    let s = home.store();
    let date = "2026-09-07";
    s.prepare_day(date).unwrap();
    assert!(s
        .record_review(
            date,
            Category::MeetingResults,
            ReviewOutcome::NeedsAction,
            None,
            Some("missing"),
            None,
        )
        .is_err());
    let observation = s
        .add_observation(
            Category::MeetingResults,
            "meeting:123",
            Some("123"),
            "合意事項",
            Some("明日までに資料を確認"),
            "2026-09-07T10:00:00+09:00",
        )
        .unwrap();
    let candidate = s
        .propose_action(&observation.id, "review-material", "資料を確認する")
        .unwrap()
        .unwrap();
    let task = s.accept_candidate(&candidate.id, Some(date)).unwrap();
    s.record_review(
        date,
        Category::MeetingResults,
        ReviewOutcome::NeedsAction,
        None,
        Some(&task.id),
        None,
    )
    .unwrap();
    assert_eq!(s.task_provenance(&task.id).unwrap()[0].id, observation.id);
    assert_eq!(
        s.candidate_provenance(&candidate.id).unwrap()[0].id,
        observation.id
    );
    s.reject_candidate(&candidate.id).unwrap_err();
    let carried = s
        .carry_over(&task.id, "翌日に確認", "2026-09-08", None)
        .unwrap();
    let split = s
        .split(
            &carried.id,
            &["資料を読む".to_string(), "返信を書く".to_string()],
            "作業を分ける",
            "2026-09-09",
        )
        .unwrap();
    assert_eq!(
        s.task_provenance(&carried.id).unwrap()[0].id,
        observation.id
    );
    assert_eq!(
        s.task_provenance(&split[0].id).unwrap()[0].id,
        observation.id
    );
}

#[test]
fn rejected_actions_are_not_reproposed_and_multiple_actions_can_share_an_observation() {
    let home = Home::new();
    let s = home.store();
    let observation = s
        .add_observation(
            Category::MeetingResults,
            "meeting:456",
            Some("456"),
            "会議",
            None,
            "2026-09-07T10:00:00+09:00",
        )
        .unwrap();
    let rejected = s
        .propose_action(&observation.id, "first", "最初の対応")
        .unwrap()
        .unwrap();
    s.reject_candidate(&rejected.id).unwrap();
    assert!(s
        .propose_action(&observation.id, "first", "書き換え")
        .unwrap()
        .is_none());
    assert!(s
        .propose_action(&observation.id, "second", "別の対応")
        .unwrap()
        .is_some());
}

#[test]
fn routines_generate_one_current_occurrence_and_report_recent_missed_dates() {
    let home = Home::new();
    let s = home.store();
    let routine = s
        .add_routine("週次確認", &["Mon".to_string()], "2026-08-01")
        .unwrap();
    let first = s.prepare_day("2026-09-07").unwrap();
    assert_eq!(first.generated_tasks.len(), 1);
    assert_eq!(
        s.prepare_day("2026-09-07").unwrap().generated_tasks.len(),
        0
    );
    assert!(first
        .missed_routines
        .iter()
        .any(|gap| gap.routine_id == routine.id && gap.date == "2026-08-10"));
    resolve_all(&s, "2026-09-07");
    assert!(s.close_day("2026-09-07").unwrap().is_err());
}

#[test]
fn concurrent_preparation_creates_one_routine_occurrence() {
    let home = Home::new();
    let db_path = home.0.join("dayloop.db");
    let first = Store::open_at(&db_path).unwrap();
    first.set_required_categories(&[]).unwrap();
    first
        .add_routine("並行の定期作業", &["Mon".to_string()], "2026-09-01")
        .unwrap();
    let second = Store::open_at(&db_path).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let other = barrier.clone();
    let one = std::thread::spawn(move || {
        barrier.wait();
        first
            .prepare_day("2026-09-07")
            .unwrap()
            .generated_tasks
            .len()
    });
    let two = std::thread::spawn(move || {
        other.wait();
        second
            .prepare_day("2026-09-07")
            .unwrap()
            .generated_tasks
            .len()
    });
    assert_eq!(one.join().unwrap() + two.join().unwrap(), 1);
    assert_eq!(
        Store::open_at(&db_path)
            .unwrap()
            .tasks_for_day("2026-09-07")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn enum_json_contracts_use_stable_snake_case_names() {
    assert_eq!(
        serde_json::to_string(&Category::MeetingResults).unwrap(),
        "\"meeting_results\""
    );
    assert_eq!(
        serde_json::to_string(&ReviewOutcome::NeedsAction).unwrap(),
        "\"needs_action\""
    );
    assert_eq!(
        serde_json::to_string(&FetchStatus::Unavailable).unwrap(),
        "\"unavailable\""
    );
}

#[test]
fn unknown_persisted_required_category_is_rejected() {
    let home = Home::new();
    let db_path = home.0.join("dayloop.db");
    drop(Store::open_at(&db_path).unwrap());
    let db = Connection::open(&db_path).unwrap();
    db.execute(
        "INSERT INTO required_categories(category) VALUES('unknown')",
        [],
    )
    .unwrap();
    drop(db);
    assert!(Store::open_at(&db_path)
        .unwrap()
        .required_categories()
        .is_err());
}

#[test]
fn closed_day_preparation_only_reads_existing_business_state() {
    let home = Home::new();
    let s = home.store();
    s.set_required_categories(&[]).unwrap();
    s.close_day("2026-09-07").unwrap().unwrap();
    s.add_routine("閉鎖日には生成しない", &["Mon".to_string()], "2026-09-01")
        .unwrap();
    let before = s.tasks_for_day("2026-09-07").unwrap().len();
    let prepared = s.prepare_day("2026-09-07").unwrap();
    assert!(prepared.reviews.is_empty());
    assert!(prepared.generated_tasks.is_empty());
    assert_eq!(s.tasks_for_day("2026-09-07").unwrap().len(), before);
    let db = Connection::open(home.0.join("dayloop.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM routine_occurrences", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn needs_action_is_pending_after_linked_candidate_is_rejected_but_a_valid_task_still_resolves() {
    let home = Home::new();
    let s = home.store();
    s.set_required_categories(&[Category::MeetingResults])
        .unwrap();
    let date = "2026-09-07";
    s.prepare_day(date).unwrap();
    let candidate = s
        .add_candidate("候補", "manual", Some("candidate:later-rejected"))
        .unwrap()
        .unwrap();
    let task = s
        .add_task("実在する対応", None, None, "manual", None, Some(date))
        .unwrap();
    s.record_review(
        date,
        Category::MeetingResults,
        ReviewOutcome::NeedsAction,
        None,
        Some(&task.id),
        Some(&candidate.id),
    )
    .unwrap();
    s.reject_candidate(&candidate.id).unwrap();
    assert!(s.pending_reviews(date).unwrap().is_empty());

    let other = s
        .add_candidate("別候補", "manual", Some("candidate:rejected-only"))
        .unwrap()
        .unwrap();
    s.record_review(
        date,
        Category::MeetingResults,
        ReviewOutcome::NeedsAction,
        None,
        None,
        Some(&other.id),
    )
    .unwrap();
    s.reject_candidate(&other.id).unwrap();
    assert_eq!(s.pending_reviews(date).unwrap().len(), 1);
    let db = Connection::open(home.0.join("dayloop.db")).unwrap();
    db.execute(
        "UPDATE candidates SET status='corrupt' WHERE id=?1",
        [&other.id],
    )
    .unwrap();
    assert!(s.pending_reviews(date).is_err());
}

#[test]
fn fetch_timestamps_require_rfc3339_and_instant_ordering() {
    let home = Home::new();
    let s = home.store();
    let input = |started_at: &str, finished_at: &str| FetchReportInput {
        category: Category::Outlook,
        source: "fixture".into(),
        scope: "inbox".into(),
        status: FetchStatus::Success,
        item_count: 0,
        started_at: started_at.into(),
        finished_at: finished_at.into(),
        reason: None,
    };
    assert!(s
        .record_fetch_report(input("2026-09-07T08:00", "2026-09-07T08:01:00+09:00"))
        .is_err());
    assert!(s
        .record_fetch_report(input(
            "2026-09-07T09:00:00+09:00",
            "2026-09-07T08:59:00+09:00"
        ))
        .is_err());
    s.record_fetch_report(input("2026-09-07T09:00:00+09:00", "2026-09-07T00:30:00Z"))
        .unwrap();
}

#[test]
fn concurrent_duplicate_observations_return_the_same_record() {
    let home = Home::new();
    let db_path = home.0.join("dayloop.db");
    let one_store = Store::open_at(&db_path).unwrap();
    let two_store = Store::open_at(&db_path).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let other = barrier.clone();
    let one = std::thread::spawn(move || {
        barrier.wait();
        one_store
            .add_observation(
                Category::Teams,
                "teams:1",
                None,
                "通知",
                None,
                "2026-09-07T08:00:00+09:00",
            )
            .unwrap()
            .id
    });
    let two = std::thread::spawn(move || {
        other.wait();
        two_store
            .add_observation(
                Category::Teams,
                "teams:1",
                None,
                "通知の別文面",
                None,
                "2026-09-07T08:00:00+09:00",
            )
            .unwrap()
            .id
    });
    assert_eq!(one.join().unwrap(), two.join().unwrap());
}

#[test]
fn manual_records_without_source_refs_have_empty_provenance() {
    let home = Home::new();
    let s = home.store();
    let task = s
        .add_task("手動タスク", None, None, "manual", None, None)
        .unwrap();
    let candidate = s
        .add_candidate("手動候補", "manual", None)
        .unwrap()
        .unwrap();
    assert!(s.task_provenance(&task.id).unwrap().is_empty());
    assert!(s.candidate_provenance(&candidate.id).unwrap().is_empty());
}

#[test]
fn occurrence_insert_failure_rolls_back_its_task_and_review_set() {
    let home = Home::new();
    let s = home.store();
    s.set_required_categories(&[]).unwrap();
    s.add_routine("失敗する定期作業", &["Mon".to_string()], "2026-09-01")
        .unwrap();
    let db = Connection::open(home.0.join("dayloop.db")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_occurrence BEFORE INSERT ON routine_occurrences
         BEGIN SELECT RAISE(ABORT, 'synthetic occurrence failure'); END;",
    )
    .unwrap();
    assert!(s.prepare_day("2026-09-07").is_err());
    assert!(s.tasks_for_day("2026-09-07").unwrap().is_empty());
    assert!(s.get_day("2026-09-07").unwrap().is_none());
    assert_eq!(
        db.query_row("SELECT count(*) FROM routine_occurrences", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
