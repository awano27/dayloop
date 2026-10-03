//! Isolated durable-ledger invariants; no Outlook, Teams, Jev, or user data.
use dayloop::{
    commitment::{self, dispatch},
    model::State,
    store::Store,
};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Mutex};
static LOCK: Mutex<()> = Mutex::new(());
struct Home {
    dir: PathBuf,
    old: Option<String>,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl Home {
    fn new() -> Self {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-commitment-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("DAYLOOP_HOME", &dir);
        Self {
            dir,
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
            Some(v) => std::env::set_var("DAYLOOP_HOME", v),
            None => std::env::remove_var("DAYLOOP_HOME"),
        };
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn register(store: &Store, op: &str, extra: Value) -> Value {
    let mut args = json!({"op_id":op,"request":"レビュー依頼","counterparty":"担当者","reply_due":"2099-01-05T12:00:00+09:00","next_check":"2099-01-05T12:00:00+09:00","source":"manual","source_ref":"request-A","evidence":"依頼原文"});
    for (k, v) in extra.as_object().unwrap() {
        args[k] = v.clone();
    }
    dispatch(store, "commitment_register", &args).unwrap()["commitment"].clone()
}
fn ingest(store: &Store, id: &str, event: &str, kind: &str, when: &str, extra: Value) -> Value {
    let mut args = json!({"source":"fixture","external_id":event,"kind":kind,"occurred_at":when,"commitment_id":id,"source_ref":format!("sample/{event}"),"evidence":format!("原文 {event}")});
    for (k, v) in extra.as_object().unwrap() {
        args[k] = v.clone();
    }
    dispatch(store, "commitment_ingest", &args).unwrap()["update"].clone()
}
fn apply(store: &Store, op: &str, update: &Value, revision: i64) -> Value {
    dispatch(
        store,
        "commitment_apply",
        &json!({"op_id":op,"update_id":update["id"],"expected_revision":revision,"confirmed":true}),
    )
    .unwrap()["commitment"]
        .clone()
}

#[test]
fn past_daily_cutoff_includes_afternoon_and_excludes_next_midnight() {
    let home = Home::new();
    let store = home.store();
    let afternoon = register(&store, "past-afternoon", json!({"next_check":"2020-01-06T15:00:00+09:00"}));
    register(&store, "next-midnight", json!({"next_check":"2020-01-07T00:00:00+09:00"}));
    let at = commitment::day_cutoff("2020-01-06").unwrap();
    let questions = commitment::due_questions(&store, &at).unwrap();
    assert_eq!(questions.len(), 1, "past day must include its afternoon only: {at}");
    assert_eq!(questions[0]["target_id"], afternoon["id"]);
}

#[test]
fn future_daily_cutoff_does_not_advance_the_real_clock() {
    let before = chrono::Utc::now();
    let at = chrono::DateTime::parse_from_rfc3339(&commitment::day_cutoff("2100-01-01").unwrap()).unwrap().with_timezone(&chrono::Utc);
    let after = chrono::Utc::now();
    assert!(before <= at && at <= after, "future day must use the actual instant: {at}");
}

#[test]
fn todays_daily_cutoff_is_due_exactly_at_the_confirmation_instant() {
    let home = Home::new();
    let store = home.store();
    let c = register(&store, "today-boundary", json!({"next_check":"2026-10-03T15:00:00+09:00"}));
    for (instant, expected_count) in [
        ("2026-10-03T05:59:59.999999999Z", 0),
        ("2026-10-03T06:00:00Z", 1),
        ("2026-10-03T06:00:00.000000001Z", 1),
    ] {
        let at = chrono::DateTime::parse_from_rfc3339(instant).unwrap().with_timezone(&chrono::Utc);
        let cutoff = commitment::day_cutoff_at("2026-10-03", at).unwrap();
        assert_eq!(chrono::DateTime::parse_from_rfc3339(&cutoff).unwrap().with_timezone(&chrono::Utc), at);
        let questions = commitment::due_questions(&store, &cutoff).unwrap();
        assert_eq!(questions.len(), expected_count, "instant={instant}");
        if expected_count == 1 {
            assert_eq!(questions[0]["target_id"], c["id"]);
        }
        // Looking at tomorrow must not bring tomorrow's clock forward either.
        let future = commitment::day_cutoff_at("2026-10-04", at).unwrap();
        assert_eq!(future, cutoff);
    }
}

#[test]
fn past_jst_day_includes_its_final_nanosecond_only() {
    let home = Home::new();
    let store = home.store();
    let last = register(&store, "last-nanosecond", json!({"next_check":"2026-10-03T23:59:59.999999999+09:00"}));
    register(&store, "next-midnight", json!({"next_check":"2026-10-04T00:00:00+09:00"}));
    let at = chrono::DateTime::parse_from_rfc3339("2026-10-03T15:00:00Z").unwrap().with_timezone(&chrono::Utc);
    let cutoff = commitment::day_cutoff_at("2026-10-03", at).unwrap();
    assert_eq!(cutoff, "2026-10-03T14:59:59.999999999Z");
    let questions = commitment::due_questions(&store, &cutoff).unwrap();
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0]["target_id"], last["id"]);
    assert!(commitment::day_cutoff_at("invalid", at).is_err());
}

#[test]
fn ledger_plan_confirmation_refuses_unanswered_commitments() {
    let home = Home::new();
    let store = home.store();
    register(&store, "pending-plan", json!({"next_check":"2020-01-06T00:00:00+09:00"}));
    let revision = store.plan_revision("2020-01-06").unwrap();
    assert!(store.confirm_plan("2020-01-06", Some(&revision)).is_err(), "ledger must enforce confirmation prerequisites itself");
    assert!(store.get_day("2020-01-06").unwrap().is_none());
}

#[test]
fn ledger_plan_confirmation_refuses_unanswered_source_proposals() {
    let home = Home::new();
    let store = home.store();
    let c = register(&store, "future-plan", json!({}));
    ingest(&store, c["id"].as_str().unwrap(), "unapproved-deadline", "deadline_change", "2026-01-01T00:00:00Z", json!({"next_check":"2100-01-01T09:00:00+09:00"}));
    let revision = store.plan_revision("2020-01-06").unwrap();
    assert!(store.confirm_plan("2020-01-06", Some(&revision)).is_err(), "pending proposal must block confirmation even with a future wait");
    assert!(store.get_day("2020-01-06").unwrap().is_none());
}
#[test]
fn lifecycle_persists_closure_restart_and_does_not_complete_work() {
    let home = Home::new();
    let store = home.store();
    let day = "2020-01-06";
    let work = store
        .add_task("レビューに依存する仕事", None, None, "manual", None, None)
        .unwrap();
    let registered = register(
        &store,
        "register",
        json!({"related_task_ids":[work.id],"blocked_task_ids":[work.id]}),
    );
    let id = registered["id"].as_str().unwrap();
    assert!(commitment::blocked_task_ids(&store)
        .unwrap()
        .contains(&work.id));
    assert!(
        commitment::due_questions(&store, "2099-01-05T11:59:59+09:00")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        commitment::due_count(&store, "2099-01-05T12:00:00+09:00").unwrap(),
        1
    );
    let change = ingest(
        &store,
        id,
        "deadline",
        "deadline_change",
        "2020-01-07T09:00:00Z",
        json!({"reply_due":"2099-01-08T12:00:00+09:00","next_check":"2099-01-08T12:00:00+09:00"}),
    );
    // Even future promises surface incoming proposals, but never auto-apply.
    assert_eq!(
        commitment::due_count(&store, "2020-01-07T10:00:00Z").unwrap(),
        1
    );
    let changed = apply(&store, "apply-deadline", &change, 1);
    assert_eq!(changed["revision"], 2);
    assert!(commitment::due_questions(&store, "2099-01-08T02:59:59Z")
        .unwrap()
        .is_empty());
    let q = commitment::due_questions(&store, "2099-01-08T03:00:00Z").unwrap();
    assert_eq!(q.len(), 1);
    assert!(q[0]["options"][0]["tool"].is_null());
    store.close_day(day).unwrap().unwrap();
    let closed = store.get_day(day).unwrap().unwrap().closed_at;
    drop(store);
    let store = home.store();
    let reopened = dispatch(
        &store,
        "commitment_list",
        &json!({"at":"2099-01-07T00:00:00Z"}),
    )
    .unwrap();
    assert_eq!(reopened["commitments"][0]["id"], id);
    assert_eq!(
        reopened["commitments"][0]["next_check"],
        changed["next_check"]
    );
    let reply = ingest(
        &store,
        id,
        "reply",
        "reply",
        "2020-01-08T09:00:00Z",
        json!({"request":"修正箇所をご確認ください"}),
    );
    let replied = apply(&store, "apply-reply", &reply, 2);
    assert_eq!(replied["status"], "replied");
    assert_eq!(store.get_task(&work.id).unwrap().state, State::Backlog);
    assert!(commitment::blocked_task_ids(&store).unwrap().is_empty());
    let next = store
        .add_task("修正を反映する", None, None, "manual", None, None)
        .unwrap();
    let checked=dispatch(&store,"commitment_check",&json!({"op_id":"review-reply","commitment_id":id,"expected_revision":3,"confirmed":true,"action":"acknowledge","evidence":"返答の内容を確認","next_check":"2099-01-09T12:00:00+09:00","next_task_id":next.id})).unwrap();
    assert_eq!(checked["commitment"]["status"], "replied");
    assert_eq!(store.get_task(&next.id).unwrap().state, State::Backlog);
    let settled=dispatch(&store,"commitment_check",&json!({"op_id":"settle","commitment_id":id,"expected_revision":4,"confirmed":true,"action":"settle","evidence":"修正完了と相手の了承"})).unwrap();
    assert_eq!(settled["commitment"]["status"], "settled");
    assert_eq!(store.get_day(day).unwrap().unwrap().closed_at, closed);
    let history = dispatch(&store, "commitment_history", &json!({"commitment_id":id})).unwrap();
    assert_eq!(history["decisions"].as_array().unwrap().len(), 5);
    assert_eq!(
        history["decisions"][1]["before"]["next_check"],
        registered["next_check"]
    );
    assert_eq!(
        history["decisions"][1]["after"]["next_check"],
        changed["next_check"]
    );
}
#[test]
fn source_and_operation_identities_are_idempotent_and_conflict_checked() {
    let home = Home::new();
    let store = home.store();
    let first = register(&store, "reg", json!({}));
    let again = register(&store, "reg", json!({}));
    assert_eq!(first, again);
    let conflict = json!({"op_id":"reg","request":"別の仕事","counterparty":"担当者","next_check":"2099-01-01T00:00:00Z","source":"manual","evidence":"原文"});
    assert!(dispatch(&store, "commitment_register", &conflict).is_err());
    let id = first["id"].as_str().unwrap();
    let a = ingest(
        &store,
        id,
        "event",
        "deadline_change",
        "2020-01-01T09:00:00Z",
        json!({"next_check":"2099-02-01T09:00:00Z"}),
    );
    let b = ingest(
        &store,
        id,
        "event",
        "deadline_change",
        "2020-01-01T09:00:00Z",
        json!({"next_check":"2099-02-01T09:00:00Z"}),
    );
    assert_eq!(a, b);
    let args = json!({"op_id":"apply","update_id":a["id"],"expected_revision":1,"confirmed":true});
    let applied = dispatch(&store, "commitment_apply", &args).unwrap();
    assert_eq!(
        applied,
        dispatch(&store, "commitment_apply", &args).unwrap()
    );
    let conflict = json!({"source":"fixture","external_id":"event","kind":"deadline_change","occurred_at":"2020-01-01T09:00:00Z","commitment_id":id,"next_check":"2099-03-01T09:00:00Z","evidence":"違う原文"});
    assert!(dispatch(&store, "commitment_ingest", &conflict).is_err());
    assert_eq!(
        dispatch(&store, "commitment_history", &json!({"commitment_id":id})).unwrap()["decisions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn stale_question_and_old_source_update_are_rejected_atomically() {
    let home = Home::new();
    let store = home.store();
    let first = register(&store, "reg", json!({"next_check":"2020-01-01T00:00:00Z"}));
    let id = first["id"].as_str().unwrap();
    let stale = commitment::due_questions(&store, "2020-01-01T00:00:00Z").unwrap()[0]["options"][2]
        ["args"]
        .clone();
    let newer = ingest(
        &store,
        id,
        "new",
        "deadline_change",
        "2020-01-03T00:00:00Z",
        json!({"next_check":"2099-02-01T00:00:00Z"}),
    );
    apply(&store, "new", &newer, 1);
    let mut stale = stale;
    stale["confirmed"] = json!(true);
    stale["evidence"] = json!("古い画面");
    assert!(dispatch(&store, "commitment_check", &stale)
        .unwrap_err()
        .to_string()
        .contains("stale"));
    let old = ingest(
        &store,
        id,
        "old",
        "deadline_change",
        "2020-01-02T00:00:00Z",
        json!({"next_check":"2099-01-01T00:00:00Z"}),
    );
    let args = json!({"op_id":"old","update_id":old["id"],"expected_revision":2,"confirmed":true});
    assert!(dispatch(&store, "commitment_apply", &args)
        .unwrap_err()
        .to_string()
        .contains("old"));
    let mut reject = args;
    reject["action"] = json!("reject");
    reject["evidence"] = json!("既に新しい期限で確定している");
    let rejected = dispatch(&store, "commitment_apply", &reject).unwrap();
    assert_eq!(
        rejected["commitment"]["next_check"],
        "2099-02-01T00:00:00.000000000Z"
    );
    assert_eq!(rejected["commitment"]["revision"], 3);
}
#[test]
fn older_source_cannot_replace_manual_confirmation_schedule() {
    let home = Home::new();
    let store = home.store();
    let c = register(&store, "reg", json!({}));
    let id = c["id"].as_str().unwrap();
    dispatch(&store,"commitment_check",&json!({"op_id":"defer","commitment_id":id,"expected_revision":1,"confirmed":true,"action":"defer","evidence":"最新の判断","next_check":"2099-03-01T00:00:00Z"})).unwrap();
    let old = ingest(
        &store,
        id,
        "old",
        "deadline_change",
        "2020-01-01T00:00:00Z",
        json!({"next_check":"2099-01-01T00:00:00Z"}),
    );
    assert!(dispatch(
        &store,
        "commitment_apply",
        &json!({"op_id":"old","update_id":old["id"],"expected_revision":2,"confirmed":true})
    )
    .unwrap_err()
    .to_string()
    .contains("manual"));
    assert_eq!(
        dispatch(&store, "commitment_list", &json!({})).unwrap()["commitments"][0]["next_check"],
        "2099-03-01T00:00:00.000000000Z"
    );
}
#[test]
fn equal_titles_remain_distinct_and_unlinked_updates_need_manual_selection() {
    let home = Home::new();
    let store = home.store();
    let a = register(&store, "A", json!({"source_ref":"A"}));
    let b = register(&store, "B", json!({"source_ref":"B"}));
    assert_ne!(a["id"], b["id"]);
    let update=dispatch(&store,"commitment_ingest",&json!({"source":"fixture","external_id":"unlinked","kind":"reply","request":"レビュー依頼","occurred_at":"2020-01-01T00:00:00Z","evidence":"返答"})).unwrap();
    assert_eq!(update["candidates"].as_array().unwrap().len(), 2);
    let args = json!({"op_id":"link","update_id":update["update"]["id"],"expected_revision":1,"confirmed":true});
    assert!(dispatch(&store, "commitment_apply", &args)
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    let mut chosen = args;
    chosen["commitment_id"] = a["id"].clone();
    dispatch(&store, "commitment_apply", &chosen).unwrap();
    let list = dispatch(&store, "commitment_list", &json!({})).unwrap();
    let commitments = list["commitments"].as_array().unwrap();
    assert_eq!(
        commitments.iter().find(|v| v["id"] == b["id"]).unwrap()["status"],
        "waiting"
    );
}
#[test]
fn carry_descendants_preserve_promise_identity_without_task_writes() {
    let home = Home::new();
    let store = home.store();
    let task = store
        .add_task(
            "依存する仕事",
            None,
            None,
            "manual",
            None,
            Some("2020-01-01"),
        )
        .unwrap();
    let c = register(
        &store,
        "reg",
        json!({"related_task_ids":[task.id],"blocked_task_ids":[task.id]}),
    );
    let carried = store
        .carry_over(&task.id, "相手待ち", "2020-01-02", None)
        .unwrap();
    let list = dispatch(&store, "commitment_list", &json!({})).unwrap();
    assert_eq!(list["commitments"][0]["id"], c["id"]);
    assert!(list["commitments"][0]["related_task_ids"]
        .as_array()
        .unwrap()
        .contains(&json!(carried.id)));
    assert!(commitment::blocked_task_ids(&store)
        .unwrap()
        .contains(&carried.id));
    assert_eq!(store.get_task(&task.id).unwrap().state, State::Carried);
}
#[test]
fn unavailable_and_partial_sync_never_mean_absence_or_erase_due_work() {
    let home = Home::new();
    let store = home.store();
    register(&store, "reg", json!({"next_check":"2020-01-01T00:00:00Z"}));
    for status in ["unavailable", "partial", "failed"] {
        dispatch(&store,"commitment_sync",&json!({"op_id":status,"source":"fixture","status":status,"at":"2020-01-02T00:00:00Z","evidence":"取得できた範囲は不明"})).unwrap();
    }
    let list = dispatch(
        &store,
        "commitment_list",
        &json!({"at":"2020-01-02T00:00:00Z"}),
    )
    .unwrap();
    assert_eq!(list["commitments"][0]["status"], "waiting");
    assert_eq!(list["questions"].as_array().unwrap().len(), 1);
    assert_eq!(list["sync"].as_array().unwrap().len(), 3);
}
#[test]
fn new_request_and_cancellation_are_explicit_and_input_validation_is_strict() {
    let home = Home::new();
    let store = home.store();
    let args = json!({"source":"fixture","external_id":"new","kind":"new_request","occurred_at":"2020-01-01T00:00:00Z","request":"レビュー","counterparty":"同僚","next_check":"2099-01-01T09:00:00+09:00","evidence":"依頼"});
    let update = dispatch(&store, "commitment_ingest", &args).unwrap();
    assert!(
        dispatch(&store, "commitment_list", &json!({})).unwrap()["commitments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut apply_args =
        json!({"op_id":"apply","update_id":update["update"]["id"],"expected_revision":0});
    assert!(dispatch(&store, "commitment_apply", &apply_args).is_err());
    apply_args["confirmed"] = json!(true);
    let c = dispatch(&store, "commitment_apply", &apply_args).unwrap()["commitment"].clone();
    assert_eq!(c["next_check"], "2099-01-01T00:00:00.000000000Z");
    let cancel = ingest(
        &store,
        c["id"].as_str().unwrap(),
        "cancel",
        "cancellation",
        "2020-01-02T00:00:00Z",
        json!({}),
    );
    assert_eq!(
        dispatch(&store, "commitment_list", &json!({})).unwrap()["commitments"][0]["status"],
        "waiting"
    );
    let cancelled = apply(&store, "cancel", &cancel, 1);
    assert_eq!(cancelled["status"], "cancelled");
    assert!(cancelled["next_check"].is_null());
    let mut invalid = args.clone();
    invalid["external_id"] = json!("invalid");
    invalid["next_check"] = json!("2099-01-01");
    assert!(dispatch(&store, "commitment_ingest", &invalid).is_err());
    invalid["next_check"] = Value::Null;
    assert!(dispatch(&store, "commitment_ingest", &invalid).is_err());
    invalid["next_check"] = json!("2099-01-01T00:00:00Z");
    invalid["surprise"] = json!(true);
    assert!(dispatch(&store, "commitment_ingest", &invalid).is_err());
}
#[test]
fn populated_legacy_migration_is_additive_and_repeatable() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY,title TEXT);INSERT INTO tasks VALUES('old','既存仕事');CREATE TABLE days(date TEXT PRIMARY KEY,closed_at TEXT);INSERT INTO days VALUES('2020-01-01','closed');").unwrap();
    commitment::migrate(&conn).unwrap();
    commitment::migrate(&conn).unwrap();
    assert_eq!(
        conn.query_row("SELECT title FROM tasks WHERE id='old'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "既存仕事"
    );
    assert_eq!(
        conn.query_row("SELECT closed_at FROM days", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "closed"
    );
}
#[test]
fn failing_migration_rolls_back_all_new_objects_and_preserves_legacy() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE tasks(id TEXT PRIMARY KEY,title TEXT);INSERT INTO tasks VALUES('old','既存仕事');CREATE VIEW commitment_updates AS SELECT 1 AS id;").unwrap();
    assert!(commitment::migrate(&conn).is_err());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='commitments'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT title FROM tasks", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "既存仕事"
    );
}

#[test]
fn source_events_decisions_and_sync_history_cannot_be_rewritten() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    commitment::migrate(&conn).unwrap();
    conn.execute_batch("INSERT INTO commitment_updates VALUES('event','fixture','external',NULL,'2020-01-01T00:00:00Z','{}'); INSERT INTO commitment_decisions VALUES('decision','promise','event','reply',NULL,'{}','verified','2020-01-02T00:00:00Z'); INSERT INTO commitment_syncs VALUES('sync','fixture','failed','2020-01-01T00:00:00Z','unavailable'); INSERT INTO commitment_rejections VALUES('other-event','no matching request','2020-01-02T00:00:00Z');").unwrap();
    for table in [
        "commitment_updates",
        "commitment_decisions",
        "commitment_syncs",
        "commitment_rejections",
    ] {
        assert!(conn.execute(&format!("DELETE FROM {table}"), []).is_err());
        let id_column = if table == "commitment_rejections" {
            "update_id"
        } else {
            "id"
        };
        assert!(conn
            .execute(&format!("UPDATE {table} SET {id_column}='replacement'"), [])
            .is_err());
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn unlinked_no_match_rejection_is_auditable_and_does_not_create_a_promise() {
    let home = Home::new();
    let store = home.store();
    let input = json!({"source":"fixture","external_id":"unknown-change","kind":"deadline_change","occurred_at":"2020-01-01T00:00:00Z","next_check":"2099-01-01T00:00:00Z","evidence":"対象の依頼が存在しない変更"});
    let update = dispatch(&store, "commitment_ingest", &input).unwrap();
    let questions = commitment::due_questions(&store, "2020-01-02T00:00:00Z").unwrap();
    assert_eq!(questions.len(), 1);
    let mut reject = questions[0]["options"][2]["args"].clone();
    assert_eq!(reject["expected_revision"], 0);
    assert!(reject.get("commitment_id").is_none());
    reject["confirmed"] = json!(true);
    reject["evidence"] = json!("本人が追跡対象なしと確認");
    let rejected = dispatch(&store, "commitment_apply", &reject).unwrap();
    assert_eq!(rejected["update"]["status"], "rejected");
    assert_eq!(rejected["update"]["id"], update["update"]["id"]);
    assert_eq!(
        rejected,
        dispatch(&store, "commitment_apply", &reject).unwrap()
    );
    assert_eq!(
        commitment::due_count(&store, "2020-01-02T00:00:00Z").unwrap(),
        0
    );
    let list = dispatch(&store, "commitment_list", &json!({})).unwrap();
    assert!(list["commitments"].as_array().unwrap().is_empty());
    assert_eq!(list["rejected_updates"].as_array().unwrap().len(), 1);
    assert_eq!(
        dispatch(&store, "commitment_ingest", &input).unwrap()["update"]["status"],
        "rejected"
    );
}

#[test]
fn old_reply_without_explicit_schedule_preserves_newer_manual_schedule() {
    let home = Home::new();
    let store = home.store();
    let c = register(&store, "reg", json!({}));
    let id = c["id"].as_str().unwrap();
    dispatch(&store,"commitment_check",&json!({"op_id":"defer","commitment_id":id,"expected_revision":1,"confirmed":true,"action":"defer","evidence":"本人が最新の確認日時を指定","next_check":"2099-03-01T00:00:00Z"})).unwrap();
    let reply = ingest(
        &store,
        id,
        "historic-reply",
        "reply",
        "2020-01-01T00:00:00Z",
        json!({"request":"以前の返答の内容"}),
    );
    let c = apply(&store, "historic-reply", &reply, 2);
    assert_eq!(c["status"], "replied");
    assert_eq!(c["next_check"], "2099-03-01T00:00:00.000000000Z");
}
