use dayloop::store::Store;
use rusqlite::Connection;
use std::path::PathBuf;

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dayloop-migrate-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn db(&self) -> PathBuf {
        self.0.join("dayloop.db")
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_future_database_is_refused_without_rewriting_its_bytes() {
    let home = Home::new();
    let db = Connection::open(home.db()).unwrap();
    db.execute_batch("CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('preserve'); PRAGMA user_version=99;").unwrap();
    drop(db);
    let before = std::fs::read(home.db()).unwrap();
    assert!(Store::open_at(home.db()).is_err());
    assert_eq!(std::fs::read(home.db()).unwrap(), before);
}

#[test]
fn diagnosis_rejects_missing_or_malformed_current_schema_without_repair() {
    for alteration in [
        "DROP TABLE ledger_audit",
        "DROP TABLE events",
        "ALTER TABLE ledger_audit RENAME COLUMN reason TO wrong_column",
    ] {
        let home = Home::new();
        drop(Store::open_at(home.db()).unwrap());
        let db = Connection::open(home.db()).unwrap();
        db.execute_batch(alteration).unwrap();
        drop(db);
        let before = std::fs::read(home.db()).unwrap();
        assert!(Store::open_read_only(home.db()).is_err(), "{alteration}");
        assert!(Store::open_at(home.db()).is_err(), "{alteration}");
        assert_eq!(std::fs::read(home.db()).unwrap(), before);
    }
}

#[test]
fn a_failed_migration_leaves_schema_version_and_legacy_rows_unchanged() {
    let home = Home::new();
    let store = Store::open_at(home.db()).unwrap();
    let task = store
        .add_task("移行前", None, None, "manual", None, None)
        .unwrap();
    drop(store);
    let db = Connection::open(home.db()).unwrap();
    db.execute_batch("DROP TABLE events; CREATE VIEW events AS SELECT 'conflict' AS entry_id; DROP TABLE ledger_audit; PRAGMA user_version=0;").unwrap();
    drop(db);
    assert!(Store::open_at(home.db()).is_err());
    let db = Connection::open(home.db()).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='ledger_audit'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT title FROM tasks WHERE id=?1", [&task.id], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "移行前"
    );
}

#[test]
fn a_legacy_ledger_is_backed_up_consistently_before_versioning() {
    let home = Home::new();
    let db = Connection::open(home.db()).unwrap();
    db.execute_batch("CREATE TABLE tasks (id TEXT PRIMARY KEY,title TEXT NOT NULL,source TEXT NOT NULL,source_ref TEXT,due TEXT,estimate_min INTEGER,plan_date TEXT,state TEXT NOT NULL,state_reason TEXT,carried_count INTEGER NOT NULL DEFAULT 0,evidence TEXT,created_at TEXT NOT NULL,closed_at TEXT,carried_from TEXT);
        INSERT INTO tasks(id,title,source,state,created_at) VALUES('legacy-id','keep me','manual','backlog','2026-09-05T12:00:00+09:00');").unwrap();
    drop(db);
    let store = Store::open_at(home.db()).unwrap();
    assert_eq!(store.backlog().unwrap()[0].id, "legacy-id");
    let backups: Vec<_> = std::fs::read_dir(home.0.join("backups"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open(&backups[0]).unwrap();
    let version: i64 = backup
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 0);
    let title: String = backup
        .query_row("SELECT title FROM tasks WHERE id='legacy-id'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(title, "keep me");
    let current = Connection::open(home.db()).unwrap();
    let version: i64 = current
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert!(version > 0);
}

#[test]
fn incompatible_legacy_tables_are_refused_instead_of_reinterpreted() {
    let home = Home::new();
    let db = Connection::open(home.db()).unwrap();
    db.execute_batch(
        "CREATE TABLE tasks(unknown TEXT); INSERT INTO tasks VALUES('do not replace');",
    )
    .unwrap();
    drop(db);
    let before = std::fs::read(home.db()).unwrap();
    assert!(Store::open_at(home.db()).is_err());
    assert_eq!(std::fs::read(home.db()).unwrap(), before);
}

#[test]
fn reopen_is_audited_and_never_rewrites_completed_task_outcomes() {
    let home = Home::new();
    let store = Store::open_at(home.db()).unwrap();
    let task = store
        .add_task("done", None, None, "manual", None, Some("2026-09-05"))
        .unwrap();
    store.confirm_plan("2026-09-05").unwrap();
    store
        .transition(&task.id, dayloop::model::State::Done, None, Some("proof"))
        .unwrap();
    store.close_day("2026-09-05").unwrap().unwrap();
    assert!(store.reopen_day("2026-09-05", " ").is_err());
    store.reopen_day("2026-09-05", "訂正").unwrap();
    let day = store.get_day("2026-09-05").unwrap().unwrap();
    assert!(day.closed_at.is_none() && day.plan_confirmed_at.is_none());
    assert_eq!(
        store.get_task(&task.id).unwrap().evidence.as_deref(),
        Some("proof")
    );
    assert!(store.reopen_day("2026-09-05", "repeat").is_err());
    let db = Connection::open(home.db()).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM ledger_audit WHERE action='reopen' AND reason='訂正'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn integrity_report_finds_legacy_contradictions_without_repairing_them() {
    let home = Home::new();
    let store = Store::open_at(home.db()).unwrap();
    let task = store
        .add_task(
            "legacy open",
            None,
            None,
            "manual",
            None,
            Some("2026-09-05"),
        )
        .unwrap();
    let raw = Connection::open(home.db()).unwrap();
    raw.execute("UPDATE days SET closed_at='2026-09-05T18:00:00+09:00'", [])
        .unwrap();
    let issues = store.integrity_issues().unwrap();
    assert!(issues
        .iter()
        .any(|i| i.code == "closed_day_open_task" && i.task_id.as_deref() == Some(&task.id)));
    assert!(store
        .get_day("2026-09-05")
        .unwrap()
        .unwrap()
        .closed_at
        .is_some());
    assert_eq!(
        store.get_task(&task.id).unwrap().state,
        dayloop::model::State::Planned
    );
}
