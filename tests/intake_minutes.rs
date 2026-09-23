use dayloop::store::Store;

static MINUTES_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Home {
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = MINUTES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-minutes-{}", std::process::id()));
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
fn actions_become_meeting_candidates_and_info_does_not() {
    let home = Home::new();
    let store = home.store();
    let text = "決定: 見積を直す\n共有: 来週は休会\n雑談\n";
    let report = dayloop::intake::minutes::ingest(&store, text).unwrap();
    assert_eq!(report.actions, 1);
    assert_eq!(report.info, 1);
    assert_eq!(report.ignored, 1);
    let cands = store.open_candidates().unwrap();
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].title, "見積を直す");
    assert_eq!(cands[0].source, "meeting");
    assert!(cands[0].source_ref.as_deref().unwrap().starts_with("minutes:"));
}

#[test]
fn decision_heading_becomes_a_candidate_and_share_heading_does_not() {
    let home = Home::new();
    let store = home.store();
    let text = "\
決定事項
- 見積を直す
共有事項
- 来週は休会
雑談
";
    let report = dayloop::intake::minutes::ingest(&store, text).unwrap();
    assert_eq!(report.actions, 1);
    assert_eq!(report.info, 1);
    let cands = store.open_candidates().unwrap();
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].title, "見積を直す");
    assert_eq!(cands[0].source, "meeting");
}
