use dayloop::graph;
use dayloop::model::State;
use dayloop::order;
use dayloop::store::Store;

static ORDER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Home {
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = ORDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-order-{}", std::process::id()));
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
fn saved_preference_swaps_only_an_equal_rank_pair() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    store
        .add_task("甲", Some(date), Some(30), "manual", None, Some(date))
        .unwrap();
    store
        .add_task("乙", Some(date), Some(30), "manual", None, Some(date))
        .unwrap();
    graph::record_order(&store, "乙", "甲").unwrap();
    let titles: Vec<_> = order::day_tasks(&store, date)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles, vec!["乙".to_string(), "甲".to_string()]);
}

#[test]
fn saved_preference_does_not_override_overdue() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    store
        .add_task("超過", Some("2026-09-21"), Some(30), "manual", None, Some(date))
        .unwrap();
    store
        .add_task("今日", Some(date), Some(30), "manual", None, Some(date))
        .unwrap();
    graph::record_order(&store, "今日", "超過").unwrap();
    let titles: Vec<_> = order::day_tasks(&store, date)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles[0], "超過");
    assert!(store
        .open_tasks_for_day(date)
        .unwrap()
        .iter()
        .all(|t| t.state == State::Planned));
}
