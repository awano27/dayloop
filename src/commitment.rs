//! Durable, human-confirmed promises. Source events and decisions are append-only;
//! daily task state is never changed here.
use crate::store::Store;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS commitments(
 id TEXT PRIMARY KEY, revision INTEGER NOT NULL, status TEXT NOT NULL,
 next_check TEXT, data TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS commitment_due ON commitments(status,next_check);
CREATE TABLE IF NOT EXISTS commitment_updates(
 id TEXT PRIMARY KEY, source TEXT NOT NULL, external_id TEXT NOT NULL,
 target_id TEXT, occurred_at TEXT NOT NULL, payload TEXT NOT NULL,
 UNIQUE(source,external_id));
CREATE TABLE IF NOT EXISTS commitment_decisions(
 id TEXT PRIMARY KEY, commitment_id TEXT NOT NULL, update_id TEXT UNIQUE,
 action TEXT NOT NULL, before_data TEXT, after_data TEXT NOT NULL,
 evidence TEXT NOT NULL, decided_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS commitment_operations(
 op_id TEXT PRIMARY KEY, payload TEXT NOT NULL, result TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS commitment_rejections(
 update_id TEXT PRIMARY KEY, evidence TEXT NOT NULL, decided_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS commitment_syncs(
 id TEXT PRIMARY KEY, source TEXT NOT NULL, status TEXT NOT NULL,
 at TEXT NOT NULL, evidence TEXT NOT NULL);
CREATE TRIGGER IF NOT EXISTS commitment_update_immutable BEFORE UPDATE ON commitment_updates BEGIN SELECT RAISE(ABORT,'source updates are immutable'); END;
CREATE TRIGGER IF NOT EXISTS commitment_update_no_delete BEFORE DELETE ON commitment_updates BEGIN SELECT RAISE(ABORT,'source updates are immutable'); END;
CREATE TRIGGER IF NOT EXISTS commitment_decision_immutable BEFORE UPDATE ON commitment_decisions BEGIN SELECT RAISE(ABORT,'decisions are immutable'); END;
CREATE TRIGGER IF NOT EXISTS commitment_decision_no_delete BEFORE DELETE ON commitment_decisions BEGIN SELECT RAISE(ABORT,'decisions are immutable'); END;
CREATE TRIGGER IF NOT EXISTS commitment_sync_immutable BEFORE UPDATE ON commitment_syncs BEGIN SELECT RAISE(ABORT,'sync records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS commitment_sync_no_delete BEFORE DELETE ON commitment_syncs BEGIN SELECT RAISE(ABORT,'sync records are immutable'); END;
CREATE TRIGGER IF NOT EXISTS commitment_rejection_immutable BEFORE UPDATE ON commitment_rejections BEGIN SELECT RAISE(ABORT,'rejections are immutable'); END;
CREATE TRIGGER IF NOT EXISTS commitment_rejection_no_delete BEFORE DELETE ON commitment_rejections BEGIN SELECT RAISE(ABORT,'rejections are immutable'); END;
"#;

fn atomic<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT commitment_transaction")?;
    match f() {
        Ok(v) => {
            conn.execute_batch("RELEASE commitment_transaction")?;
            Ok(v)
        }
        Err(e) => {
            conn.execute_batch(
                "ROLLBACK TO commitment_transaction; RELEASE commitment_transaction",
            )?;
            Err(e)
        }
    }
}
pub fn migrate(conn: &Connection) -> Result<()> {
    atomic(conn, || {
        conn.execute_batch(SCHEMA)?;
        Ok(())
    })
}
fn uid() -> String {
    ulid::Ulid::new().to_string()
}
fn timestamp(s: &str) -> Result<String> {
    Ok(DateTime::parse_from_rfc3339(s)
        .context("timestamp must be RFC3339 with offset")?
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Nanos, true))
}
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}
pub fn day_cutoff(date: &str) -> Result<String> {
    day_cutoff_at(date, Utc::now())
}

/// Past JST days include their final nanosecond; today and future views use
/// the actual instant. A future requested date never advances the clock.
pub fn day_cutoff_at(date: &str, at: DateTime<Utc>) -> Result<String> {
    let day = crate::util::parse_date(date)?;
    let offset = FixedOffset::east_opt(9 * 3600).unwrap();
    if day < at.with_timezone(&offset).date_naive() {
        return timestamp(&format!("{day}T23:59:59.999999999+09:00"));
    }
    Ok(at.to_rfc3339_opts(SecondsFormat::Nanos, true))
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    let s = v
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("{key} must be a string"))?;
    if s.trim().is_empty() {
        bail!("{key} must not be blank");
    }
    Ok(s)
}
fn optional<'a>(v: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => Ok(Some(required(v, key)?)),
    }
}
fn strict(v: &Value, keys: &[&str]) -> Result<()> {
    let obj = v.as_object().context("arguments must be an object")?;
    for k in obj.keys() {
        if !keys.contains(&k.as_str()) {
            bail!("unknown argument: {k}");
        }
    }
    Ok(())
}
fn confirmed(v: &Value) -> Result<()> {
    if v.get("confirmed") != Some(&Value::Bool(true)) {
        bail!("confirmed:true requires the person's explicit decision");
    }
    Ok(())
}
fn revision(v: &Value) -> Result<i64> {
    v.get("expected_revision")
        .and_then(Value::as_i64)
        .filter(|n| *n >= 0)
        .context("expected_revision must be a nonnegative integer")
}
fn get(conn: &Connection, id: &str) -> Result<Value> {
    let data: String = conn
        .query_row("SELECT data FROM commitments WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .context("commitment not found; exact ID required")?;
    Ok(serde_json::from_str(&data)?)
}
fn all(conn: &Connection) -> Result<Vec<Value>> {
    let mut st = conn.prepare("SELECT data FROM commitments ORDER BY id")?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
}
fn task_ids(conn: &Connection, v: &Value, key: &str) -> Result<Value> {
    let Some(value) = v.get(key) else {
        return Ok(json!([]));
    };
    let ids = value
        .as_array()
        .with_context(|| format!("{key} must be an array"))?;
    let mut roots = Vec::<String>::new();
    for id in ids {
        let mut id = id.as_str().context("task ID must be a string")?.to_string();
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(id.clone()) {
                bail!("task ancestry cycle");
            }
            let parent: Option<String> = conn
                .query_row("SELECT carried_from FROM tasks WHERE id=?1", [&id], |r| {
                    r.get(0)
                })
                .optional()?
                .context("task not found; exact ID required")?;
            match parent {
                Some(p) => id = p,
                None => break,
            }
        }
        if !roots.contains(&id) {
            roots.push(id);
        }
    }
    Ok(json!(roots))
}
fn descendants(conn: &Connection, roots: &Value) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for root in roots.as_array().context("corrupt task links")? {
        let mut st=conn.prepare("WITH RECURSIVE linked(id) AS (SELECT id FROM tasks WHERE id=?1 UNION SELECT tasks.id FROM tasks JOIN linked ON tasks.carried_from=linked.id) SELECT id FROM linked ORDER BY id")?;
        for id in st.query_map([root.as_str().context("corrupt task ID")?], |r| {
            r.get::<_, String>(0)
        })? {
            let id = id?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    Ok(ids)
}
fn presented(conn: &Connection, mut v: Value) -> Result<Value> {
    v["related_task_ids"] = json!(descendants(conn, &v["related_task_roots"])?);
    v["blocked_task_ids"] = json!(descendants(conn, &v["blocked_task_roots"])?);
    Ok(v)
}
pub fn blocked_task_ids(store: &Store) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let conn = store.connection();
    for v in all(conn)? {
        if v["status"] == "waiting" {
            for id in descendants(conn, &v["blocked_task_roots"])? {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
    }
    Ok(out)
}
/// Human promise decisions cannot be replayed from a task's learned daily graph.
pub fn protected_task_ids(store: &Store) -> Result<Vec<String>> {
    let conn = store.connection();
    let mut out = Vec::new();
    for v in all(conn)? {
        if !["waiting", "replied"].contains(&v["status"].as_str().unwrap_or("")) {
            continue;
        }
        for roots in ["related_task_roots", "blocked_task_roots"] {
            for id in descendants(conn, &v[roots])? {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
    }
    Ok(out)
}
fn create(conn: &Connection, args: &Value) -> Result<Value> {
    let next = timestamp(required(args, "next_check")?)?;
    let reply = optional(args, "reply_due")?.map(timestamp).transpose()?;
    Ok(
        json!({"id":uid(),"revision":1,"status":"waiting","request":required(args,"request")?,
        "counterparty":required(args,"counterparty")?,"reply_due":reply,"next_check":next,
        "source":required(args,"source")?,"source_ref":optional(args,"source_ref")?,"evidence":required(args,"evidence")?,
        "related_task_roots":task_ids(conn,args,"related_task_ids")?,"blocked_task_roots":task_ids(conn,args,"blocked_task_ids")?,
        "last_source_at":null,"created_at":now(),"updated_at":now(),"reply":null}),
    )
}
fn persist(
    conn: &Connection,
    before: Option<&Value>,
    after: &Value,
    update: Option<&str>,
    action: &str,
    evidence: &str,
) -> Result<()> {
    conn.execute("INSERT INTO commitments(id,revision,status,next_check,data) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,status=excluded.status,next_check=excluded.next_check,data=excluded.data",
        params![after["id"].as_str(),after["revision"].as_i64(),after["status"].as_str(),after["next_check"].as_str(),after.to_string()])?;
    conn.execute("INSERT INTO commitment_decisions(id,commitment_id,update_id,action,before_data,after_data,evidence,decided_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![uid(),after["id"].as_str(),update,action,before.map(Value::to_string),after.to_string(),evidence,now()])?;
    Ok(())
}
fn pending(conn: &Connection) -> Result<Vec<Value>> {
    let mut st=conn.prepare("SELECT u.id,u.payload FROM commitment_updates u LEFT JOIN commitment_decisions d ON d.update_id=u.id LEFT JOIN commitment_rejections r ON r.update_id=u.id WHERE d.id IS NULL AND r.update_id IS NULL ORDER BY u.occurred_at,u.id")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.map(|r| {
        let (id, p) = r?;
        let mut v: Value = serde_json::from_str(&p)?;
        v["id"] = json!(id);
        v["status"] = json!("pending");
        Ok(v)
    })
    .collect()
}

/// Stable ledger data for plan freshness, excluding random question operation
/// IDs and the advancing clock. Future promises also change the reviewed state.
pub(crate) fn plan_state(store: &Store) -> Result<Value> {
    Ok(json!({"commitments":all(store.connection())?,"pending_updates":pending(store.connection())?,"sync":syncs(store.connection())?}))
}
fn ingest(conn: &Connection, args: &Value) -> Result<Value> {
    strict(
        args,
        &[
            "source",
            "external_id",
            "kind",
            "occurred_at",
            "commitment_id",
            "request",
            "counterparty",
            "reply_due",
            "next_check",
            "source_ref",
            "evidence",
            "related_task_ids",
            "blocked_task_ids",
        ],
    )?;
    let source = required(args, "source")?;
    let external = required(args, "external_id")?;
    let kind = required(args, "kind")?;
    if ![
        "new_request",
        "additional_info",
        "deadline_change",
        "cancellation",
        "reply",
    ]
    .contains(&kind)
    {
        bail!("unsupported source update kind");
    }
    let fields = match kind {
        "deadline_change" => &["reply_due", "next_check"][..],
        "cancellation" => &[][..],
        "reply" => &["request", "next_check"][..],
        "additional_info" => &[
            "request",
            "counterparty",
            "related_task_ids",
            "blocked_task_ids",
        ][..],
        _ => &[
            "request",
            "counterparty",
            "reply_due",
            "next_check",
            "related_task_ids",
            "blocked_task_ids",
        ][..],
    };
    for key in [
        "request",
        "counterparty",
        "reply_due",
        "next_check",
        "related_task_ids",
        "blocked_task_ids",
    ] {
        if args.get(key).is_some() && !fields.contains(&key) {
            bail!("{kind} does not accept {key}");
        }
    }
    required(args, "evidence")?;
    let mut normalized = args.clone();
    normalized["occurred_at"] = json!(timestamp(required(args, "occurred_at")?)?);
    for key in ["reply_due", "next_check"] {
        if args.get(key).is_some() {
            normalized[key] = json!(timestamp(required(args, key)?)?);
        }
    }
    for key in ["request", "counterparty", "source_ref"] {
        optional(args, key)?;
    }
    task_ids(conn, args, "related_task_ids")?;
    task_ids(conn, args, "blocked_task_ids")?;
    let target = optional(args, "commitment_id")?;
    if let Some(id) = target {
        get(conn, id)?;
        if kind == "new_request" {
            bail!("new_request cannot target existing commitment");
        }
    }
    if kind == "new_request" {
        create(conn, &normalized)?;
    }
    if kind == "deadline_change"
        && normalized.get("reply_due").is_none()
        && normalized.get("next_check").is_none()
    {
        bail!("deadline_change needs reply_due or next_check");
    }
    let encoded_payload = normalized.to_string();
    let old: Option<(String, String)> = conn
        .query_row(
            "SELECT id,payload FROM commitment_updates WHERE source=?1 AND external_id=?2",
            params![source, external],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let id = if let Some((id, payload)) = old {
        if payload != encoded_payload {
            bail!("source identity conflict: same source/external_id with different payload");
        }
        id
    } else {
        let id = uid();
        conn.execute("INSERT INTO commitment_updates(id,source,external_id,target_id,occurred_at,payload) VALUES(?1,?2,?3,?4,?5,?6)",params![id,source,external,target,normalized["occurred_at"].as_str(),encoded_payload])?;
        id
    };
    normalized["id"] = json!(id);
    normalized["status"] = json!(if conn.query_row(
        "SELECT COUNT(*) FROM commitment_rejections WHERE update_id=?1",
        [&id],
        |r| r.get::<_, i64>(0)
    )? > 0
    {
        "rejected"
    } else if conn.query_row(
        "SELECT COUNT(*) FROM commitment_decisions WHERE update_id=?1",
        [&id],
        |r| r.get::<_, i64>(0)
    )? > 0
    {
        "resolved"
    } else {
        "pending"
    });
    let candidates = if target.is_none() && kind != "new_request" {
        all(conn)?
            .into_iter()
            .filter(|v| v["status"] == "waiting" || v["status"] == "replied")
            .map(|v| presented(conn, v))
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![]
    };
    Ok(json!({"update":normalized,"candidates":candidates}))
}
fn apply(conn: &Connection, args: &Value) -> Result<Value> {
    strict(
        args,
        &[
            "op_id",
            "update_id",
            "commitment_id",
            "expected_revision",
            "confirmed",
            "action",
            "evidence",
        ],
    )?;
    confirmed(args)?;
    let expected = revision(args)?;
    let id = required(args, "update_id")?;
    let payload: String = conn
        .query_row(
            "SELECT payload FROM commitment_updates WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .context("update not found")?;
    if conn.query_row(
        "SELECT (SELECT COUNT(*) FROM commitment_decisions WHERE update_id=?1)+(SELECT COUNT(*) FROM commitment_rejections WHERE update_id=?1)",
        [id],
        |r| r.get::<_, i64>(0),
    )? > 0
    {
        bail!("source update already resolved; replay the original op_id");
    }
    let update: Value = serde_json::from_str(&payload)?;
    let kind = required(&update, "kind")?;
    let action = optional(args, "action")?.unwrap_or("apply");
    if !["apply", "reject"].contains(&action) {
        bail!("action must be apply or reject");
    }
    let stored_target = optional(&update, "commitment_id")?;
    let chosen = optional(args, "commitment_id")?;
    if action == "reject" && kind != "new_request" && stored_target.is_none() && chosen.is_none() {
        if expected != 0 {
            bail!("unlinked rejection needs expected_revision:0");
        }
        let evidence = required(args, "evidence")?;
        conn.execute(
            "INSERT INTO commitment_rejections(update_id,evidence,decided_at) VALUES(?1,?2,?3)",
            params![id, evidence, now()],
        )?;
        return Ok(json!({"update":{"id":id,"status":"rejected","evidence":evidence}}));
    }
    if stored_target.is_some() && chosen.is_some() && stored_target != chosen {
        bail!("target conflicts with imported exact commitment ID");
    }
    if kind == "new_request" {
        if chosen.is_some() || expected != 0 {
            bail!("new_request needs expected_revision:0 and no existing target");
        }
        let mut after = create(conn, &update)?;
        if action == "reject" {
            after["status"] = json!("cancelled");
            after["next_check"] = Value::Null;
        }
        after["last_source_at"] = update["occurred_at"].clone();
        after["last_update"] = update.clone();
        let evidence = if action == "reject" {
            required(args, "evidence")?
        } else {
            required(&update, "evidence")?
        };
        persist(
            conn,
            None,
            &after,
            Some(id),
            if action == "reject" { "reject" } else { kind },
            evidence,
        )?;
        return Ok(json!({"commitment":presented(conn,after)?}));
    }
    let target = stored_target
        .or(chosen)
        .context("ambiguous update: select exact commitment_id and current expected_revision")?;
    let before = get(conn, target)?;
    if before["revision"].as_i64() != Some(expected) {
        bail!("stale commitment revision; refresh the question");
    }
    let mut after = before.clone();
    if action == "apply" {
        if before["status"] == "settled" || before["status"] == "cancelled" {
            bail!("terminal commitment cannot be changed; register a new promise");
        }
        if let Some(last) = before["last_source_at"].as_str() {
            if required(&update, "occurred_at")? <= last {
                bail!("old or equal-time conflicting source update cannot overwrite current state");
            }
        }
        if update.get("next_check").is_some() {
            if let Some(local) = before["next_check_guard_at"].as_str() {
                if required(&update, "occurred_at")? <= local {
                    bail!(
                        "old source deadline cannot overwrite a newer manual confirmation schedule"
                    );
                }
            }
        }
        match kind {
            "additional_info" => {
                for key in ["request", "counterparty", "source_ref"] {
                    if let Some(value) = optional(&update, key)? {
                        after[key] = json!(value);
                    }
                }
                for (key, root_key) in [
                    ("related_task_ids", "related_task_roots"),
                    ("blocked_task_ids", "blocked_task_roots"),
                ] {
                    if update.get(key).is_some() {
                        after[root_key] = task_ids(conn, &update, key)?;
                    }
                }
                after["evidence"] = update["evidence"].clone();
            }
            "deadline_change" => {
                for key in ["reply_due", "next_check"] {
                    if update.get(key).is_some() {
                        after[key] = update[key].clone();
                    }
                }
            }
            "cancellation" => {
                after["status"] = json!("cancelled");
                after["next_check"] = Value::Null;
            }
            "reply" => {
                after["status"] = json!("replied");
                after["reply"] = json!({"content":update.get("request").unwrap_or(&update["evidence"]),"evidence":update["evidence"],"source":update["source"],"source_ref":update["source_ref"],"received_at":update["occurred_at"]});
                if let Some(next) = update.get("next_check") {
                    after["next_check"] = next.clone();
                } else if before["next_check_guard_at"].as_str().is_none_or(|local| {
                    required(&update, "occurred_at").is_ok_and(|occurred| occurred > local)
                }) {
                    after["next_check"] = json!(now());
                }
            }
            _ => bail!("unsupported update kind"),
        }
        after["last_source_at"] = update["occurred_at"].clone();
        after["last_update"] = update.clone();
    }
    after["revision"] = json!(expected + 1);
    after["updated_at"] = json!(now());
    let evidence = if action == "reject" {
        required(args, "evidence")?
    } else {
        required(&update, "evidence")?
    };
    persist(
        conn,
        Some(&before),
        &after,
        Some(id),
        if action == "reject" { "reject" } else { kind },
        evidence,
    )?;
    Ok(json!({"commitment":presented(conn,after)?}))
}
fn check(conn: &Connection, args: &Value) -> Result<Value> {
    strict(
        args,
        &[
            "op_id",
            "commitment_id",
            "expected_revision",
            "confirmed",
            "action",
            "evidence",
            "next_check",
            "next_task_id",
        ],
    )?;
    confirmed(args)?;
    let expected = revision(args)?;
    let before = get(conn, required(args, "commitment_id")?)?;
    if before["revision"].as_i64() != Some(expected) {
        bail!("stale commitment revision; refresh the question");
    }
    if before["status"] == "settled" || before["status"] == "cancelled" {
        bail!("commitment already terminal");
    }
    let action = required(args, "action")?;
    let evidence = required(args, "evidence")?;
    let mut after = before.clone();
    match action {
        "defer" | "acknowledge" => {
            let at = timestamp(required(args, "next_check")?)?;
            if at <= now() {
                bail!("next_check must be in the future");
            }
            after["next_check"] = json!(at);
            after["next_check_guard_at"] = json!(now());
        }
        "settle" => {
            if args.get("next_check").is_some() {
                bail!("settlement cannot schedule another confirmation");
            }
            after["status"] = json!("settled");
            after["next_check"] = Value::Null;
        }
        _ => bail!("action must be defer, acknowledge, or settle"),
    }
    if let Some(id) = optional(args, "next_task_id")? {
        if before["status"] != "replied" {
            bail!("next_task_id requires a confirmed reply");
        }
        let roots = task_ids(conn, &json!({"related_task_ids":[id]}), "related_task_ids")?;
        let list = after["related_task_roots"]
            .as_array_mut()
            .context("corrupt task roots")?;
        for root in roots.as_array().unwrap() {
            if !list.contains(root) {
                list.push(root.clone());
            }
        }
        after["next_task_id"] = json!(id);
    }
    after["revision"] = json!(expected + 1);
    after["updated_at"] = json!(now());
    persist(conn, Some(&before), &after, None, action, evidence)?;
    Ok(json!({"commitment":presented(conn,after)?}))
}
fn sync(conn: &Connection, args: &Value) -> Result<Value> {
    strict(args, &["op_id", "source", "status", "at", "evidence"])?;
    let status = required(args, "status")?;
    if !["success", "partial", "failed", "unavailable"].contains(&status) {
        bail!("unsupported sync status");
    }
    let source = required(args, "source")?;
    let at = timestamp(required(args, "at")?)?;
    let evidence = required(args, "evidence")?;
    let id = uid();
    conn.execute(
        "INSERT INTO commitment_syncs(id,source,status,at,evidence) VALUES(?1,?2,?3,?4,?5)",
        params![id, source, status, at, evidence],
    )?;
    Ok(json!({"sync":{"id":id,"source":source,"status":status,"at":at,"evidence":evidence}}))
}
fn syncs(conn: &Connection) -> Result<Vec<Value>> {
    let mut st =
        conn.prepare("SELECT id,source,status,at,evidence FROM commitment_syncs ORDER BY at,id")?;
    let rows=st.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"source":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"at":r.get::<_,String>(3)?,"evidence":r.get::<_,String>(4)?})))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
pub fn dispatch(store: &Store, name: &str, args: &Value) -> Result<Value> {
    let conn = store.connection();
    match name {
        "commitment_list" => {
            strict(args, &["at"])?;
            let at = optional(args, "at")?
                .map(timestamp)
                .transpose()?
                .unwrap_or_else(now);
            let commitments = all(conn)?
                .into_iter()
                .map(|v| presented(conn, v))
                .collect::<Result<Vec<_>>>()?;
            let mut st=conn.prepare("SELECT u.id,u.payload,r.evidence,r.decided_at FROM commitment_rejections r JOIN commitment_updates u ON u.id=r.update_id ORDER BY r.rowid")?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?;
            let mut rejected = vec![];
            for row in rows {
                let (id, payload, evidence, decided_at) = row?;
                rejected.push(json!({"id":id,"status":"rejected","update":serde_json::from_str::<Value>(&payload)?,"evidence":evidence,"decided_at":decided_at}));
            }
            return Ok(
                json!({"commitments":commitments,"pending_updates":pending(conn)?,"rejected_updates":rejected,"sync":syncs(conn)?,"questions":due_questions(store,&at)?}),
            );
        }
        "commitment_history" => {
            strict(args, &["commitment_id"])?;
            let id = required(args, "commitment_id")?;
            get(conn, id)?;
            let mut st=conn.prepare("SELECT id,update_id,action,before_data,after_data,evidence,decided_at FROM commitment_decisions WHERE commitment_id=?1 ORDER BY rowid")?;
            let rows = st.query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })?;
            let mut decisions = vec![];
            let mut updates = vec![];
            for row in rows {
                let (d, u, a, b, f, e, t) = row?;
                decisions.push(json!({"id":d,"update_id":u,"action":a,"before":b.map(|s|serde_json::from_str::<Value>(&s)).transpose()?,"after":serde_json::from_str::<Value>(&f)?,"evidence":e,"decided_at":t}));
                if let Some(u) = u {
                    let p: String = conn.query_row(
                        "SELECT payload FROM commitment_updates WHERE id=?1",
                        [&u],
                        |r| r.get(0),
                    )?;
                    let mut value: Value = serde_json::from_str(&p)?;
                    value["id"] = json!(u);
                    updates.push(value);
                }
            }
            for mut u in pending(conn)? {
                if u["commitment_id"] == id {
                    u["status"] = json!("pending");
                    updates.push(u);
                }
            }
            return Ok(json!({"decisions":decisions,"updates":updates}));
        }
        "commitment_ingest" => return atomic(conn, || ingest(conn, args)),
        _ => {}
    }
    if ![
        "commitment_register",
        "commitment_apply",
        "commitment_check",
        "commitment_sync",
    ]
    .contains(&name)
    {
        bail!("unknown commitment tool: {name}");
    }
    let op = required(args, "op_id")?;
    let payload = json!({"tool":name,"args":args}).to_string();
    atomic(conn, || {
        let old: Option<(String, String)> = conn
            .query_row(
                "SELECT payload,result FROM commitment_operations WHERE op_id=?1",
                [op],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((p, r)) = old {
            if p != payload {
                bail!("operation identity conflict: op_id reused with different input");
            }
            return Ok(serde_json::from_str(&r)?);
        }
        let result = match name {
            "commitment_register" => {
                strict(
                    args,
                    &[
                        "op_id",
                        "request",
                        "counterparty",
                        "reply_due",
                        "next_check",
                        "source",
                        "source_ref",
                        "evidence",
                        "related_task_ids",
                        "blocked_task_ids",
                    ],
                )?;
                let after = create(conn, args)?;
                persist(
                    conn,
                    None,
                    &after,
                    None,
                    "register",
                    required(args, "evidence")?,
                )?;
                json!({"commitment":presented(conn,after)?})
            }
            "commitment_apply" => apply(conn, args)?,
            "commitment_check" => check(conn, args)?,
            "commitment_sync" => sync(conn, args)?,
            _ => unreachable!(),
        };
        conn.execute(
            "INSERT INTO commitment_operations(op_id,payload,result) VALUES(?1,?2,?3)",
            params![op, payload, result.to_string()],
        )?;
        Ok(result)
    })
}
pub fn due_questions(store: &Store, at: &str) -> Result<Vec<Value>> {
    let at = timestamp(at)?;
    let conn = store.connection();
    let mut questions = vec![];
    for update in pending(conn)? {
        let target = update["commitment_id"].as_str();
        let current = target.map(|id| get(conn, id)).transpose()?;
        let rev = current.as_ref().and_then(|c| c["revision"].as_i64());
        let new = update["kind"] == "new_request";
        let mut args = json!({"op_id":uid(),"update_id":update["id"],"action":"apply"});
        let mut needs = vec!["confirmed"];
        if new {
            args["expected_revision"] = json!(0);
        } else if let Some(rev) = rev {
            args["commitment_id"] = json!(target);
            args["expected_revision"] = json!(rev);
        } else {
            needs.extend(["commitment_id", "expected_revision"]);
        }
        let mut reject = args.clone();
        reject["op_id"] = json!(uid());
        reject["action"] = json!("reject");
        let mut reject_needs = needs.clone();
        if target.is_none() && !new {
            reject["expected_revision"] = json!(0);
            reject_needs = vec!["confirmed"];
        }
        reject_needs.push("evidence");
        questions.push(json!({"kind":"commitment_update","target_id":update["id"],"title":current.as_ref().map(|c|c["request"].clone()).unwrap_or_else(||update.get("request").cloned().unwrap_or(json!("依頼への変更"))),
            "question":format!("{} の {} を確認してください。依頼元: {} / 現在: {} / 変更案: {} / 対応候補(自動統合なし): {}",update["source"],update["kind"],update["source_ref"],current.as_ref().unwrap_or(&Value::Null),update,if target.is_none()&&!new{json!(all(conn)?.into_iter().filter(|v|v["status"]=="waiting"||v["status"]=="replied").map(|v|json!({"id":v["id"],"revision":v["revision"],"request":v["request"],"source_ref":v["source_ref"]})).collect::<Vec<_>>())}else{json!([])}),"default":0,
            "options":[{"label":"後で確認する","tool":null,"args":{},"needs":[]},{"label":"変更を確認して適用","tool":"commitment_apply","args":args,"needs":needs},{"label":"適用せず記録","tool":"commitment_apply","args":reject,"needs":reject_needs}]}));
    }
    for v in all(conn)? {
        if !["waiting", "replied"].contains(&v["status"].as_str().unwrap_or("")) {
            continue;
        }
        if v["next_check"].as_str().is_none_or(|s| s > at.as_str()) {
            continue;
        }
        let args = json!({"op_id":uid(),"commitment_id":v["id"],"expected_revision":v["revision"],"action":"defer"});
        let mut settle = args.clone();
        settle["op_id"] = json!(uid());
        settle["action"] = json!("settle");
        questions.push(json!({"kind":"commitment_check","target_id":v["id"],"title":v["request"],"default":0,
            "question":format!("{}: {} / 状態 {} / 確認 {} / 依頼元 {} {} / 根拠 {} / 最新の変更と根拠 {} / 返答 {}",v["counterparty"],v["request"],v["status"],v["next_check"],v["source"],v["source_ref"],v["evidence"],v["last_update"],v["reply"]),
            "options":[{"label":"後で確認する","tool":null,"args":{},"needs":[]},{"label":"確認して次の日時を指定","tool":"commitment_check","args":args,"needs":["confirmed","evidence","next_check"]},{"label":"根拠を確認して決着","tool":"commitment_check","args":settle,"needs":["confirmed","evidence"]}]}));
    }
    Ok(questions)
}
pub fn due_count(store: &Store, at: &str) -> Result<usize> {
    Ok(due_questions(store, at)?.len())
}
