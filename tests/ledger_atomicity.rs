use dayloop::model::State;
use dayloop::store::Store;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("dayloop-atomic-{}", ulid::Ulid::new())))
    }
    fn db(&self) -> PathBuf {
        self.0.join("dayloop.db")
    }
    fn store(&self) -> Store {
        let store = Store::open_at(self.db()).unwrap();
        store.set_required_categories(&[]).unwrap();
        store
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn simultaneous_add_and_close_cannot_leave_open_work_on_a_closed_day() {
    let home = Home::new();
    let a = home.store();
    let b = home.store();
    let barrier = Arc::new(Barrier::new(2));
    let other = barrier.clone();
    let add = std::thread::spawn(move || {
        barrier.wait();
        a.add_task("並行追加", None, None, "manual", None, Some("2026-09-07"))
            .is_ok()
    });
    let close = std::thread::spawn(move || {
        other.wait();
        b.close_day("2026-09-07").unwrap().is_ok()
    });
    let added = add.join().unwrap();
    let closed = close.join().unwrap();
    assert_ne!(added, closed);
    assert!(home.store().integrity_issues().unwrap().is_empty());
}

#[test]
fn simultaneous_carry_creates_one_successor_only() {
    let home = Home::new();
    let a = home.store();
    let b = home.store();
    let t = a
        .add_task("並行持越し", None, None, "manual", None, Some("2026-09-07"))
        .unwrap();
    let id = t.id.clone();
    let other_id = id.clone();
    let barrier = Arc::new(Barrier::new(2));
    let other = barrier.clone();
    let one = std::thread::spawn(move || {
        barrier.wait();
        a.carry_over(&id, "翌日", "2026-09-08", None).is_ok()
    });
    let two = std::thread::spawn(move || {
        other.wait();
        b.carry_over(&other_id, "翌日", "2026-09-08", None).is_ok()
    });
    assert_ne!(one.join().unwrap(), two.join().unwrap());
    let store = home.store();
    assert_eq!(store.tasks_for_day("2026-09-08").unwrap().len(), 1);
    assert_eq!(store.get_task(&t.id).unwrap().state, State::Carried);
}

#[test]
fn database_failure_rolls_back_source_transition_and_all_split_children() {
    let home = Home::new();
    let store = home.store();
    let t = store
        .add_task("元", None, None, "manual", None, Some("2026-09-07"))
        .unwrap();
    let db = Connection::open(home.db()).unwrap();
    db.execute_batch("CREATE TRIGGER fail_child BEFORE INSERT ON tasks WHEN NEW.title='失敗' BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END;").unwrap();
    assert!(store
        .split(
            &t.id,
            &["作成できる".into(), "失敗".into()],
            "分割",
            "2026-09-08"
        )
        .is_err());
    assert!(store.tasks_for_day("2026-09-08").unwrap().is_empty());
    assert!(store.get_day("2026-09-08").unwrap().is_none());
    assert_eq!(store.get_task(&t.id).unwrap().state, State::Planned);
    db.execute_batch("DROP TRIGGER fail_child; CREATE TRIGGER fail_all BEFORE INSERT ON tasks BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END;").unwrap();
    assert!(store.carry_over(&t.id, "翌日", "2026-09-08", None).is_err());
    assert_eq!(store.get_task(&t.id).unwrap().state, State::Planned);
}

#[test]
fn unknown_state_and_wildcard_id_cannot_silently_complete_work() {
    let home = Home::new();
    let store = home.store();
    let t = store
        .add_task("残す", None, None, "manual", None, Some("2026-09-07"))
        .unwrap();
    assert!(store.transition("%", State::Done, None, None).is_err());
    let db = Connection::open(home.db()).unwrap();
    db.execute("UPDATE tasks SET state='future_state' WHERE id=?1", [&t.id])
        .unwrap();
    assert!(store.transition(&t.id, State::Done, None, None).is_err());
    assert!(store.close_day("2026-09-07").is_err());
    assert!(store
        .integrity_issues()
        .unwrap()
        .iter()
        .any(|i| i.code == "unknown_state"));
}

#[test]
fn missing_or_wildcard_candidate_ids_never_adopt_a_candidate() {
    let home = Home::new();
    let store = home.store();
    let candidate = store
        .add_candidate("本人の判断を待つ", "manual", None)
        .unwrap()
        .unwrap();
    assert!(store.accept_candidate("", None).is_err());
    assert!(store.accept_candidate("%", None).is_err());
    assert!(store.reject_candidate("_%").is_err());
    assert_eq!(store.get_candidate(&candidate.id).unwrap().status, "open");
    assert!(store.backlog().unwrap().is_empty());
}
