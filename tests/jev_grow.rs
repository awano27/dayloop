use dayloop::jev::{self, Decider, JevOutcome};
use dayloop::model::State;
use dayloop::store::Store;

static GROW_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Home {
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = GROW_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-grow-{}", std::process::id()));
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

struct Script {
    calls: usize,
}

impl Decider for Script {
    fn decide(&mut self, _state: &str, choices: &[String]) -> JevOutcome {
        self.calls += 1;
        if choices.iter().any(|c| c == "today") {
            JevOutcome::Answer { choice: "today".into(), confidence: 0.9 }
        } else if choices.iter().any(|c| c == "not_done:no_time") {
            JevOutcome::Answer { choice: "not_done:no_time".into(), confidence: 0.8 }
        } else {
            JevOutcome::Answer { choice: "ask".into(), confidence: 0.2 }
        }
    }
}

#[test]
fn a_confident_candidate_becomes_an_edge_and_the_next_copy_is_not_asked() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-23";
    store.add_candidate("週報", "outlook", Some("mail:1")).unwrap();
    store.add_candidate("週報", "outlook", Some("mail:2")).unwrap();
    let mut decider = Script { calls: 0 };
    let n = jev::grow(&store, date, &mut decider, 0.5).unwrap();
    assert!(n >= 2);
    assert_eq!(decider.calls, 2);
    assert!(store.open_candidates().unwrap().is_empty());
    let tasks = store.tasks_for_day(date).unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|t| t.state == State::NotDone));
}

#[test]
fn a_low_score_stays_a_question() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-23";
    store.add_candidate("新しい依頼", "outlook", Some("mail:3")).unwrap();
    struct Low;
    impl Decider for Low {
        fn decide(&mut self, _state: &str, _choices: &[String]) -> JevOutcome {
            JevOutcome::Answer { choice: "today".into(), confidence: 0.2 }
        }
    }
    let n = jev::grow(&store, date, &mut Low, 0.5).unwrap();
    assert_eq!(n, 0);
    assert_eq!(store.open_candidates().unwrap().len(), 1);
}
