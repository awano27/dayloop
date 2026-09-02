//! Spec §5 invariants 1–5, each as one test. Uses an isolated DAYLOOP_HOME.

use std::path::PathBuf;
use std::sync::Mutex;

use dayloop::model::State;
use dayloop::store::{CarryBlocked, Store, CANDIDATE_STALE_DAYS, MAX_CARRY};
use dayloop::util::days_since;

static LOCK: Mutex<()> = Mutex::new(());

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
            "dayloop-inv-{}-{}",
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

/// 1. Day.closed_at が入るのは planned/in_progress が 0 件のときだけ。
#[test]
fn invariant_1_close_only_when_no_open_tasks() {
    let home = Home::new();
    let store = home.store();
    let d = "2026-09-03";
    let t = store
        .add_task("残件", None, None, "manual", None, Some(d))
        .unwrap();
    let open = store.close_day(d).unwrap();
    assert!(open.is_err(), "open が残る日は閉じられない");
    assert!(store.get_day(d).unwrap().unwrap().closed_at.is_none());

    store.transition(&t.id, State::Done, None, None).unwrap();
    store.close_day(d).unwrap().unwrap();
    assert!(store.get_day(d).unwrap().unwrap().closed_at.is_some());
}

/// 2. not_done / carried / dropped には state_reason が必須。
#[test]
fn invariant_2_reason_required() {
    let home = Home::new();
    let store = home.store();
    let d = "2026-09-03";
    let a = store.add_task("A", None, None, "manual", None, Some(d)).unwrap();
    let b = store.add_task("B", None, None, "manual", None, Some(d)).unwrap();
    let c = store.add_task("C", None, None, "manual", None, Some(d)).unwrap();

    assert!(store.transition(&a.id, State::NotDone, Some(""), None).is_err());
    assert!(store.transition(&a.id, State::NotDone, None, None).is_err());
    assert!(store.transition(&b.id, State::Dropped, Some("  "), None).is_err());
    assert!(store.carry_over(&c.id, "   ", "2026-09-04", None).is_err());

    store
        .transition(&a.id, State::NotDone, Some("待ち"), None)
        .unwrap();
    store
        .transition(&b.id, State::Dropped, Some("不要"), None)
        .unwrap();
    store.carry_over(&c.id, "翌日へ", "2026-09-04", None).unwrap();
}

/// 3. carried_count が 3 になった Task は再持ち越しできない（期限変更以外）。
#[test]
fn invariant_3_carry_limit() {
    let home = Home::new();
    let store = home.store();
    let t0 = store
        .add_task("経費", None, None, "manual", None, Some("2026-09-01"))
        .unwrap();
    let t1 = store.carry_over(&t0.id, "1", "2026-09-02", None).unwrap();
    let t2 = store.carry_over(&t1.id, "2", "2026-09-03", None).unwrap();
    let t3 = store.carry_over(&t2.id, "3", "2026-09-04", None).unwrap();
    assert_eq!(t3.carried_count, MAX_CARRY);
    let blocked = store.blocked_carry_tasks().unwrap();
    assert!(blocked.iter().any(|t| t.id == t3.id));

    let err = store.carry_over(&t3.id, "4", "2026-09-07", None).unwrap_err();
    assert!(err.downcast_ref::<CarryBlocked>().is_some());

    let t4 = store
        .carry_over(&t3.id, "期限見直し", "2026-09-07", Some("2026-09-10"))
        .unwrap();
    assert_eq!(t4.carried_count, 1);
}

/// 4. Candidate は accept/reject するまで消えず、7日放置は stale。
#[test]
fn invariant_4_candidates_persist_and_stale_after_7_days() {
    let home = Home::new();
    let store = home.store();
    let c = store
        .add_candidate("返信", "teams", Some("teams:1"))
        .unwrap()
        .unwrap();
    assert_eq!(store.open_candidates().unwrap().len(), 1);
    assert!(store
        .add_candidate("重複", "teams", Some("teams:1"))
        .unwrap()
        .is_none());

    store.reject_candidate(&c.id).unwrap();
    assert!(store.open_candidates().unwrap().is_empty());
    assert!(store
        .add_candidate("再", "teams", Some("teams:1"))
        .unwrap()
        .is_none());

    let again = store
        .add_candidate("別件", "outlook", Some("mail:2"))
        .unwrap()
        .unwrap();
    store
        .accept_candidate(&again.id, Some("2026-09-03"))
        .unwrap();
    assert!(store.open_candidates().unwrap().is_empty());

    assert_eq!(CANDIDATE_STALE_DAYS, 7);
    let old = (chrono::Local::now() - chrono::Duration::days(8))
        .format("%Y-%m-%dT00:00:00+09:00")
        .to_string();
    assert!(days_since(&old) >= CANDIDATE_STALE_DAYS);
}

/// 5. 前日の closed_at が null なら、当日 Plan は前日 Close から始まる。
#[test]
fn invariant_5_unclosed_previous_day_blocks_plan() {
    let home = Home::new();
    let store = home.store();
    store
        .add_task("昨日の残り", None, None, "manual", None, Some("2026-09-02"))
        .unwrap();
    let unclosed = store.unclosed_days_before("2026-09-03").unwrap();
    assert_eq!(unclosed, vec!["2026-09-02".to_string()]);

    store
        .add_task("もう一件", None, None, "manual", None, Some("2026-09-02"))
        .unwrap();
    let t = &store.open_tasks_for_day("2026-09-02").unwrap()[0];
    store.transition(&t.id, State::Done, None, None).unwrap();
    let t = &store.open_tasks_for_day("2026-09-02").unwrap()[0];
    store.transition(&t.id, State::Done, None, None).unwrap();
    store.close_day("2026-09-02").unwrap().unwrap();
    assert!(store.unclosed_days_before("2026-09-03").unwrap().is_empty());
}
