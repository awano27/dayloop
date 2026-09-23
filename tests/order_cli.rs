use dayloop::order;
use dayloop::store::Store;

static CLI_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Home {
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = CLI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-order-cli-{}", std::process::id()));
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
fn next_open_is_the_first_ranked_task() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    store
        .add_task("今日", Some(date), Some(30), "manual", None, Some(date))
        .unwrap();
    let late = store
        .add_task("超過", Some("2026-09-21"), Some(30), "manual", None, Some(date))
        .unwrap();
    let next = order::next_open(&store, date).unwrap().unwrap();
    assert_eq!(next.id, late.id);
}

#[test]
fn next_skips_a_task_the_graph_already_knows_how_to_carry() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    let first = store
        .add_task("経費精算", Some(date), Some(30), "manual", None, Some(date))
        .unwrap();
    store
        .carry_over(&first.id, "時間不足", "2026-09-23", None)
        .unwrap();
    store
        .add_task("経費精算", Some(date), Some(30), "manual", None, Some(date))
        .unwrap();
    let other = store
        .add_task("別件", None, Some(60), "manual", None, Some(date))
        .unwrap();

    let next = order::prepare_next(&store, date).unwrap().unwrap();
    assert_eq!(next.id, other.id);
    assert!(store
        .tasks_for_day(date)
        .unwrap()
        .iter()
        .filter(|t| t.title == "経費精算")
        .all(|t| !t.state.is_open()));
}
