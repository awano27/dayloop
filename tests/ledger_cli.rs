//! User-visible recovery and backup boundaries, exercised through the shipped CLI.
use std::path::PathBuf;
use std::process::{Command, Output};

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dayloop-ledger-cli-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_dayloop"))
            .args(args)
            .env("DAYLOOP_HOME", &self.0)
            .env_remove("DAYLOOP_LLM_ENDPOINT")
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn explicit_reopen_requires_a_reason_and_allows_a_corrected_plan() {
    let home = Home::new();
    home.ok(&["close", "--date", "2026-09-05", "--yes"]);
    assert!(!home
        .run(&["add", "late work", "--date", "2026-09-05"])
        .status
        .success());
    assert!(!home
        .run(&["reopen", "--date", "2026-09-05", "--reason", "  "])
        .status
        .success());
    home.ok(&[
        "reopen",
        "--date",
        "2026-09-05",
        "--reason",
        "追加作業を確認した",
    ]);
    home.ok(&["add", "追加作業", "--date", "2026-09-05"]);
    assert_eq!(
        home.run(&["close", "--date", "2026-09-05", "--yes"])
            .status
            .code(),
        Some(2)
    );
    let report: serde_json::Value =
        serde_json::from_str(&home.ok(&["ledger", "check", "--json"])).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["issues"], serde_json::json!([]));
}

#[test]
fn backup_preserves_data_and_refuses_to_overwrite_an_existing_backup() {
    let home = Home::new();
    home.ok(&["add", "バックアップ対象", "--backlog"]);
    let backup = home.0.join("backups").join("manual.db");
    let dest = backup.to_str().unwrap();
    home.ok(&["backup", "--output", dest]);
    let before = std::fs::read(&backup).unwrap();
    let db = rusqlite::Connection::open(&backup).unwrap();
    let title: String = db
        .query_row("SELECT title FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "バックアップ対象");
    drop(db);
    assert!(!home.run(&["backup", "--output", dest]).status.success());
    assert_eq!(std::fs::read(&backup).unwrap(), before);
}

#[test]
fn diagnosis_neither_creates_a_ledger_nor_migrates_an_existing_one() {
    let home = Home::new();
    assert!(!home.run(&["ledger", "check", "--json"]).status.success());
    assert_eq!(std::fs::read_dir(&home.0).unwrap().count(), 0);
    home.ok(&["add", "既存台帳", "--backlog"]);
    let path = home.0.join("dayloop.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE; PRAGMA user_version=0;",
    )
    .unwrap();
    drop(db);
    let before = std::fs::read(&path).unwrap();
    home.ok(&["ledger", "check", "--json"]);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!home.0.join("backups").exists());
}
