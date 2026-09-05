use dayloop::model::State;
use dayloop::store::{CarryBlocked, Store};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());
struct Home {
    path: PathBuf,
    old: Option<std::ffi::OsString>,
    _guard: MutexGuard<'static, ()>,
}
impl Home {
    fn new() -> Self {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = std::env::temp_dir().join(format!("dayloop-integrity-{}", ulid::Ulid::new()));
        let old = std::env::var_os("DAYLOOP_HOME");
        std::env::set_var("DAYLOOP_HOME", &path);
        Self {
            path,
            old,
            _guard: guard,
        }
    }
    fn store(&self) -> Store {
        Store::open().unwrap()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        match &self.old {
            Some(p) => std::env::set_var("DAYLOOP_HOME", p),
            None => std::env::remove_var("DAYLOOP_HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn every_planning_route_rejects_closed_days_without_consuming_source_work() {
    let home = Home::new();
    let s = home.store();
    s.close_day("2026-09-08").unwrap().unwrap();
    let t = s
        .add_task("元の作業", None, None, "manual", None, Some("2026-09-07"))
        .unwrap();
    let backlog = s
        .add_task("未計画", None, None, "manual", None, None)
        .unwrap();
    let c = s
        .add_candidate("候補", "manual", Some("test:closed"))
        .unwrap()
        .unwrap();
    assert!(s
        .add_task("追加", None, None, "manual", None, Some("2026-09-08"))
        .is_err());
    assert!(s.schedule(&backlog.id, "2026-09-08").is_err());
    assert!(s.carry_over(&t.id, "翌日", "2026-09-08", None).is_err());
    assert!(s
        .split(&t.id, &["A".into(), "B".into()], "分割", "2026-09-08")
        .is_err());
    assert!(s.accept_candidate(&c.id, Some("2026-09-08")).is_err());
    assert_eq!(s.get_task(&t.id).unwrap().state, State::Planned);
    assert_eq!(s.get_task(&backlog.id).unwrap().state, State::Backlog);
    assert_eq!(s.get_candidate(&c.id).unwrap().status, "open");
    assert!(s.tasks_for_day("2026-09-08").unwrap().is_empty());
}

#[test]
fn schedule_cannot_hide_prior_day_work() {
    let home = Home::new();
    let s = home.store();
    let t = s
        .add_task(
            "昨日の未完了",
            None,
            None,
            "manual",
            None,
            Some("2026-09-07"),
        )
        .unwrap();
    assert!(s.schedule(&t.id, "2026-09-08").is_err());
    s.transition(&t.id, State::InProgress, None, None).unwrap();
    assert!(s.schedule(&t.id, "2026-09-08").is_err());
    assert_eq!(
        s.get_task(&t.id).unwrap().plan_date.as_deref(),
        Some("2026-09-07")
    );
    let b = s
        .add_task("未計画", None, None, "manual", None, None)
        .unwrap();
    assert_eq!(
        s.schedule(&b.id, "2026-09-08").unwrap().state,
        State::Planned
    );
}

#[test]
fn unchanged_due_does_not_reset_carry_count_or_bypass_limit() {
    let home = Home::new();
    let s = home.store();
    let t = s
        .add_task(
            "期限付き",
            Some("2026-09-30"),
            None,
            "manual",
            None,
            Some("2026-09-07"),
        )
        .unwrap();
    let t = s.carry_over(&t.id, "一回目", "2026-09-08", None).unwrap();
    let t = s
        .carry_over(&t.id, "同じ期限", "2026-09-09", Some("2026-09-30"))
        .unwrap();
    assert_eq!(t.carried_count, 2);
    let t = s.carry_over(&t.id, "三回目", "2026-09-10", None).unwrap();
    assert!(s
        .carry_over(&t.id, "同じ期限", "2026-09-11", Some("2026-09-30"))
        .unwrap_err()
        .downcast_ref::<CarryBlocked>()
        .is_some());
    assert_eq!(
        s.carry_over(&t.id, "期限変更", "2026-09-11", Some("2026-10-01"))
            .unwrap()
            .carried_count,
        1
    );
}

#[test]
fn direct_confirm_plan_cannot_skip_an_unclosed_previous_day() {
    let home = Home::new();
    let s = home.store();
    let t = s
        .add_task("残件", None, None, "manual", None, Some("2026-09-07"))
        .unwrap();
    assert!(s.confirm_plan("2026-09-08").is_err());
    s.transition(&t.id, State::Done, None, None).unwrap();
    assert!(s.confirm_plan("2026-09-08").is_err());
    s.close_day("2026-09-07").unwrap().unwrap();
    s.confirm_plan("2026-09-08").unwrap();
}

#[test]
fn an_empty_previously_confirmed_or_reopened_day_must_also_be_closed() {
    let home = Home::new();
    let s = home.store();
    s.confirm_plan("2026-09-07").unwrap();
    assert!(s.confirm_plan("2026-09-08").is_err());
    s.close_day("2026-09-07").unwrap().unwrap();
    s.confirm_plan("2026-09-08").unwrap();
    s.reopen_day("2026-09-07", "訂正").unwrap();
    assert!(s.confirm_plan("2026-09-08").is_err());
}

#[test]
fn store_rejects_invalid_dates_and_blank_split_reasons_without_mutation() {
    let home = Home::new();
    let s = home.store();
    assert!(s
        .add_task("invalid", None, None, "manual", None, Some("2026-9-7"))
        .is_err());
    let t = s
        .add_task("元", None, None, "manual", None, Some("2026-09-07"))
        .unwrap();
    assert!(s
        .split(&t.id, &["A".into(), "B".into()], "  ", "2026-09-08")
        .is_err());
    assert!(s.carry_over(&t.id, "移動", "not-a-date", None).is_err());
    assert_eq!(s.get_task(&t.id).unwrap().state, State::Planned);
}

#[test]
fn markdown_import_rolls_back_all_edits_if_a_later_id_belongs_to_another_day() {
    let home = Home::new();
    let s = home.store();
    let today = s
        .add_task("今日", None, None, "manual", None, Some("2026-09-07"))
        .unwrap();
    let other = s
        .add_task("別日", None, None, "manual", None, Some("2026-09-08"))
        .unwrap();
    let path = dayloop::markdown::export(&s, "2026-09-07").unwrap();
    std::fs::write(
        &path,
        format!(
            "## 今日のタスク\n- [ ] 新規\n- [x] 今日 <!-- id:{} -->\n- [x] 別日 <!-- id:{} -->\n",
            today.id, other.id
        ),
    )
    .unwrap();
    assert!(dayloop::markdown::import(&s, "2026-09-07").is_err());
    assert_eq!(s.tasks_for_day("2026-09-07").unwrap().len(), 1);
    assert_eq!(s.get_task(&today.id).unwrap().state, State::Planned);
    assert_eq!(s.get_task(&other.id).unwrap().state, State::Planned);
}

#[test]
fn markdown_import_rolls_back_before_an_invalid_id_and_rejects_closed_day_additions() {
    let home = Home::new();
    let s = home.store();
    let path = dayloop::markdown::export(&s, "2026-09-07").unwrap();
    std::fs::write(
        &path,
        "## 今日のタスク\n- [ ] 新規\n- [x] 不明 <!-- id:UNKNOWN -->\n",
    )
    .unwrap();
    assert!(dayloop::markdown::import(&s, "2026-09-07").is_err());
    assert!(s.tasks_for_day("2026-09-07").unwrap().is_empty());
    s.close_day("2026-09-07").unwrap().unwrap();
    std::fs::write(&path, "## 今日のタスク\n- [ ] 新規\n").unwrap();
    assert!(dayloop::markdown::import(&s, "2026-09-07").is_err());
    assert!(s.tasks_for_day("2026-09-07").unwrap().is_empty());
}
