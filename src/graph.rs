//! Decision graph. A human answer grows an edge. The next matching node skips the question.
//! Jev output never calls [`record`].

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::facts::{DueRelation, Fit};
use crate::model::{State, Task};
use crate::reason;
use crate::store::Store;
use crate::util::{self, next_workday};

pub const GRAPH_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS graph_nodes (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  key TEXT NOT NULL,
  UNIQUE(kind, key)
);
CREATE TABLE IF NOT EXISTS graph_edges (
  id TEXT PRIMARY KEY,
  node_id TEXT NOT NULL,
  to_choice TEXT NOT NULL,
  reason_code TEXT,
  support INTEGER NOT NULL,
  active INTEGER NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS graph_hits (
  id TEXT PRIMARY KEY,
  node_id TEXT NOT NULL,
  to_choice TEXT NOT NULL,
  reason_code TEXT,
  created_at TEXT NOT NULL
);
"#;

#[derive(Debug, Clone)]
pub struct Situation {
    pub title: String,
    pub sender: Option<String>,
    pub source: String,
    pub kind: String,
    pub due: DueRelation,
    pub fit: Fit,
    pub flagged: bool,
    pub carried_count: i64,
    /// Keeps a situation node from matching a different day by accident.
    pub anchor: String,
}

impl Situation {
    pub fn from_task(t: &Task, kind: &str) -> Self {
        let anchor = t.plan_date.clone().unwrap_or_else(util::today);
        Self {
            title: t.title.clone(),
            sender: None,
            source: t.source.clone(),
            kind: kind.to_string(),
            due: crate::facts::due_relation(&anchor, t.due.as_deref()),
            fit: Fit::Unknown,
            flagged: false,
            carried_count: t.carried_count,
            anchor,
        }
    }

    pub fn mail(title: &str, sender: &str, anchor: &str) -> Self {
        Self {
            title: title.to_string(),
            sender: Some(sender.to_string()),
            source: "outlook".into(),
            kind: "mail".into(),
            due: DueRelation::None,
            fit: Fit::Unknown,
            flagged: false,
            carried_count: 0,
            anchor: anchor.to_string(),
        }
    }

    pub fn intake(title: &str, source: &str, anchor: &str) -> Self {
        Self {
            title: title.to_string(),
            sender: None,
            source: source.to_string(),
            kind: "mail".into(),
            due: DueRelation::None,
            fit: Fit::Unknown,
            flagged: false,
            carried_count: 0,
            anchor: anchor.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Learned {
    pub id: String,
    pub to_choice: String,
    pub reason_code: Option<String>,
}

fn title_key(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn situation_key(sit: &Situation) -> String {
    format!(
        "{}|sender:{}|due:{}|fit:{}|flag:{}|carried:{}|source:{}|day:{}",
        sit.kind,
        sit.sender.as_deref().unwrap_or(""),
        sit.due.as_str(),
        sit.fit.as_str(),
        sit.flagged as u8,
        sit.carried_count,
        sit.source,
        sit.anchor
    )
}

fn node_id(store: &Store, kind: &str, key: &str) -> Result<String> {
    let conn = store.connection();
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM graph_nodes WHERE kind=?1 AND key=?2",
            params![kind, key],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    let id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO graph_nodes(id, kind, key) VALUES(?1,?2,?3)",
        params![id, kind, key],
    )?;
    Ok(id)
}

fn activate(store: &Store, node_id: &str, choice: &str, reason: Option<&str>) -> Result<String> {
    let conn = store.connection();
    let now = util::now();
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM graph_edges WHERE node_id=?1 AND to_choice=?2",
            params![node_id, choice],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        conn.execute(
            "UPDATE graph_edges SET active=0 WHERE node_id=?1 AND id<>?2",
            params![node_id, id],
        )?;
        conn.execute(
            "UPDATE graph_edges SET active=1, support=support+1, reason_code=?2, updated_at=?3 WHERE id=?1",
            params![id, reason, now],
        )?;
        return Ok(id);
    }
    conn.execute(
        "UPDATE graph_edges SET active=0 WHERE node_id=?1",
        params![node_id],
    )?;
    let id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO graph_edges(id, node_id, to_choice, reason_code, support, active, updated_at)
         VALUES(?1,?2,?3,?4,1,1,?5)",
        params![id, node_id, choice, reason, now],
    )?;
    Ok(id)
}

fn deactivate_all(store: &Store, node_id: &str) -> Result<()> {
    store.connection().execute(
        "UPDATE graph_edges SET active=0 WHERE node_id=?1",
        params![node_id],
    )?;
    Ok(())
}

fn note_sender(store: &Store, sender: &str, choice: &str, reason: Option<&str>) -> Result<()> {
    let key = sender.trim();
    if key.is_empty() {
        return Ok(());
    }
    let node = node_id(store, "sender", key)?;
    let conn = store.connection();
    let hit = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO graph_hits(id, node_id, to_choice, reason_code, created_at) VALUES(?1,?2,?3,?4,?5)",
        params![hit, node, choice, reason, util::now()],
    )?;
    let mut stmt = conn.prepare(
        "SELECT to_choice FROM graph_hits WHERE node_id=?1 ORDER BY created_at DESC, id DESC LIMIT 2",
    )?;
    let last: Vec<String> = stmt
        .query_map(params![node], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if last.len() == 2 && last[0] == last[1] {
        activate(store, &node, choice, reason)?;
    } else {
        deactivate_all(store, &node)?;
    }
    Ok(())
}

/// Remember a human answer. Title and situation become active immediately.
/// A sender edge becomes active only after two answers in a row with the same choice.
pub fn record(store: &Store, sit: &Situation, choice: &str, reason: Option<&str>) -> Result<()> {
    let title = node_id(store, "title", &title_key(&sit.title))?;
    activate(store, &title, choice, reason)?;
    let situation = node_id(store, "situation", &situation_key(sit))?;
    activate(store, &situation, choice, reason)?;
    if let Some(sender) = &sit.sender {
        note_sender(store, sender, choice, reason)?;
    }
    Ok(())
}

fn active_edge(store: &Store, kind: &str, key: &str) -> Result<Option<Learned>> {
    let conn = store.connection();
    let node = conn
        .query_row(
            "SELECT id FROM graph_nodes WHERE kind=?1 AND key=?2",
            params![kind, key],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    let Some(node) = node else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT id, to_choice, reason_code FROM graph_edges WHERE node_id=?1 AND active=1",
    )?;
    let rows: Vec<Learned> = stmt
        .query_map(params![node], |r| {
            Ok(Learned {
                id: r.get(0)?,
                to_choice: r.get(1)?,
                reason_code: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    match rows.len() {
        0 => Ok(None),
        1 => Ok(Some(rows.into_iter().next().unwrap())),
        _ => Ok(None),
    }
}

/// Title, then situation, then sender. A split active set yields no decision.
pub fn resolve(store: &Store, sit: &Situation) -> Result<Option<Learned>> {
    if let Some(edge) = active_edge(store, "title", &title_key(&sit.title))? {
        return Ok(Some(edge));
    }
    if let Some(edge) = active_edge(store, "situation", &situation_key(sit))? {
        return Ok(Some(edge));
    }
    if let Some(sender) = &sit.sender {
        if let Some(edge) = active_edge(store, "sender", sender.trim())? {
            return Ok(Some(edge));
        }
    }
    Ok(None)
}

pub fn state_line(store: &Store, sit: &Situation) -> Result<String> {
    let mut line = format!(
        "title={} due={} fit={} carried={}",
        title_key(&sit.title),
        sit.due.as_str(),
        sit.fit.as_str(),
        sit.carried_count
    );
    if let Some(sender) = &sit.sender {
        if let Some(edge) = active_edge(store, "sender", sender.trim())? {
            line.push_str(&format!(" sender_edge={}", edge.to_choice));
        }
    }
    Ok(line)
}

pub fn order_node_key(a: &str, b: &str) -> String {
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    format!("{x}\n{y}")
}

pub fn record_order(store: &Store, first: &str, second: &str) -> Result<()> {
    if first == second || first.trim().is_empty() || second.trim().is_empty() {
        anyhow::bail!("2つの違うタイトルが必要です");
    }
    let node = node_id(store, "order", &order_node_key(first, second))?;
    activate(store, &node, first, None)?;
    Ok(())
}

pub fn saved_first(store: &Store, a: &str, b: &str) -> Result<Option<String>> {
    let Some(edge) = active_edge(store, "order", &order_node_key(a, b))? else {
        return Ok(None);
    };
    if edge.to_choice == a || edge.to_choice == b {
        Ok(Some(edge.to_choice))
    } else {
        Ok(None)
    }
}

fn reason_text(edge: &Learned) -> Option<String> {
    edge.reason_code.as_deref().map(reason::reason_for_ledger)
}

fn apply_task_choice(store: &Store, t: &Task, edge: &Learned) -> Result<bool> {
    let reason = reason_text(edge);
    let applied = match edge.to_choice.as_str() {
        "done" => {
            store.transition_silent(&t.id, State::Done, None, None)?;
            true
        }
        "not_done" => {
            let Some(r) = reason else { return Ok(false) };
            store.transition_silent(&t.id, State::NotDone, Some(&r), None)?;
            true
        }
        "drop" => {
            let Some(r) = reason else { return Ok(false) };
            store.transition_silent(&t.id, State::Dropped, Some(&r), None)?;
            true
        }
        "start" => {
            store.transition_silent(&t.id, State::InProgress, None, None)?;
            true
        }
        "carry" => {
            let Some(r) = reason else { return Ok(false) };
            let base = t.plan_date.clone().unwrap_or_else(util::today);
            let to = next_workday(&base)?;
            match store.carry_over_silent(&t.id, &r, &to, None) {
                Ok(_) => true,
                Err(e) if e.downcast_ref::<crate::store::CarryBlocked>().is_some() => false,
                Err(e) => return Err(e),
            }
        }
        _ => false,
    };
    if applied {
        store.mark_decided(&t.id, &edge.id)?;
    }
    Ok(applied)
}

pub fn apply_known_tasks(store: &Store, date: &str) -> Result<usize> {
    let ids: Vec<String> = store
        .open_tasks_for_day(date)?
        .into_iter()
        .map(|t| t.id)
        .collect();
    let mut n = 0;
    for id in ids {
        let t = store.get_task(&id)?;
        if !t.state.is_open() {
            continue;
        }
        let sit = Situation::from_task(&t, "close");
        let Some(edge) = resolve(store, &sit)? else {
            continue;
        };
        if apply_task_choice(store, &t, &edge)? {
            n += 1;
        }
    }
    Ok(n)
}

pub fn apply_known_candidates(store: &Store, date: &str) -> Result<usize> {
    let cands = store.open_candidates()?;
    let mut n = 0;
    for c in cands {
        let sit = Situation::intake(&c.title, &c.source, date);
        let Some(edge) = resolve(store, &sit)? else {
            continue;
        };
        match edge.to_choice.as_str() {
            "today" => {
                store.accept_candidate_silent(&c.id, Some(date))?;
            }
            "backlog" => {
                store.accept_candidate_silent(&c.id, None)?;
            }
            "shelve" => store.shelve_candidate_silent(&c.id)?,
            "reject" => store.reject_candidate_silent(&c.id)?,
            _ => continue,
        }
        n += 1;
    }
    Ok(n)
}

impl Store {
    pub fn revise_task(&self, id: &str, choice: &str, reason: Option<&str>) -> Result<Task> {
        let t = self.get_task(id)?;
        let target = if t.state.is_open() {
            t.clone()
        } else {
            self.open_child(&t.id)?.unwrap_or(t.clone())
        };
        let mut sit = Situation::from_task(&target, "close");
        sit.title = t.title.clone();
        let code = match reason {
            Some(r) => Some(reason::canonical_code(r)?),
            None => None,
        };
        record(self, &sit, choice, code.as_deref())?;
        let edge = resolve(self, &sit)?.ok_or_else(|| anyhow::anyhow!("枝が書けませんでした"))?;
        if target.state.is_open() {
            apply_task_choice(self, &target, &edge)?;
        }
        self.get_task(&target.id)
    }

    pub fn revise_candidate(&self, id: &str, choice: &str) -> Result<crate::model::Candidate> {
        let c = self.get_candidate(id)?;
        let anchor = util::today();
        record(self, &Situation::intake(&c.title, &c.source, &anchor), choice, None)?;
        if c.status == "open" {
            match choice {
                "today" => {
                    self.accept_candidate_silent(&c.id, Some(&anchor))?;
                }
                "backlog" => {
                    self.accept_candidate_silent(&c.id, None)?;
                }
                "shelve" => self.shelve_candidate_silent(&c.id)?,
                "reject" => self.reject_candidate_silent(&c.id)?,
                _ => {}
            }
        }
        self.get_candidate(&c.id)
    }
}
