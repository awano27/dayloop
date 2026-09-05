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
    Local.with_ymd_and_hms(2026, 9, 3, h, m, 0).single().unwrap()
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

    let id = Store::open().unwrap().open_candidates().unwrap()[0].id.clone();
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
    assert_eq!(r.events, 1, "only the +2h meeting falls in today..=tomorrow: {r:?}");

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
