use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Local, TimeZone};
use dayloop::config::IntakeConfig;
use dayloop::intake::fixture::FixtureSource;
use dayloop::intake::{self, SyncResult};
use dayloop::store::Store;

static LOCK: Mutex<()> = Mutex::new(());

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/outlook")
}

fn at(h: u32, m: u32) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(2026, 9, 3, h, m, 0)
        .single()
        .unwrap()
}

fn intake_cfg() -> IntakeConfig {
    IntakeConfig {
        important_senders: vec!["田中".into()],
        lookback_days: 365,
        ..IntakeConfig::default()
    }
}

struct Home {
    dir: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!(
            "dayloop-pipe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "[jev]\nmode = \"off\"\n").unwrap();
        unsafe {
            std::env::set_var("DAYLOOP_HOME", &dir);
        }
        Self {
            dir,
            _guard: guard,
            old,
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        unsafe {
            match &self.old {
                Some(v) => std::env::set_var("DAYLOOP_HOME", v),
                None => std::env::remove_var("DAYLOOP_HOME"),
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ingest_at(now: DateTime<Local>) -> SyncResult {
    let store = Store::open().unwrap();
    let src = FixtureSource::load_at(&fixture_dir(), now).unwrap();
    let since = now - Duration::days(365);
    intake::ingest(&store, &src, &intake_cfg(), since, now, false).unwrap()
}

#[test]
fn fixture_twice_does_not_duplicate_and_respects_reject() {
    let _home = Home::new();
    let now = at(9, 0);

    let r1 = ingest_at(now);
    assert_eq!(r1.candidates_added, 4, "{r1:?}");
    assert_eq!(r1.events, 2, "{r1:?}");

    let store = Store::open().unwrap();
    assert_eq!(store.open_candidates().unwrap().len(), 4);
    let today = now.format("%Y-%m-%d").to_string();
    let events = store.events_for_day(&today).unwrap();
    assert_eq!(events.len(), 1);
    assert!(events.iter().any(|e| e.subject == "週次定例"));

    let r2 = ingest_at(now);
    assert_eq!(r2.candidates_added, 0, "{r2:?}");
    assert_eq!(Store::open().unwrap().open_candidates().unwrap().len(), 4);

    let id = Store::open().unwrap().open_candidates().unwrap()[0]
        .id
        .clone();
    Store::open().unwrap().reject_candidate(&id).unwrap();
    let r3 = ingest_at(now);
    assert_eq!(r3.candidates_added, 0, "{r3:?}");
    assert_eq!(Store::open().unwrap().open_candidates().unwrap().len(), 3);

    let path = dayloop::markdown::export(&Store::open().unwrap(), &today).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("## 今日の予定"), "{text}");
    assert!(text.contains("週次定例"), "{text}");
}

#[test]
fn cal_prep_crosses_midnight_when_base_is_2330() {
    let _home = Home::new();
    let now = at(23, 30);
    let r = ingest_at(now);
    assert_eq!(
        r.events, 1,
        "only the +2h meeting falls in today..=tomorrow: {r:?}"
    );

    let today = now.format("%Y-%m-%d").to_string();
    let tomorrow = (now.date_naive() + Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let store = Store::open().unwrap();
    assert!(
        store.events_for_day(&today).unwrap().is_empty(),
        "cal-prep at 23:30+2h must not land on {today}"
    );
    let next = store.events_for_day(&tomorrow).unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].subject, "週次定例");
}

#[test]
fn fetched_batch_rolls_back_when_second_save_fails() {
    let _home = Home::new();
    let store = Store::open().unwrap();
    let db = rusqlite::Connection::open(dayloop::paths::db_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_second BEFORE INSERT ON candidates WHEN NEW.title = 'second' BEGIN SELECT RAISE(ABORT, 'secret-body'); END;").unwrap();
    let items = vec![
        dayloop::github::Item {
            title: "first".into(),
            source_ref: "github:first".into(),
            open: true,
        },
        dayloop::github::Item {
            title: "second".into(),
            source_ref: "github:second".into(),
            open: true,
        },
    ];
    assert!(intake::github_intake::ingest(&store, &items).is_err());
    assert!(
        store.open_candidates().unwrap().is_empty(),
        "failed fetched batch must roll back"
    );
}

#[test]
fn inbox_invalid_utf8_and_mixed_files_are_not_successful_zero() {
    let home = Home::new();
    let store = Store::open().unwrap();
    let dir = home.dir.join("inbox");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("bad.txt"), [0xff, 0xfe]).unwrap();
    let bad = intake::run_inbox(&store).unwrap();
    assert!(!bad.ok, "invalid UTF8 must be reported: {bad:?}");
    assert_eq!(bad.candidates_added, 0);
    std::fs::write(dir.join("good.txt"), "TODO: synthetic action").unwrap();
    let mixed = intake::run_inbox(&store).unwrap();
    assert!(
        !mixed.ok,
        "mixed input must report partial failure: {mixed:?}"
    );
    assert_eq!(mixed.candidates_added, 1);
    let retry = intake::run_inbox(&store).unwrap();
    assert_eq!(retry.candidates_added, 0);
    assert_eq!(retry.candidates_skipped, 1);
    assert_eq!(store.open_candidates().unwrap().len(), 1);
}

#[test]
fn inbox_file_path_reports_enumeration_failure() {
    let home = Home::new();
    let store = Store::open().unwrap();
    std::fs::write(home.dir.join("inbox"), "not a directory").unwrap();
    let result = intake::run_inbox(&store).expect("existing unreadable inbox must not disappear");
    assert!(!result.ok);
    assert!(result.error.unwrap().contains("enumerate"));
}

#[test]
fn inbox_file_storage_failure_rolls_back_and_retry_deduplicates() {
    let home = Home::new();
    let store = Store::open().unwrap();
    let dir = home.dir.join("inbox");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "TODO: first\nTODO: second").unwrap();
    let db = rusqlite::Connection::open(dayloop::paths::db_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_second BEFORE INSERT ON candidates WHEN NEW.title = 'second' BEGIN SELECT RAISE(ABORT, 'secret-body'); END;").unwrap();
    let failed = intake::run_inbox(&store).unwrap();
    assert!(!failed.ok, "save failure must be visible: {failed:?}");
    assert_eq!(failed.candidates_added, 0);
    assert!(failed.error.as_deref().unwrap().contains("store"));
    assert!(!failed.error.as_deref().unwrap().contains("secret-body"));
    assert!(store.open_candidates().unwrap().is_empty());
    db.execute_batch("DROP TRIGGER fail_second").unwrap();
    assert_eq!(intake::run_inbox(&store).unwrap().candidates_added, 2);
    let retry = intake::run_inbox(&store).unwrap();
    assert_eq!(retry.candidates_added, 0);
    assert_eq!(retry.candidates_skipped, 2);
}

#[test]
fn inbox_zero_action_success_is_distinct_from_unrun_and_mixed_failure() {
    let home = Home::new();
    let store = Store::open().unwrap();
    assert!(intake::run_inbox(&store).is_none());
    let dir = home.dir.join("inbox");
    std::fs::create_dir(&dir).unwrap();
    let empty = intake::run_inbox(&store).unwrap();
    assert!(empty.ok);
    assert_eq!(empty.candidates_added, 0);
    std::fs::write(dir.join("good.txt"), "shared information only").unwrap();
    std::fs::write(dir.join("bad.txt"), [0xff]).unwrap();
    let mixed = intake::run_inbox(&store).unwrap();
    assert_eq!(mixed.status(), "partial");
    assert_eq!(mixed.candidates_added, 0);
    assert_eq!(mixed.files_processed, 1);
    assert_eq!(mixed.files_failed, 1);
}

#[test]
fn all_fetched_adapters_roll_back_and_preserve_existing_commitments() {
    use dayloop::commitment::dispatch;
    use serde_json::json;
    let _home = Home::new();
    let store = Store::open().unwrap();
    dispatch(&store,"commitment_register", &json!({"op_id":"register","request":"synthetic request","counterparty":"synthetic person","reply_due":"2099-01-05T12:00:00+09:00","next_check":"2099-01-05T12:00:00+09:00","source":"manual","source_ref":"request-A","evidence":"synthetic"})).unwrap();
    let snapshot = dispatch(&store, "commitment_list", &json!({})).unwrap();
    let db = rusqlite::Connection::open(dayloop::paths::db_path()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_second BEFORE INSERT ON candidates WHEN NEW.title = 'second' BEGIN SELECT RAISE(ABORT, 'secret-body'); END;").unwrap();
    let github: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|s| dayloop::github::Item {
            title: s.into(),
            source_ref: format!("github:{s}"),
            open: true,
        })
        .collect();
    let jira: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|s| dayloop::jira::Item {
            title: s.into(),
            source_ref: format!("jira:{s}"),
            open: true,
        })
        .collect();
    let devops: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|s| dayloop::devops::Item {
            title: s.into(),
            source_ref: format!("devops:{s}"),
            open: true,
        })
        .collect();
    let teams: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|s| dayloop::chat::Item {
            title: s.into(),
            source_ref: format!("teams:{s}"),
        })
        .collect();
    for outcome in [
        intake::github_intake::ingest(&store, &github),
        intake::jira_live::ingest(&store, &jira),
        intake::devops_live::ingest(&store, &devops),
        intake::chat_live::ingest(&store, &teams),
    ] {
        assert!(outcome.is_err());
        assert!(store.open_candidates().unwrap().is_empty());
    }
    assert_eq!(
        dispatch(&store, "commitment_list", &json!({})).unwrap(),
        snapshot
    );
}
