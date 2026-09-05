use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior,
};

use crate::model::{Candidate, Day, Event, State, Task};
use crate::paths;
use crate::util::{now, parse_date};

mod schema;

/// A task carried this many times cannot be carried again without a resolution.
pub const MAX_CARRY: i64 = 3;
/// Candidates older than this are shown first and flagged.
pub const CANDIDATE_STALE_DAYS: i64 = 7;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT 'manual',
  source_ref TEXT,
  due TEXT,
  estimate_min INTEGER,
  plan_date TEXT,
  state TEXT NOT NULL,
  state_reason TEXT,
  carried_count INTEGER NOT NULL DEFAULT 0,
  evidence TEXT,
  created_at TEXT NOT NULL,
  closed_at TEXT,
  carried_from TEXT
);
CREATE INDEX IF NOT EXISTS idx_tasks_plan_date ON tasks(plan_date);
CREATE TABLE IF NOT EXISTS days (
  date TEXT PRIMARY KEY,
  plan_confirmed_at TEXT,
  closed_at TEXT,
  retro_note TEXT
);
CREATE TABLE IF NOT EXISTS candidates (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  source TEXT NOT NULL,
  source_ref TEXT,
  created_at TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'open',
  task_id TEXT
);
CREATE TABLE IF NOT EXISTS rejected_refs (
  source_ref TEXT PRIMARY KEY
);
CREATE TABLE IF NOT EXISTS events (
  entry_id TEXT PRIMARY KEY,
  date TEXT NOT NULL,
  start TEXT NOT NULL,
  end TEXT NOT NULL,
  subject TEXT NOT NULL,
  location TEXT,
  organizer TEXT,
  is_organizer INTEGER NOT NULL DEFAULT 0,
  source TEXT NOT NULL,
  synced_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_date ON events(date);
CREATE TABLE IF NOT EXISTS required_categories (
  category TEXT PRIMARY KEY
);
CREATE TABLE IF NOT EXISTS review_sets (
  date TEXT PRIMARY KEY,
  prepared_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS reviews (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  date TEXT NOT NULL,
  category TEXT NOT NULL,
  required INTEGER NOT NULL,
  outcome TEXT NOT NULL,
  reason TEXT,
  task_id TEXT,
  candidate_id TEXT,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reviews_date_category ON reviews(date,category,id);
CREATE TABLE IF NOT EXISTS fetch_reports (
  id TEXT PRIMARY KEY,
  category TEXT NOT NULL,
  source TEXT NOT NULL,
  scope TEXT NOT NULL,
  status TEXT NOT NULL,
  item_count INTEGER NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT NOT NULL,
  reason TEXT
);
CREATE INDEX IF NOT EXISTS idx_fetch_reports_started_at ON fetch_reports(started_at);
CREATE TABLE IF NOT EXISTS observations (
  id TEXT PRIMARY KEY,
  category TEXT NOT NULL,
  source_ref TEXT NOT NULL,
  meeting_id TEXT,
  title TEXT NOT NULL,
  body TEXT,
  observed_at TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE(category,source_ref)
);
CREATE INDEX IF NOT EXISTS idx_observations_observed_at ON observations(observed_at);
CREATE TABLE IF NOT EXISTS source_observations (
  source_ref TEXT NOT NULL,
  observation_id TEXT NOT NULL,
  PRIMARY KEY(source_ref,observation_id),
  FOREIGN KEY(observation_id) REFERENCES observations(id)
);
CREATE TABLE IF NOT EXISTS routines (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  weekdays TEXT NOT NULL,
  enabled INTEGER NOT NULL,
  starts_on TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS routine_occurrences (
  routine_id TEXT NOT NULL,
  date TEXT NOT NULL,
  task_id TEXT NOT NULL,
  PRIMARY KEY(routine_id,date),
  FOREIGN KEY(routine_id) REFERENCES routines(id),
  FOREIGN KEY(task_id) REFERENCES tasks(id)
);
"#;

const TASK_COLS: &str = "id,title,source,source_ref,due,estimate_min,plan_date,state,state_reason,carried_count,evidence,created_at,closed_at";

#[derive(Debug)]
pub struct CarryBlocked {
    pub task: Task,
}

#[derive(Debug, serde::Serialize)]
pub struct LedgerIssue {
    pub code: String,
    pub date: Option<String>,
    pub task_id: Option<String>,
    pub detail: String,
}

impl fmt::Display for CarryBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "「{}」は既に {} 回持ち越されています。分割・取り下げ・期限変更のどれかを選んでください",
            self.task.title, self.task.carried_count
        )
    }
}

impl std::error::Error for CarryBlocked {}

fn row_to_task(r: &Row) -> rusqlite::Result<Task> {
    let st: String = r.get("state")?;
    let state = State::parse(&st).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            7,
            rusqlite::types::Type::Text,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "未知のタスク状態です。ledger check で診断してください",
            )
            .into(),
        )
    })?;
    Ok(Task {
        id: r.get("id")?,
        title: r.get("title")?,
        source: r.get("source")?,
        source_ref: r.get("source_ref")?,
        due: r.get("due")?,
        estimate_min: r.get("estimate_min")?,
        plan_date: r.get("plan_date")?,
        state,
        state_reason: r.get("state_reason")?,
        carried_count: r.get("carried_count")?,
        evidence: r.get("evidence")?,
        created_at: r.get("created_at")?,
        closed_at: r.get("closed_at")?,
    })
}

fn row_to_candidate(r: &Row) -> rusqlite::Result<Candidate> {
    Ok(Candidate {
        id: r.get("id")?,
        title: r.get("title")?,
        source: r.get("source")?,
        source_ref: r.get("source_ref")?,
        created_at: r.get("created_at")?,
        status: r.get("status")?,
    })
}

fn new_id() -> String {
    ulid::Ulid::new().to_string()
}

pub struct Store {
    pub(crate) conn: Connection,
    db_path: PathBuf,
}

impl Store {
    pub fn open() -> Result<Store> {
        Self::open_at(paths::db_path())
    }

    pub fn open_at(path: impl AsRef<Path>) -> Result<Store> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&conn, path, SCHEMA)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db_path = std::fs::canonicalize(path)?;
        std::fs::create_dir_all(parent.join("days"))?;
        Ok(Store { conn, db_path })
    }

    /// Inspect existing ledgers without creating directories, migrating or changing journal mode.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Store> {
        let db_path = std::fs::canonicalize(path)?;
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::check_readable(&conn)?;
        Ok(Store { conn, db_path })
    }

    pub fn data_dir(&self) -> &Path {
        self.db_path
            .parent()
            .expect("absolute database path has a parent")
    }

    pub fn backup_to(&self, path: &Path) -> Result<()> {
        schema::backup(&self.conn, path)
    }

    pub fn integrity_issues(&self) -> Result<Vec<LedgerIssue>> {
        let mut issues = Vec::new();
        let mut statement = self.conn.prepare("SELECT t.id,t.plan_date,t.state,t.state_reason,t.carried_count,t.closed_at,d.closed_at,d.date FROM tasks t LEFT JOIN days d ON d.date=t.plan_date ORDER BY t.id")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let date: Option<String> = row.get(1)?;
            let state: String = row.get(2)?;
            let reason: Option<String> = row.get(3)?;
            let count: i64 = row.get(4)?;
            let closed: Option<String> = row.get(5)?;
            let day_closed: Option<String> = row.get(6)?;
            let day_date: Option<String> = row.get(7)?;
            let mut report = |code: &str, detail: &str| {
                issues.push(LedgerIssue {
                    code: code.into(),
                    date: date.clone(),
                    task_id: Some(id.clone()),
                    detail: detail.into(),
                })
            };
            match State::parse(&state) {
                Some(s) => {
                    if s.is_open() && day_closed.is_some() {
                        report(
                            "closed_day_open_task",
                            "閉鎖済みの日に未確定タスクがあります",
                        );
                    }
                    if s.is_open() && date.is_none() || s == State::Backlog && date.is_some() {
                        report("state_date_mismatch", "状態と予定日が整合しません");
                    }
                    if s.needs_reason() && reason.as_deref().is_none_or(|r| r.trim().is_empty()) {
                        report("missing_reason", "終端状態に必要な理由がありません");
                    }
                    if s.is_terminal() != closed.is_some() {
                        report("state_closed_at_mismatch", "状態と確定時刻が整合しません");
                    }
                }
                None => report("unknown_state", "未知のタスク状態です"),
            }
            if count < 0 {
                report("negative_carry_count", "持ち越し回数が負の値です");
            }
            if date.is_some() && day_date.is_none() {
                report("missing_day", "予定日の day レコードがありません");
            }
            if date.as_deref().is_some_and(|d| parse_date(d).is_err()) {
                report("invalid_plan_date", "予定日が不正です");
            }
        }
        Ok(issues)
    }

    /// Serialize each read-check-write sequence, including nested Store operations.
    pub(crate) fn atomic<T>(&self, action: impl FnOnce() -> Result<T>) -> Result<T> {
        if self.conn.is_autocommit() {
            let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
            let value = action()?;
            tx.commit()?;
            Ok(value)
        } else {
            self.conn.execute_batch("SAVEPOINT dayloop_write")?;
            match action() {
                Ok(value) => {
                    self.conn.execute_batch("RELEASE dayloop_write")?;
                    Ok(value)
                }
                Err(error) => {
                    self.conn
                        .execute_batch("ROLLBACK TO dayloop_write; RELEASE dayloop_write")?;
                    Err(error)
                }
            }
        }
    }

    pub(crate) fn require_open_day(&self, date: &str) -> Result<()> {
        parse_date(date)?;
        if self
            .get_day(date)?
            .is_some_and(|day| day.closed_at.is_some())
        {
            bail!("{date} は閉鎖済みです。訂正する場合は理由を指定して reopen してください");
        }
        Ok(())
    }

    fn query_tasks(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Task>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st.query_map(p, row_to_task)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------- days ----------

    pub fn ensure_day(&self, date: &str) -> Result<()> {
        parse_date(date)?;
        self.atomic(|| {
            self.conn
                .execute("INSERT OR IGNORE INTO days(date) VALUES(?1)", params![date])?;
            self.ensure_review_set(date)
        })
    }

    pub fn get_day(&self, date: &str) -> Result<Option<Day>> {
        Ok(self
            .conn
            .query_row(
                "SELECT date,plan_confirmed_at,closed_at,retro_note FROM days WHERE date=?1",
                params![date],
                |r| {
                    Ok(Day {
                        date: r.get(0)?,
                        plan_confirmed_at: r.get(1)?,
                        closed_at: r.get(2)?,
                        retro_note: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn confirm_plan(&self, date: &str) -> Result<()> {
        self.atomic(|| {
            self.require_open_day(date)?;
            let previous = self.unclosed_days_before(date)?;
            if !previous.is_empty() {
                bail!("先に前日までを確定してください: {}", previous.join(", "));
            }
            self.ensure_day(date)?;
            self.conn.execute(
                "UPDATE days SET plan_confirmed_at=?2 WHERE date=?1",
                params![date, now()],
            )?;
            Ok(())
        })
    }

    /// Invariant 1: a day closes only when no planned/in_progress task remains.
    pub fn close_day(&self, date: &str) -> Result<std::result::Result<(), Vec<Task>>> {
        self.atomic(|| {
            parse_date(date)?;
            let open: Vec<_> = self
                .tasks_for_day(date)?
                .into_iter()
                .filter(|t| t.state.is_open())
                .collect();
            if !open.is_empty() {
                return Ok(Err(open));
            }
            self.ensure_day(date)?;
            self.require_reviews_resolved(date)?;
            self.conn.execute(
                "UPDATE days SET closed_at=COALESCE(closed_at, ?2) WHERE date=?1",
                params![date, now()],
            )?;
            Ok(Ok(()))
        })
    }

    pub fn reopen_day(&self, date: &str, reason: &str) -> Result<()> {
        parse_date(date)?;
        if reason.trim().is_empty() {
            bail!("再開には理由が必須です");
        }
        self.atomic(|| {
            if self.get_day(date)?.is_none_or(|d| d.closed_at.is_none()) {
                bail!("再開できるのは閉鎖済みの日だけです");
            }
            self.conn.execute(
                "UPDATE days SET closed_at=NULL, plan_confirmed_at=NULL WHERE date=?1",
                [date],
            )?;
            self.conn.execute(
                "INSERT INTO ledger_audit(date,action,reason,created_at) VALUES(?1,'reopen',?2,?3)",
                params![date, reason.trim(), now()],
            )?;
            Ok(())
        })
    }

    pub fn set_retro(&self, date: &str, note: &str) -> Result<()> {
        if note.trim().is_empty() {
            bail!("振り返りメモは空にできません");
        }
        self.atomic(|| {
            self.ensure_day(date)?;
            self.conn.execute(
                "UPDATE days SET retro_note=?2 WHERE date=?1",
                params![date, note.trim()],
            )?;
            Ok(())
        })
    }

    /// Invariant 5: prior unclosed ledger days, including empty/reopened days.
    pub fn unclosed_days_before(&self, date: &str) -> Result<Vec<String>> {
        parse_date(date)?;
        let mut st = self.conn.prepare(
            "SELECT date FROM days WHERE date < ?1 AND closed_at IS NULL
             UNION SELECT DISTINCT t.plan_date FROM tasks t
             LEFT JOIN days d ON d.date = t.plan_date
             WHERE t.plan_date IS NOT NULL AND t.plan_date < ?1
             AND (d.closed_at IS NULL OR t.state IN ('planned','in_progress'))
             ORDER BY 1",
        )?;
        let rows = st.query_map(params![date], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------- tasks ----------

    pub fn get_task(&self, idish: &str) -> Result<Task> {
        let up = idish.trim().to_uppercase();
        if up.is_empty() {
            bail!("タスク ID を指定してください");
        }
        let v = self.query_tasks(
            &format!("SELECT {TASK_COLS} FROM tasks WHERE upper(id) = ?1 OR substr(upper(id),1,length(?1)) = ?1 OR substr(upper(id),-length(?1)) = ?1 ORDER BY created_at"),
            &[&up],
        )?;
        match v.len() {
            0 => bail!("タスクが見つかりません: {idish}"),
            1 => Ok(v.into_iter().next().unwrap()),
            n => bail!(
                "{idish} に一致するタスクが {n} 件あります。ID をもう少し長く指定してください"
            ),
        }
    }

    pub fn tasks_for_day(&self, date: &str) -> Result<Vec<Task>> {
        self.query_tasks(
            &format!("SELECT {TASK_COLS} FROM tasks WHERE plan_date=?1 ORDER BY created_at"),
            &[&date],
        )
    }

    pub fn open_tasks_for_day(&self, date: &str) -> Result<Vec<Task>> {
        self.query_tasks(
            &format!(
                "SELECT {TASK_COLS} FROM tasks WHERE plan_date=?1 AND state IN ('planned','in_progress') ORDER BY created_at"
            ),
            &[&date],
        )
    }

    pub fn backlog(&self) -> Result<Vec<Task>> {
        self.query_tasks(
            &format!("SELECT {TASK_COLS} FROM tasks WHERE state='backlog' ORDER BY due IS NULL, due, created_at"),
            &[],
        )
    }

    pub fn tasks_in_range(&self, from: &str, to: &str) -> Result<Vec<Task>> {
        self.query_tasks(
            &format!(
                "SELECT {TASK_COLS} FROM tasks WHERE plan_date >= ?1 AND plan_date <= ?2 ORDER BY plan_date, created_at"
            ),
            &[&from, &to],
        )
    }

    /// Invariant 3: open tasks that have hit the carry limit.
    pub fn blocked_carry_tasks(&self) -> Result<Vec<Task>> {
        self.query_tasks(
            &format!(
                "SELECT {TASK_COLS} FROM tasks WHERE state IN ('planned','in_progress','backlog') AND carried_count >= ?1 ORDER BY carried_count DESC, created_at"
            ),
            &[&MAX_CARRY],
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_task(
        &self,
        title: &str,
        due: Option<&str>,
        estimate_min: Option<i64>,
        source: &str,
        source_ref: Option<&str>,
        plan_date: Option<&str>,
    ) -> Result<Task> {
        self.atomic(|| {
        let title = title.trim();
        if title.is_empty() {
            bail!("タイトルが空です");
        }
        if let Some(date) = plan_date { self.require_open_day(date)?; }
        if let Some(date) = due { parse_date(date)?; }
        if estimate_min.is_some_and(|n| n < 0) { bail!("見積は0以上の分数を指定してください"); }
        let id = new_id();
        let state = if plan_date.is_some() {
            State::Planned
        } else {
            State::Backlog
        };
        self.conn.execute(
            "INSERT INTO tasks(id,title,source,source_ref,due,estimate_min,plan_date,state,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![id, title, source, source_ref, due, estimate_min, plan_date, state.as_str(), now()],
        )?;
        if let Some(d) = plan_date {
            self.ensure_day(d)?;
        }
        self.get_task(&id)
        })
    }

    /// Schedule backlog only. Moving planned work requires a reasoned carry-over.
    pub fn schedule(&self, id: &str, date: &str) -> Result<Task> {
        self.atomic(|| {
        self.require_open_day(date)?;
        let t = self.get_task(id)?;
        if t.state != State::Backlog || t.plan_date.is_some() {
            bail!("schedule は未計画タスク専用です。予定済みの作業は理由付きで carry してください");
        }
        self.conn.execute(
            "UPDATE tasks SET plan_date=?2, state=?3 WHERE id=?1",
            params![t.id, date, State::Planned.as_str()],
        )?;
        self.ensure_day(date)?;
        self.get_task(&t.id)
        })
    }

    /// Invariant 2: not_done / dropped require a reason. Carry goes through carry_over.
    pub fn transition(
        &self,
        id: &str,
        to: State,
        reason: Option<&str>,
        evidence: Option<&str>,
    ) -> Result<Task> {
        self.atomic(|| {
        let t = self.get_task(id)?;
        if let Some(date) = &t.plan_date { self.require_open_day(date)?; }
        if t.state.is_terminal() {
            bail!("「{}」は既に {} です", t.title, t.state.label_ja());
        }
        if matches!(to, State::Carried | State::Backlog | State::Planned) {
            bail!("予定変更・持ち越しには専用の操作を使ってください");
        }
        if to.needs_reason() && reason.map(|r| r.trim().is_empty()).unwrap_or(true) {
            bail!("{} には理由が必須です", to.label_ja());
        }
        if to == State::InProgress && t.plan_date.is_none() {
            bail!("未計画のタスクは先に予定へ入れてください（plan か start --date）");
        }
        let closed = if to.is_terminal() { Some(now()) } else { None };
        self.conn.execute(
            "UPDATE tasks SET state=?2, state_reason=?3, evidence=COALESCE(?4, evidence), closed_at=?5 WHERE id=?1",
            params![t.id, to.as_str(), reason.map(str::trim), evidence, closed],
        )?;
        self.get_task(&t.id)
        })
    }

    /// Carry to `to_date`. Returns the new task. Errors with CarryBlocked at the limit
    /// unless a rescheduled due date is given (which resets the counter).
    pub fn carry_over(
        &self,
        id: &str,
        reason: &str,
        to_date: &str,
        reschedule_due: Option<&str>,
    ) -> Result<Task> {
        self.atomic(|| {
        self.require_open_day(to_date)?;
        let t = self.get_task(id)?;
        if let Some(date) = &t.plan_date { self.require_open_day(date)?; }
        if !t.state.is_open() && t.state != State::Backlog {
            bail!("「{}」は既に {} です", t.title, t.state.label_ja());
        }
        if reason.trim().is_empty() {
            bail!("持ち越しには理由が必須です");
        }
        let mut count = t.carried_count;
        let mut due = t.due.clone();
        if let Some(d) = reschedule_due {
            parse_date(d)?;
            if due.as_deref() != Some(d) {
                due = Some(d.to_string());
                count = 0;
            }
        }
        if count >= MAX_CARRY {
            return Err(CarryBlocked { task: t }.into());
        }
        let new_id = new_id();
        self.conn.execute(
            "UPDATE tasks SET state='carried', state_reason=?2, closed_at=?3 WHERE id=?1",
            params![t.id, reason.trim(), now()],
        )?;
        self.conn.execute(
            "INSERT INTO tasks(id,title,source,source_ref,due,estimate_min,plan_date,state,carried_count,created_at,carried_from)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'planned',?8,?9,?10)",
            params![
                new_id,
                t.title,
                t.source,
                t.source_ref,
                due,
                t.estimate_min,
                to_date,
                count + 1,
                now(),
                t.id
            ],
        )?;
        self.ensure_day(to_date)?;
        self.get_task(&new_id)
        })
    }

    /// Split into new tasks on `to_date`; the original is dropped with a split reason.
    pub fn split(
        &self,
        id: &str,
        titles: &[String],
        reason: &str,
        to_date: &str,
    ) -> Result<Vec<Task>> {
        self.atomic(|| {
        self.require_open_day(to_date)?;
        if reason.trim().is_empty() { bail!("分割には理由が必須です"); }
        let t = self.get_task(id)?;
        if let Some(date) = &t.plan_date { self.require_open_day(date)?; }
        if t.state.is_terminal() {
            bail!("「{}」は既に {} です", t.title, t.state.label_ja());
        }
        let titles: Vec<&str> = titles.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if titles.len() < 2 {
            bail!("分割先は 2 つ以上指定してください");
        }
        self.conn.execute(
            "UPDATE tasks SET state='dropped', state_reason=?2, closed_at=?3 WHERE id=?1",
            params![t.id, format!("分割: {}", reason.trim()), now()],
        )?;
        let mut ids = Vec::new();
        for title in titles {
            let nid = new_id();
            self.conn.execute(
                "INSERT INTO tasks(id,title,source,source_ref,due,estimate_min,plan_date,state,carried_count,created_at,carried_from)
                 VALUES(?1,?2,?3,?4,?5,NULL,?6,'planned',0,?7,?8)",
                params![nid, title, t.source, t.source_ref, t.due, to_date, now(), t.id],
            )?;
            ids.push(nid);
        }
        self.ensure_day(to_date)?;
        ids.iter().map(|i| self.get_task(i)).collect()
        })
    }

    // ---------- candidates ----------

    /// Returns None when the source_ref was already rejected or already exists.
    pub fn add_candidate(
        &self,
        title: &str,
        source: &str,
        source_ref: Option<&str>,
    ) -> Result<Option<Candidate>> {
        self.atomic(|| {
        let title = title.trim();
        if title.is_empty() {
            bail!("タイトルが空です");
        }
        if let Some(r) = source_ref {
            let rejected: Option<String> = self
                .conn
                .query_row("SELECT source_ref FROM rejected_refs WHERE source_ref=?1", params![r], |x| x.get(0))
                .optional()?;
            if rejected.is_some() {
                return Ok(None);
            }
            let exists: Option<String> = self
                .conn
                .query_row("SELECT id FROM candidates WHERE source_ref=?1", params![r], |x| x.get(0))
                .optional()?;
            if exists.is_some() {
                return Ok(None);
            }
        }
        let id = new_id();
        self.conn.execute(
            "INSERT INTO candidates(id,title,source,source_ref,created_at,status) VALUES(?1,?2,?3,?4,?5,'open')",
            params![id, title, source, source_ref, now()],
        )?;
        Ok(Some(self.get_candidate(&id)?))
        })
    }

    pub fn get_candidate(&self, idish: &str) -> Result<Candidate> {
        let up = idish.trim().to_uppercase();
        if up.is_empty() {
            bail!("候補 ID を指定してください");
        }
        let mut st = self.conn.prepare(
            "SELECT id,title,source,source_ref,created_at,status FROM candidates WHERE upper(id)=?1 OR substr(upper(id),1,length(?1))=?1 OR substr(upper(id),-length(?1))=?1",
        )?;
        let v: Vec<Candidate> = st
            .query_map(params![up], row_to_candidate)?
            .collect::<rusqlite::Result<_>>()?;
        match v.len() {
            0 => bail!("候補が見つかりません: {idish}"),
            1 => Ok(v.into_iter().next().unwrap()),
            n => bail!("{idish} に一致する候補が {n} 件あります"),
        }
    }

    /// Invariant 4: stale candidates first.
    pub fn open_candidates(&self) -> Result<Vec<Candidate>> {
        let mut st = self.conn.prepare(
            "SELECT id,title,source,source_ref,created_at,status FROM candidates WHERE status='open' ORDER BY created_at",
        )?;
        let v: Vec<Candidate> = st
            .query_map([], row_to_candidate)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn accept_candidate(&self, id: &str, plan_date: Option<&str>) -> Result<Task> {
        self.atomic(|| {
            let c = self.get_candidate(id)?;
            if c.status != "open" {
                bail!("候補「{}」は既に {} です", c.title, c.status);
            }
            let t = self.add_task(
                &c.title,
                None,
                None,
                &c.source,
                c.source_ref.as_deref(),
                plan_date,
            )?;
            self.conn.execute(
                "UPDATE candidates SET status='accepted', task_id=?2 WHERE id=?1",
                params![c.id, t.id],
            )?;
            Ok(t)
        })
    }

    pub fn reject_candidate(&self, id: &str) -> Result<()> {
        self.atomic(|| {
            let c = self.get_candidate(id)?;
            if c.status != "open" {
                bail!("候補「{}」は既に {} です", c.title, c.status);
            }
            self.conn.execute(
                "UPDATE candidates SET status='rejected' WHERE id=?1",
                params![c.id],
            )?;
            if let Some(r) = c.source_ref {
                self.conn.execute(
                    "INSERT OR IGNORE INTO rejected_refs(source_ref) VALUES(?1)",
                    params![r],
                )?;
            }
            Ok(())
        })
    }

    // ---------- events ----------

    /// Replace all events for `date` with `events` (empty clears that day).
    pub fn upsert_events(&self, date: &str, events: Vec<Event>) -> Result<()> {
        parse_date(date)?;
        self.atomic(|| {
        self.conn.execute("DELETE FROM events WHERE date=?1", params![date])?;
        for e in &events {
            self.conn.execute(
                "INSERT OR REPLACE INTO events(entry_id,date,start,end,subject,location,organizer,is_organizer,source,synced_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    e.entry_id,
                    date,
                    e.start,
                    e.end,
                    e.subject,
                    e.location,
                    e.organizer,
                    e.is_organizer as i64,
                    e.source,
                    e.synced_at
                ],
            )?;
        }
        Ok(()) })
    }

    /// Replace one source's successful calendar snapshot; a failed fetch must not call this.
    pub fn upsert_events_from_source(
        &self,
        date: &str,
        source: &str,
        events: Vec<Event>,
    ) -> Result<()> {
        parse_date(date)?;
        if source.trim().is_empty() {
            bail!("予定の取得元は必須です");
        }
        self.atomic(|| {
            self.conn.execute("DELETE FROM events WHERE date=?1 AND source=?2",params![date,source])?;
            for e in &events {
                if e.source!=source||e.date!=date {bail!("予定の取得元または日付が一致しません");}
                let owner:Option<String>=self.conn.query_row("SELECT source FROM events WHERE entry_id=?1",[&e.entry_id],|r|r.get(0)).optional()?;
                if owner.as_deref().is_some_and(|s|s!=source) {bail!("予定IDが別の取得元と競合しています");}
                self.conn.execute("INSERT OR REPLACE INTO events(entry_id,date,start,end,subject,location,organizer,is_organizer,source,synced_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![e.entry_id,date,e.start,e.end,e.subject,e.location,e.organizer,e.is_organizer as i64,source,e.synced_at])?;
            }
            Ok(())
        })
    }

    pub fn events_for_day(&self, date: &str) -> Result<Vec<Event>> {
        let mut st = self.conn.prepare(
            "SELECT entry_id,date,start,end,subject,location,organizer,is_organizer,source,synced_at
             FROM events WHERE date=?1 ORDER BY start",
        )?;
        let rows = st.query_map(params![date], |r| {
            Ok(Event {
                entry_id: r.get(0)?,
                date: r.get(1)?,
                start: r.get(2)?,
                end: r.get(3)?,
                subject: r.get(4)?,
                location: r.get(5)?,
                organizer: r.get(6)?,
                is_organizer: r.get::<_, i64>(7)? != 0,
                source: r.get(8)?,
                synced_at: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
