use dayloop::model::State;
use dayloop::store::Store;

static JEV_GATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Home {
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = JEV_GATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-jev-gate-{}", std::process::id()));
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

struct Fixed {
    choice: String,
    confidence: f64,
}

impl dayloop::jev::Decider for Fixed {
    fn decide(&mut self, _state: &str, _choices: &[String]) -> dayloop::jev::JevOutcome {
        dayloop::jev::JevOutcome::Answer {
            choice: self.choice.clone(),
            confidence: self.confidence,
        }
    }
}

#[test]
fn missing_floor_does_not_write_the_ledger() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    let t = store
        .add_task("未知の件", None, None, "manual", None, Some(date))
        .unwrap();
    let mut decider = Fixed { choice: "done".into(), confidence: 0.99 };
    let n = dayloop::jev::apply_if_measured(&store, date, &mut decider, None).unwrap();
    assert_eq!(n, 0);
    assert_eq!(store.get_task(&t.id).unwrap().state, State::Planned);
    assert!(store.get_task(&t.id).unwrap().decided_by.is_none());
}

#[test]
fn measured_floor_applies_only_at_or_above_it() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    let high = store
        .add_task("高い件", None, None, "manual", None, Some(date))
        .unwrap();
    let mut decider = Fixed { choice: "done".into(), confidence: 0.9 };
    let n = dayloop::jev::apply_if_measured(&store, date, &mut decider, Some(0.85)).unwrap();
    assert_eq!(n, 1);
    assert_eq!(store.get_task(&high.id).unwrap().state, State::Done);
    let low = store
        .add_task("低い件", Some("2026-09-30"), None, "manual", None, Some(date))
        .unwrap();
    let mut weak = Fixed { choice: "done".into(), confidence: 0.5 };
    let n = dayloop::jev::apply_if_measured(&store, date, &mut weak, Some(0.85)).unwrap();
    assert_eq!(n, 0);
    assert_eq!(store.get_task(&low.id).unwrap().state, State::Planned);
}
