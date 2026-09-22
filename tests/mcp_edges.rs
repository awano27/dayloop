use dayloop::graph;
use dayloop::model::State;
use dayloop::store::Store;
use dayloop::tools;

static MCP_EDGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Home {
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = MCP_EDGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-mcp-edge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("DAYLOOP_HOME", &dir) };
        Self { dir, _guard: guard, old }
    }
    fn store(&self) -> Store { Store::open().unwrap() }
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
fn close_day_applies_a_known_title_and_closes() {
    let home = Home::new();
    let store = home.store();
    let first = store
        .add_task("経費精算", None, None, "manual", None, Some("2026-09-22"))
        .unwrap();
    store
        .transition(&first.id, State::NotDone, Some("no_time"), None)
        .unwrap();
    let second = store
        .add_task("経費精算", None, None, "manual", None, Some("2026-09-23"))
        .unwrap();
    let v = tools::dispatch(&store, "close_day", &serde_json::json!({"date":"2026-09-23"}));
    assert_eq!(v["closed"], true);
    assert!(v["questions"].as_array().unwrap().is_empty());
    let second = store.get_task(&second.id).unwrap();
    assert_eq!(second.state, State::NotDone);
    assert!(second.decided_by.unwrap().starts_with("graph:"));
}

#[test]
fn plan_day_shelves_a_known_candidate_without_asking() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    graph::record(
        &store,
        &graph::Situation::intake("週報", "outlook", date),
        "shelve",
        None,
    )
    .unwrap();
    store.add_candidate("週報", "outlook", Some("mail:1")).unwrap();
    let v = tools::dispatch(&store, "plan_day", &serde_json::json!({"date": date}));
    let questions = v["questions"].as_array().unwrap();
    assert!(questions.iter().all(|q| q["title"] != "週報"));
    assert!(store.open_candidates().unwrap().is_empty());
}

#[test]
fn check_in_drops_a_known_title_before_questions() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    let t = store
        .add_task("不要な依頼", None, None, "manual", None, Some(date))
        .unwrap();
    store
        .transition(&t.id, State::Dropped, Some("not_needed"), None)
        .unwrap();
    store
        .add_task("不要な依頼", None, None, "manual", None, Some(date))
        .unwrap();
    let v = tools::dispatch(&store, "check_in", &serde_json::json!({"date": date}));
    assert!(v["questions"].as_array().unwrap().is_empty());
}

#[test]
fn get_today_does_not_apply_edges() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-23";
    let first = store
        .add_task("経費精算", None, None, "manual", None, Some("2026-09-22"))
        .unwrap();
    store
        .transition(&first.id, State::NotDone, Some("no_time"), None)
        .unwrap();
    let second = store
        .add_task("経費精算", None, None, "manual", None, Some(date))
        .unwrap();
    tools::dispatch(&store, "get_today", &serde_json::json!({"date": date}));
    assert_eq!(store.get_task(&second.id).unwrap().state, State::Planned);
}
