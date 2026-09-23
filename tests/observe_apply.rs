use std::collections::BTreeMap;

use dayloop::graph;
use dayloop::model::State;
use dayloop::observe;
use dayloop::store::Store;

static OBSERVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Home {
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = OBSERVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-observe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("DAYLOOP_HOME", &dir) };
        Self {
            dir,
            _guard: guard,
            old,
        }
    }
    fn store(&self) -> Store {
        Store::open().unwrap()
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

#[test]
fn merged_ref_becomes_done_without_a_graph_edge() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    let t = store
        .add_task(
            "PRを出す",
            Some(date),
            None,
            "ticket",
            Some("github:pr:dayloop#7"),
            Some(date),
        )
        .unwrap();
    let open = store
        .add_task(
            "まだ開いている",
            Some(date),
            None,
            "ticket",
            Some("ticket:ABC-2"),
            Some(date),
        )
        .unwrap();
    let none = store
        .add_task("参照なし", Some(date), None, "manual", None, Some(date))
        .unwrap();
    let map = observe::parse_fixture(
        r#"[
        {"source_ref":"github:pr:dayloop#7","state":"merged"},
        {"source_ref":"ticket:ABC-2","state":"open"}
    ]"#,
    )
    .unwrap();
    let n = observe::apply(&store, date, &map).unwrap();
    assert_eq!(n, 1);
    let done = store.get_task(&t.id).unwrap();
    assert_eq!(done.state, State::Done);
    assert_eq!(done.evidence.as_deref(), Some("observe:github:pr:dayloop#7"));
    assert_eq!(
        done.decided_by.as_deref(),
        Some("observe:github:pr:dayloop#7")
    );
    assert_eq!(store.get_task(&open.id).unwrap().state, State::Planned);
    assert_eq!(store.get_task(&none.id).unwrap().state, State::Planned);
    let sit = graph::Situation::from_task(&done, "close");
    assert!(graph::resolve(&store, &sit).unwrap().is_none());
    let again = store
        .add_task(
            "PRを出す",
            Some("2026-09-23"),
            None,
            "manual",
            None,
            Some("2026-09-23"),
        )
        .unwrap();
    assert_eq!(graph::apply_known_tasks(&store, "2026-09-23").unwrap(), 0);
    assert_eq!(store.get_task(&again.id).unwrap().state, State::Planned);
}

#[test]
fn empty_config_observes_nothing() {
    let cfg = dayloop::config::Config::default();
    assert!(cfg.observe.fixture.is_empty());
    assert!(observe::load_map(&cfg.observe).unwrap().is_empty());
}

#[test]
fn finished_meeting_prep_is_done_and_a_future_one_is_not() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    store
        .upsert_events(
            date,
            vec![dayloop::model::Event {
                entry_id: "cal-1".into(),
                date: date.into(),
                start: "2026-09-22T10:00:00+09:00".into(),
                end: "2026-09-22T11:00:00+09:00".into(),
                subject: "週次".into(),
                location: None,
                organizer: None,
                is_organizer: true,
                source: "outlook".into(),
                synced_at: "2026-09-22T09:00:00+09:00".into(),
            }],
        )
        .unwrap();
    let prep = store
        .add_task(
            "会議準備: 週次",
            Some(date),
            Some(30),
            "outlook",
            Some("outlook:cal:cal-1:prep"),
            Some(date),
        )
        .unwrap();
    let early = observe::apply_day(&store, date, &BTreeMap::new(), "10:30").unwrap();
    assert_eq!(early, 0);
    assert_eq!(store.get_task(&prep.id).unwrap().state, State::Planned);
    let late = observe::apply_day(&store, date, &BTreeMap::new(), "12:00").unwrap();
    assert_eq!(late, 1);
    let done = store.get_task(&prep.id).unwrap();
    assert_eq!(done.state, State::Done);
    assert_eq!(
        done.decided_by.as_deref(),
        Some("observe:outlook:cal:cal-1:prep")
    );
    let sit = graph::Situation::from_task(&done, "close");
    assert!(graph::resolve(&store, &sit).unwrap().is_none());
}
