use dayloop::store::Store;

static FIXTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Home {
    dir: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    old: Option<String>,
}

impl Home {
    fn new() -> Self {
        let guard = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-intake-fix-{}", std::process::id()));
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
fn ticket_and_teams_fixtures_become_candidates() {
    let home = Home::new();
    let store = home.store();
    dayloop::intake::tickets::ingest(
        &store,
        r#"[{"source_ref":"ticket:ABC-1","title":"請求書を送る","state":"open"}]"#,
    )
    .unwrap();
    dayloop::intake::teams::ingest(
        &store,
        r#"[{"source_ref":"teams:msg:1","title":"本番の確認をお願いします"}]"#,
    )
    .unwrap();
    let mut sources: Vec<_> = store
        .open_candidates()
        .unwrap()
        .into_iter()
        .map(|c| c.source)
        .collect();
    sources.sort();
    assert_eq!(sources, vec!["teams".to_string(), "ticket".to_string()]);
}

#[test]
fn accepted_ticket_closes_when_observation_is_closed() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    dayloop::intake::tickets::ingest(
        &store,
        r#"[{"source_ref":"ticket:ABC-1","title":"請求書を送る","state":"closed"}]"#,
    )
    .unwrap();
    let c = store.open_candidates().unwrap().remove(0);
    store.accept_candidate(&c.id, Some(date)).unwrap();
    let map = dayloop::observe::parse_fixture(
        r#"[{"source_ref":"ticket:ABC-1","state":"closed"}]"#,
    )
    .unwrap();
    dayloop::observe::apply(&store, date, &map).unwrap();
    let tasks = store.tasks_for_day(date).unwrap();
    assert_eq!(tasks[0].state, dayloop::model::State::Done);
    assert_eq!(tasks[0].source_ref.as_deref(), Some("ticket:ABC-1"));
}
