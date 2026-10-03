//! Spec §5 invariants 1–5, each as one test. Uses an isolated DAYLOOP_HOME.

use std::path::PathBuf;
use std::sync::{Arc, Barrier, Mutex};

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
    let closed_at = store.get_day(d).unwrap().unwrap().closed_at.unwrap();
    store.close_day(d).unwrap().unwrap();
    assert_eq!(store.get_day(d).unwrap().unwrap().closed_at.as_deref(), Some(closed_at.as_str()));
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

#[test]
fn closed_dates_reject_open_work_entries_without_partial_changes() {
    let home = Home::new();
    let store = home.store();
    let closed = "2026-09-10";
    assert!(store.close_day(closed).unwrap().is_ok());
    assert!(store.add_task("直接追加", None, None, "manual", None, Some(closed)).is_err());
    assert!(store.tasks_for_day(closed).unwrap().is_empty());

    let backlog = store.add_task("予定前", None, None, "manual", None, None).unwrap();
    assert!(store.schedule(&backlog.id, closed).is_err());
    let unchanged = store.get_task(&backlog.id).unwrap();
    assert_eq!(unchanged.state, State::Backlog);
    assert!(unchanged.plan_date.is_none());

    let source_day = "2026-09-09";
    let carried = store.add_task("持ち越し元", None, None, "manual", None, Some(source_day)).unwrap();
    assert!(store.carry_over(&carried.id, "理由", closed, None).is_err());
    assert_eq!(store.get_task(&carried.id).unwrap().state, State::Planned);
    assert!(store.tasks_for_day(closed).unwrap().is_empty());

    let split = store.add_task("分割元", None, None, "manual", None, Some(source_day)).unwrap();
    assert!(store.split(&split.id, &["前半".into(), "後半".into()], "理由", closed).is_err());
    assert_eq!(store.get_task(&split.id).unwrap().state, State::Planned);
    assert!(store.tasks_for_day(closed).unwrap().is_empty());

    let candidate = store.add_candidate("候補", "manual", Some("candidate:closed")).unwrap().unwrap();
    assert!(store.accept_candidate(&candidate.id, Some(closed)).is_err());
    assert_eq!(store.get_candidate(&candidate.id).unwrap().status, "open");
    assert!(store.tasks_for_day(closed).unwrap().is_empty());

    std::fs::create_dir_all(dayloop::paths::days_dir()).unwrap();
    std::fs::write(dayloop::paths::day_md_path(closed), "# 2026-09-10\n\n## 今日のタスク\n- [ ] Markdown追加\n").unwrap();
    assert!(dayloop::markdown::import(&store, closed).is_err());
    assert!(store.tasks_for_day(closed).unwrap().is_empty());

    let active = store.add_task("移動対象", None, None, "manual", None, Some(source_day)).unwrap();
    assert!(store.schedule(&active.id, "2026-09-11").is_err());
    let unchanged = store.get_task(&active.id).unwrap();
    assert_eq!(unchanged.state, State::Planned);
    assert_eq!(unchanged.plan_date.as_deref(), Some(source_day));
}

#[test]
fn markdown_invalid_row_does_not_apply_earlier_rows() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-12";
    let task = store.add_task("既存", None, None, "manual", None, Some(date)).unwrap();
    std::fs::create_dir_all(dayloop::paths::days_dir()).unwrap();
    std::fs::write(dayloop::paths::day_md_path(date), format!(
        "# {date}\n\n## 今日のタスク\n- [x] 既存 <!-- id:{} -->\n- [x] 存在しない <!-- id:bad-id -->\n- [ ] 新規\n", task.id
    )).unwrap();
    assert!(dayloop::markdown::import(&store, date).is_err());
    assert_eq!(store.get_task(&task.id).unwrap().state, State::Planned);
    assert_eq!(store.tasks_for_day(date).unwrap().len(), 1);

    let foreign = store.add_task("別日", None, None, "manual", None, Some("2026-09-11")).unwrap();
    store.transition(&foreign.id, State::Done, None, None).unwrap();
    std::fs::write(dayloop::paths::day_md_path(date), format!(
        "# {date}\n\n## 今日のタスク\n- [x] 既存 <!-- id:{} -->\n- [x] 別日 <!-- id:{} -->\n", task.id, foreign.id
    )).unwrap();
    assert!(dayloop::markdown::import(&store, date).is_err());
    assert_eq!(store.get_task(&task.id).unwrap().state, State::Planned);
}

#[test]
fn concurrent_close_and_add_never_leave_open_work_on_closed_date() {
    let home = Home::new();
    for day in 13..23 {
        let date = format!("2026-09-{day:02}");
        let closer = home.store();
        let writer = home.store();
        let barrier = Arc::new(Barrier::new(2));
        let close_barrier = Arc::clone(&barrier);
        let add_barrier = Arc::clone(&barrier);
        let close_date = date.clone();
        let close = std::thread::spawn(move || {
            close_barrier.wait();
            closer.close_day(&close_date)
        });
        let add_date = date.clone();
        let add = std::thread::spawn(move || {
            add_barrier.wait();
            writer.add_task("競合", None, None, "manual", None, Some(&add_date))
        });
        let close_result = close.join().unwrap();
        let add_result = add.join().unwrap();
        assert!(close_result.is_ok() || add_result.is_ok(), "both race operations unexpectedly failed");
        let inspect = home.store();
        if inspect.get_day(&date).unwrap().and_then(|day| day.closed_at).is_some() {
            assert!(inspect.open_tasks_for_day(&date).unwrap().is_empty(), "{date} closed with open work");
        }
    }
}
