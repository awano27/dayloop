use std::fmt;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::model::{Candidate, Day, Event, State, Task};
use crate::paths;
use crate::util::now;

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
  carried_from TEXT,
  proposed_state TEXT,
  proposed_reason_code TEXT,
  proposal_confidence REAL,
  state_note TEXT,
  decided_by TEXT
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
"#;

const TASK_COLS: &str = "id,title,source,source_ref,due,estimate_min,plan_date,state,state_reason,carried_count,evidence,created_at,closed_at,carried_from,proposed_state,proposed_reason_code,proposal_confidence,state_note,decided_by";

#[derive(Debug)]
pub struct CarryBlocked {
    pub task: Task,
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
    Ok(Task {
        id: r.get("id")?,
        title: r.get("title")?,
        source: r.get("source")?,
        source_ref: r.get("source_ref")?,
        due: r.get("due")?,
        estimate_min: r.get("estimate_min")?,
        plan_date: r.get("plan_date")?,
        state: State::parse(&st).unwrap_or(State::Planned),
        state_reason: r.get("state_reason")?,
        carried_count: r.get("carried_count")?,
        evidence: r.get("evidence")?,
        created_at: r.get("created_at")?,
        closed_at: r.get("closed_at")?,
        carried_from: r.get("carried_from")?,
        proposed_state: r.get("proposed_state")?,
        proposed_reason_code: r.get("proposed_reason_code")?,
        proposal_confidence: r.get("proposal_confidence")?,
        state_note: r.get("state_note")?,
        decided_by: r.get("decided_by")?,
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
    conn: Connection,
}

impl Store {
    pub fn open() -> Result<Store> {
        std::fs::create_dir_all(paths::data_dir())?;
        std::fs::create_dir_all(paths::days_dir())?;
        let conn = Connection::open(paths::db_path())?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        let store = Store { conn };
        crate::graph::seed_work_steps(&store)?;
        Ok(store)
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }

    fn query_tasks(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Task>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st.query_map(p, row_to_task)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------- days ----------

    pub fn ensure_day(&self, date: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO days(date) VALUES(?1)",
            params![date],
        )?;
        Ok(())
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

    /// Invariant 5: today's plan cannot be confirmed while an earlier day still has open tasks.
    pub fn confirm_plan(&self, date: &str) -> Result<()> {
        for d in self.unclosed_days_before(date)? {
            if !self.open_tasks_for_day(&d)?.is_empty() {
                bail!("{d} が未確定です。先にその日を閉じてください");
            }
        }
        self.ensure_day(date)?;
        self.conn.execute(
            "UPDATE days SET plan_confirmed_at=?2 WHERE date=?1",
            params![date, now()],
        )?;
        Ok(())
    }

    /// Invariant 1: a day closes only when no planned/in_progress task remains.
    pub fn close_day(&self, date: &str) -> Result<std::result::Result<(), Vec<Task>>> {
        let open = self.open_tasks_for_day(date)?;
        if !open.is_empty() {
            return Ok(Err(open));
        }
        self.ensure_day(date)?;
        self.conn.execute(
            "UPDATE days SET closed_at=?2 WHERE date=?1",
            params![date, now()],
        )?;
        Ok(Ok(()))
    }

    pub fn set_retro(&self, date: &str, note: &str) -> Result<()> {
        self.ensure_day(date)?;
        self.conn.execute(
            "UPDATE days SET retro_note=?2 WHERE date=?1",
            params![date, note],
        )?;
        Ok(())
    }

    /// Invariant 5: dates before `date` that had tasks but were never closed.
    pub fn unclosed_days_before(&self, date: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare(
            "SELECT DISTINCT t.plan_date FROM tasks t
             LEFT JOIN days d ON d.date = t.plan_date
             WHERE t.plan_date IS NOT NULL AND t.plan_date < ?1 AND d.closed_at IS NULL
             ORDER BY t.plan_date",
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
            &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?1 OR id LIKE ?2 OR id LIKE ?3 ORDER BY created_at"),
            &[&up, &format!("{up}%"), &format!("%{up}")],
        )?;
        match v.len() {
            0 => bail!("タスクが見つかりません: {idish}"),
            1 => Ok(v.into_iter().next().unwrap()),
            n => bail!("{idish} に一致するタスクが {n} 件あります。ID をもう少し長く指定してください"),
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
        let title = title.trim();
        if title.is_empty() {
            bail!("タイトルが空です");
        }
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
    }

    /// Move a backlog (or planned) task onto a date.
    pub fn schedule(&self, id: &str, date: &str) -> Result<Task> {
        let t = self.get_task(id)?;
        if t.state.is_terminal() {
            bail!("「{}」は既に {} です", t.title, t.state.label_ja());
        }
        let state = if t.state == State::InProgress {
            State::InProgress
        } else {
            State::Planned
        };
        self.conn.execute(
            "UPDATE tasks SET plan_date=?2, state=?3 WHERE id=?1",
            params![t.id, date, state.as_str()],
        )?;
        self.ensure_day(date)?;
        self.get_task(&t.id)
    }

    /// Invariant 2: not_done / dropped require a reason. Carry goes through carry_over.
    /// A human call grows the decision graph. Graph replay uses [`Self::transition_silent`].
    pub fn transition(
        &self,
        id: &str,
        to: State,
        reason: Option<&str>,
        evidence: Option<&str>,
    ) -> Result<Task> {
        self.transition_in(id, to, reason, evidence, true)
    }

    pub(crate) fn transition_silent(
        &self,
        id: &str,
        to: State,
        reason: Option<&str>,
        evidence: Option<&str>,
    ) -> Result<Task> {
        self.transition_in(id, to, reason, evidence, false)
    }

    fn transition_in(
        &self,
        id: &str,
        to: State,
        reason: Option<&str>,
        evidence: Option<&str>,
        learn: bool,
    ) -> Result<Task> {
        let t = self.get_task(id)?;
        if t.state.is_terminal() {
            bail!("「{}」は既に {} です", t.title, t.state.label_ja());
        }
        if to == State::Carried {
            bail!("持ち越しは carry を使ってください");
        }
        if to.needs_reason() && reason.map(|r| r.trim().is_empty()).unwrap_or(true) {
            bail!("{} には理由が必須です", to.label_ja());
        }
        let stored_reason = match reason {
            Some(r) if !r.trim().is_empty() => Some(crate::reason::reason_for_ledger(
                &crate::reason::canonical_code(r)?,
            )),
            _ => None,
        };
        if to == State::InProgress && t.plan_date.is_none() {
            bail!("未計画のタスクは先に予定へ入れてください（plan か start --date）");
        }
        let closed = if to.is_terminal() { Some(now()) } else { None };
        self.conn.execute(
            "UPDATE tasks SET state=?2, state_reason=?3, evidence=COALESCE(?4, evidence), closed_at=?5 WHERE id=?1",
            params![t.id, to.as_str(), stored_reason, evidence, closed],
        )?;
        if learn {
            if let Some(choice) = choice_of(to) {
                let code = reason.and_then(|r| crate::reason::canonical_code(r).ok());
                crate::graph::record(self, &crate::graph::Situation::from_task(&t, "close"), choice, code.as_deref())?;
            }
        }
        self.get_task(&t.id)
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
        self.carry_in(id, reason, to_date, reschedule_due, true)
    }

    pub(crate) fn carry_over_silent(
        &self,
        id: &str,
        reason: &str,
        to_date: &str,
        reschedule_due: Option<&str>,
    ) -> Result<Task> {
        self.carry_in(id, reason, to_date, reschedule_due, false)
    }

    fn carry_in(
        &self,
        id: &str,
        reason: &str,
        to_date: &str,
        reschedule_due: Option<&str>,
        learn: bool,
    ) -> Result<Task> {
        let t = self.get_task(id)?;
        if !t.state.is_open() && t.state != State::Backlog {
            bail!("「{}」は既に {} です", t.title, t.state.label_ja());
        }
        let code = crate::reason::canonical_code(reason)?;
        let ledger = crate::reason::reason_for_ledger(&code);
        let mut count = t.carried_count;
        let mut due = t.due.clone();
        if let Some(d) = reschedule_due {
            due = Some(d.to_string());
            count = 0;
        }
        if count >= MAX_CARRY {
            return Err(CarryBlocked { task: t }.into());
        }
        let new_id = new_id();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE tasks SET state='carried', state_reason=?2, closed_at=?3 WHERE id=?1",
            params![t.id, ledger, now()],
        )?;
        tx.execute(
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
        tx.execute("INSERT OR IGNORE INTO days(date) VALUES(?1)", params![to_date])?;
        tx.commit()?;
        if learn {
            crate::graph::record(
                self,
                &crate::graph::Situation::from_task(&t, "close"),
                "carry",
                Some(&code),
            )?;
        }
        self.get_task(&new_id)
    }

    pub(crate) fn mark_decided(&self, id: &str, edge_id: &str) -> Result<()> {
        let t = self.get_task(id)?;
        self.conn.execute(
            "UPDATE tasks SET decided_by=?2 WHERE id=?1",
            params![t.id, format!("graph:{edge_id}")],
        )?;
        Ok(())
    }

    pub(crate) fn mark_observed(&self, id: &str, source_ref: &str) -> Result<()> {
        let marker = format!("observe:{source_ref}");
        self.conn.execute(
            "UPDATE tasks SET decided_by=?2, evidence=?3 WHERE id=?1",
            params![id, marker, marker],
        )?;
        Ok(())
    }

    /// The still-open task created by carrying `parent_id`, if one exists.
    pub(crate) fn open_child(&self, parent_id: &str) -> Result<Option<Task>> {
        let parent = self.get_task(parent_id)?;
        let v = self.query_tasks(
            &format!(
                "SELECT {TASK_COLS} FROM tasks WHERE carried_from=?1 AND state IN ('planned','in_progress') ORDER BY created_at DESC"
            ),
            &[&parent.id],
        )?;
        Ok(v.into_iter().next())
    }

    /// Record a suggestion. The task stays open until [`Self::commit_proposal`].
    pub fn set_proposal(
        &self,
        id: &str,
        state: State,
        reason: Option<&str>,
        confidence: Option<f64>,
    ) -> Result<Task> {
        let t = self.get_task(id)?;
        let code = match reason {
            Some(r) if !r.trim().is_empty() => Some(crate::reason::canonical_code(r)?),
            _ => None,
        };
        self.conn.execute(
            "UPDATE tasks SET proposed_state=?2, proposed_reason_code=?3, proposal_confidence=?4 WHERE id=?1",
            params![t.id, state.as_str(), code, confidence],
        )?;
        self.get_task(&t.id)
    }

    /// Turn a stored proposal into a silent ledger move, then clear the proposal columns.
    pub fn commit_proposal(&self, id: &str) -> Result<Task> {
        let t = self.get_task(id)?;
        let Some(proposed) = t.proposed_state.clone() else {
            bail!("提案がありません");
        };
        let Some(to) = State::parse(&proposed) else {
            bail!("未知の提案状態: {proposed}");
        };
        let reason = t.proposed_reason_code.clone();
        if to == State::Carried {
            let base = t.plan_date.clone().unwrap_or_else(crate::util::today);
            let dest = crate::util::next_workday(&base)?;
            let r = reason.unwrap_or_else(|| "待ち".to_string());
            self.carry_over_silent(&t.id, &r, &dest, None)?;
        } else {
            self.transition_silent(&t.id, to, reason.as_deref(), None)?;
        }
        self.conn.execute(
            "UPDATE tasks SET proposed_state=NULL, proposed_reason_code=NULL, proposal_confidence=NULL WHERE id=?1",
            params![t.id],
        )?;
        self.get_task(&t.id)
    }

    /// Split into new tasks on `to_date`; the original is dropped with a split reason.
    pub fn split(&self, id: &str, titles: &[String], reason: &str, to_date: &str) -> Result<Vec<Task>> {
        let t = self.get_task(id)?;
        if t.state.is_terminal() {
            bail!("「{}」は既に {} です", t.title, t.state.label_ja());
        }
        let titles: Vec<&str> = titles.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if titles.len() < 2 {
            bail!("分割先は 2 つ以上指定してください");
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE tasks SET state='dropped', state_reason=?2, closed_at=?3 WHERE id=?1",
            params![t.id, format!("分割: {}", reason.trim()), now()],
        )?;
        let mut ids = Vec::new();
        for title in titles {
            let nid = new_id();
            tx.execute(
                "INSERT INTO tasks(id,title,source,source_ref,due,estimate_min,plan_date,state,carried_count,created_at,carried_from)
                 VALUES(?1,?2,?3,?4,?5,NULL,?6,'planned',0,?7,?8)",
                params![nid, title, t.source, t.source_ref, t.due, to_date, now(), t.id],
            )?;
            ids.push(nid);
        }
        tx.execute("INSERT OR IGNORE INTO days(date) VALUES(?1)", params![to_date])?;
        tx.commit()?;
        ids.iter().map(|i| self.get_task(i)).collect()
    }

    // ---------- candidates ----------

    /// Returns None when the source_ref was already rejected or already exists.
    pub fn add_candidate(&self, title: &str, source: &str, source_ref: Option<&str>) -> Result<Option<Candidate>> {
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
    }

    pub fn get_candidate(&self, idish: &str) -> Result<Candidate> {
        let up = idish.trim().to_uppercase();
        let mut st = self.conn.prepare(
            "SELECT id,title,source,source_ref,created_at,status FROM candidates WHERE id=?1 OR id LIKE ?2 OR id LIKE ?3",
        )?;
        let v: Vec<Candidate> = st
            .query_map(params![up, format!("{up}%"), format!("%{up}")], row_to_candidate)?
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
        let v: Vec<Candidate> = st.query_map([], row_to_candidate)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn accept_candidate(&self, id: &str, plan_date: Option<&str>) -> Result<Task> {
        self.accept_in(id, plan_date, true)
    }

    pub(crate) fn accept_candidate_silent(&self, id: &str, plan_date: Option<&str>) -> Result<Task> {
        self.accept_in(id, plan_date, false)
    }

    fn accept_in(&self, id: &str, plan_date: Option<&str>, learn: bool) -> Result<Task> {
        let c = self.get_candidate(id)?;
        if c.status != "open" {
            bail!("候補「{}」は既に {} です", c.title, c.status);
        }
        let t = self.add_task(&c.title, None, None, &c.source, c.source_ref.as_deref(), plan_date)?;
        self.conn.execute(
            "UPDATE candidates SET status='accepted', task_id=?2 WHERE id=?1",
            params![c.id, t.id],
        )?;
        if learn {
            let choice = if plan_date.is_some() { "today" } else { "backlog" };
            let anchor = plan_date
                .map(|s| s.to_string())
                .unwrap_or_else(crate::util::today);
            crate::graph::record(
                self,
                &crate::graph::Situation::intake(&c.title, &c.source, &anchor),
                choice,
                None,
            )?;
        }
        Ok(t)
    }

    pub fn reject_candidate(&self, id: &str) -> Result<()> {
        self.reject_in(id, true)
    }

    pub(crate) fn reject_candidate_silent(&self, id: &str) -> Result<()> {
        self.reject_in(id, false)
    }

    fn reject_in(&self, id: &str, learn: bool) -> Result<()> {
        let c = self.get_candidate(id)?;
        if c.status != "open" {
            bail!("候補「{}」は既に {} です", c.title, c.status);
        }
        self.conn.execute("UPDATE candidates SET status='rejected' WHERE id=?1", params![c.id])?;
        if let Some(r) = &c.source_ref {
            self.conn
                .execute("INSERT OR IGNORE INTO rejected_refs(source_ref) VALUES(?1)", params![r])?;
        }
        if learn {
            let anchor = crate::util::today();
            crate::graph::record(
                self,
                &crate::graph::Situation::intake(&c.title, &c.source, &anchor),
                "reject",
                None,
            )?;
        }
        Ok(())
    }

    /// Park a candidate. The row stays, and `rejected_refs` is left untouched.
    pub fn shelve_candidate(&self, id: &str) -> Result<()> {
        self.shelve_in(id, true)
    }

    pub(crate) fn shelve_candidate_silent(&self, id: &str) -> Result<()> {
        self.shelve_in(id, false)
    }

    fn shelve_in(&self, id: &str, learn: bool) -> Result<()> {
        let c = self.get_candidate(id)?;
        if c.status != "open" {
            bail!("候補「{}」は既に {} です", c.title, c.status);
        }
        self.conn
            .execute("UPDATE candidates SET status='shelved' WHERE id=?1", params![c.id])?;
        if learn {
            let anchor = crate::util::today();
            crate::graph::record(
                self,
                &crate::graph::Situation::intake(&c.title, &c.source, &anchor),
                "shelve",
                None,
            )?;
        }
        Ok(())
    }

    // ---------- events ----------

    /// Replace all events for `date` with `events` (empty clears that day).
    pub fn upsert_events(&self, date: &str, events: Vec<Event>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM events WHERE date=?1", params![date])?;
        for e in &events {
            tx.execute(
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
        tx.commit()?;
        Ok(())
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

fn choice_of(to: State) -> Option<&'static str> {
    match to {
        State::Done => Some("done"),
        State::NotDone => Some("not_done"),
        State::Dropped => Some("drop"),
        State::InProgress => Some("start"),
        _ => None,
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    for (column, decl) in [
        ("carried_from", "TEXT"),
        ("proposed_state", "TEXT"),
        ("proposed_reason_code", "TEXT"),
        ("proposal_confidence", "REAL"),
        ("state_note", "TEXT"),
        ("decided_by", "TEXT"),
    ] {
        ensure_column(conn, "tasks", column, decl)?;
    }
    conn.execute_batch(crate::graph::GRAPH_SCHEMA)?;
    conn.execute_batch(crate::screen::SCHEMA)?;
    conn.pragma_update(None, "user_version", 2)?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    if names.iter().any(|n| n.eq_ignore_ascii_case(column)) {
        return Ok(());
    }
    conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"), [])?;
    Ok(())
}
