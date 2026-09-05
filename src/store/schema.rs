use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use std::path::Path;

pub(super) const VERSION: i64 = 1;

const AUDIT: &str = "CREATE TABLE IF NOT EXISTS ledger_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL, action TEXT NOT NULL, reason TEXT NOT NULL,
    created_at TEXT NOT NULL
);";

pub(super) fn check_readable(conn: &Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > VERSION {
        bail!("この台帳は新しいdayloopの形式です。診断には対応するバージョンを使ってください");
    }
    validate(conn, &table_names(conn)?, version)
}

pub(super) fn migrate(conn: &Connection, path: &Path, schema: &str) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > VERSION {
        bail!("この台帳は新しいdayloopで作成されています（schema {version}）。更新せず終了します");
    }
    let tables = table_names(conn)?;
    validate(conn, &tables, version)?;
    if version == VERSION {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let locked_version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if locked_version > VERSION {
        bail!("別のプロセスが台帳を更新しました。再起動してください");
    }
    let locked_tables = table_names(&tx)?;
    validate(&tx, &locked_tables, locked_version)?;
    if locked_version < VERSION && !locked_tables.is_empty() {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let dest = parent
            .join("backups")
            .join(format!("dayloop-pre-v{VERSION}-{}.db", ulid::Ulid::new()));
        // Keep the writer lock while a separate read connection copies the committed snapshot.
        let snapshot = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        snapshot.busy_timeout(std::time::Duration::from_secs(5))?;
        backup(&snapshot, &dest).context("移行前バックアップに失敗したため、台帳は移行しません")?;
    }
    if locked_version < VERSION {
        tx.execute_batch(schema)?;
        tx.execute_batch(AUDIT)?;
        tx.pragma_update(None, "user_version", VERSION)?;
    }
    tx.commit()?;
    Ok(())
}

fn table_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn validate(conn: &Connection, tables: &[String], version: i64) -> Result<()> {
    if !tables.is_empty() && !tables.iter().any(|t| t == "tasks") {
        bail!("既知のdayloop台帳形式ではありません。変更せず終了します");
    }
    let expected: &[(&str, &[&str])] = &[
        (
            "tasks",
            &[
                "id",
                "title",
                "source",
                "source_ref",
                "due",
                "estimate_min",
                "plan_date",
                "state",
                "state_reason",
                "carried_count",
                "evidence",
                "created_at",
                "closed_at",
                "carried_from",
            ],
        ),
        (
            "days",
            &["date", "plan_confirmed_at", "closed_at", "retro_note"],
        ),
        (
            "candidates",
            &[
                "id",
                "title",
                "source",
                "source_ref",
                "created_at",
                "status",
                "task_id",
            ],
        ),
        ("rejected_refs", &["source_ref"]),
        (
            "ledger_audit",
            &["id", "date", "action", "reason", "created_at"],
        ),
        (
            "events",
            &[
                "entry_id",
                "date",
                "start",
                "end",
                "subject",
                "location",
                "organizer",
                "is_organizer",
                "source",
                "synced_at",
            ],
        ),
    ];
    for (table, columns) in expected {
        if !tables.iter().any(|t| t == table) {
            if version == VERSION {
                bail!("台帳の{table}テーブルがありません。自動修復せず終了します");
            }
            continue;
        }
        // Table identifiers are constants above, never external input.
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let found: Vec<String> = stmt
            .query_map([], |r| r.get(1))?
            .collect::<rusqlite::Result<_>>()?;
        if columns.iter().any(|c| !found.iter().any(|f| f == c)) {
            bail!("台帳の{table}テーブルが既知の形式と異なります。変更せず終了します");
        }
    }
    Ok(())
}

/// SQLite reads a consistent snapshot, including committed WAL pages.
pub(super) fn backup(conn: &Connection, path: &Path) -> Result<()> {
    if path.exists() {
        bail!("バックアップ先は既に存在します。上書きしません");
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    // Reserve the filename without overwriting another process's output.
    let reservation = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    drop(reservation);
    let destination = path
        .to_str()
        .context("バックアップ先がUTF-8で表現できません")?;
    if let Err(error) = conn.execute("VACUUM INTO ?1", [destination]) {
        let _ = std::fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}
