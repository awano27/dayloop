//! Isolated process acceptance tests: every CLI/MCP call starts a fresh process.
//! No parent environment mutation, live adapter, AI call, or real user ledger.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dayloop-commitment-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("config.toml"),
            "[jev]\nmode = \"off\"\n[intake]\noutlook = false\n[observe]\nfixture = \"\"\n",
        )
        .unwrap();
        Self(path)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dayloop"));
        command
            .env("DAYLOOP_HOME", &self.0)
            // `next` also visits existing intake adapters. Remove child-only
            // credentials and CLI discovery so parent logins cannot be used.
            .env("PATH", "")
            .stdin(Stdio::null());
        for key in [
            "TYPESAFE_API_KEY",
            "DAYLOOP_JEV_API_KEY",
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "JIRA_BASE_URL",
            "JIRA_EMAIL",
            "JIRA_API_TOKEN",
            "TEAMS_TOKEN",
            "GRAPH_TOKEN",
            "AZURE_DEVOPS_ORG",
            "AZURE_DEVOPS_PAT",
            "ADO_PAT",
            "DAYLOOP_LINK_LIVE",
        ] {
            command.env_remove(key);
        }
        command
    }

    fn cli(&self, action: &str, args: &Value) -> Value {
        let output = self
            .command()
            .args(["commitment", action, "--json", &args.to_string()])
            .output()
            .unwrap();
        let body: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "CLI {action}: {error}; status={}; stdout={}; stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        assert_eq!(
            output.status.success(),
            body.get("error").is_none(),
            "{body}"
        );
        body
    }

    fn tool(&self, name: &str, args: &Value) -> Value {
        let mut child = self
            .command()
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        writeln!(
            input,
            "{}",
            json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{"name":name, "arguments":args}
            })
        )
        .unwrap();
        drop(input);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rpc: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "MCP {name}: {error}; {}",
                String::from_utf8_lossy(&output.stdout)
            )
        });
        let body: Value =
            serde_json::from_str(rpc["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            rpc["result"]["isError"],
            body.get("error").is_some(),
            "{rpc}"
        );
        body
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        // This unique directory was created by this test and never supplied by the user.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ok(body: Value) -> Value {
    assert!(body.get("error").is_none(), "{body}");
    body
}

fn register(op_id: &str, request: &str, next_check: &str) -> Value {
    json!({
        "op_id":op_id, "request":request, "counterparty":"sample-reviewer-A",
        "reply_due":"2099-01-07T09:00:00+09:00", "next_check":next_check,
        "source":"manual", "evidence":"fixture:review-request"
    })
}

fn apply(op_id: &str, update: &Value, commitment: &Value) -> Value {
    json!({
        "op_id":op_id, "update_id":update["id"], "commitment_id":commitment["id"],
        "expected_revision":commitment["revision"], "confirmed":true
    })
}

fn ingest(external_id: &str, kind: &str, commitment: &Value, occurred_at: &str) -> Value {
    json!({
        "source":"fixture", "external_id":external_id, "kind":kind,
        "occurred_at":occurred_at, "commitment_id":commitment["id"],
        "evidence":format!("fixture:{external_id}")
    })
}

#[test]
fn cli_and_mcp_share_registration_replay_and_input_validation() {
    let home = Home::new();
    let input = register("register-1", "Design review", "2099-01-07T09:00:00+09:00");
    let commitment = ok(home.cli("register", &input));
    assert_eq!(commitment, home.tool("commitment_register", &input));
    assert_eq!(commitment["commitment"]["request"], "Design review");

    let mut invalid = register("invalid-time", "Invalid time", "tomorrow morning");
    let cli_error = home.cli("register", &invalid);
    assert!(cli_error.get("error").is_some(), "{cli_error}");
    assert_eq!(cli_error, home.tool("commitment_register", &invalid));

    invalid["op_id"] = json!("invalid-empty");
    invalid["next_check"] = json!("2099-01-07T09:00:00+09:00");
    invalid["request"] = json!("  ");
    let cli_error = home.cli("register", &invalid);
    assert!(cli_error.get("error").is_some(), "{cli_error}");
    assert_eq!(cli_error, home.tool("commitment_register", &invalid));

    let list_args = json!({"at":"2026-10-03T00:00:00Z"});
    assert_eq!(
        home.cli("list", &list_args),
        home.tool("commitment_list", &list_args)
    );
}

#[test]
fn review_change_reply_next_work_and_settlement_survive_process_restarts() {
    let home = Home::new();
    let early_day = json!({"date":"2026-09-30"});
    let mut commitment = ok(home.cli(
        "register",
        &register("review", "Design review", "2026-10-01T00:00:00+09:00"),
    ))["commitment"]
        .clone();
    let stable_id = commitment["id"].clone();
    let early = ok(home.tool("check_in", &early_day));
    assert!(early["questions"].as_array().unwrap().is_empty(), "{early}");
    assert_eq!(ok(home.tool("close_day", &early_day))["closed"], true);
    let closed_snapshot = ok(home.tool("get_today", &early_day));

    let mut change = ingest(
        "deadline-1",
        "deadline_change",
        &commitment,
        "2026-10-03T01:00:00Z",
    );
    change["reply_due"] = json!("2026-10-02T00:00:00+09:00");
    change["next_check"] = json!("2026-10-02T00:00:00+09:00");
    let proposal = ok(home.tool("commitment_ingest", &change));
    assert_eq!(
        proposal,
        ok(home.cli("ingest", &change)),
        "same pending event must deduplicate"
    );
    assert_eq!(proposal["update"]["status"], "pending");
    let apply_args = apply("approve-deadline-1", &proposal["update"], &commitment);
    let applied = ok(home.cli("apply", &apply_args));
    assert_eq!(
        applied,
        home.tool("commitment_apply", &apply_args),
        "operation replay must be exact"
    );
    commitment = applied["commitment"].clone();
    assert_eq!(commitment["id"], stable_id);
    assert_eq!(commitment["status"], "waiting");
    let early = ok(home.tool("check_in", &json!({"date":"2026-10-01"})));
    assert!(early["questions"].as_array().unwrap().is_empty(), "{early}");

    let due = ok(home.tool("check_in", &json!({"date":"2026-10-02"})));
    let due_questions = due["questions"].as_array().unwrap();
    assert!(
        due_questions.iter().any(|q| q["target_id"] == stable_id),
        "{due}"
    );
    let defer = json!({
        "op_id":"checked-review", "commitment_id":stable_id,
        "expected_revision":commitment["revision"], "confirmed":true,
        "action":"defer", "next_check":"2099-01-11T09:00:00+09:00",
        "evidence":"fixture:reviewer-follow-up-confirmed"
    });
    commitment = ok(home.cli("check", &defer))["commitment"].clone();
    assert!(
        ok(home.tool("check_in", &json!({"date":"2026-10-02"})))["questions"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let mut reply = ingest("reply-1", "reply", &commitment, "2026-10-03T02:00:00Z");
    reply["request"] = json!("Review accepted after correcting the API contract");
    let proposal = ok(home.cli("ingest", &reply));
    commitment = ok(home.tool(
        "commitment_apply",
        &apply("approve-reply", &proposal["update"], &commitment),
    ))["commitment"]
        .clone();
    assert_eq!(commitment["status"], "replied");
    assert_eq!(commitment["id"], stable_id);
    let task = ok(home.tool(
        "add_task",
        &json!({"title":"Apply review corrections", "date":"2026-10-02"}),
    ));
    let acknowledge = json!({
        "op_id":"reply-reviewed", "commitment_id":stable_id,
        "expected_revision":commitment["revision"], "confirmed":true,
        "action":"acknowledge", "next_check":"2099-01-12T09:00:00+09:00",
        "next_task_id":task["id"], "evidence":"fixture:reply-read-and-next-step-agreed"
    });
    commitment = ok(home.cli("check", &acknowledge))["commitment"].clone();
    assert!(commitment["related_task_ids"]
        .as_array()
        .unwrap()
        .contains(&task["id"]));
    let today = ok(home.tool("get_today", &json!({"date":"2026-10-02"})));
    assert_eq!(
        today["tasks"][0]["state"], "planned",
        "reply must not finish linked work"
    );
    ok(home.tool(
        "finish_task",
        &json!({"id":task["id"],"evidence":"fixture:corrections-agreed"}),
    ));
    let settle = json!({
        "op_id":"review-settled", "commitment_id":stable_id,
        "expected_revision":commitment["revision"], "confirmed":true,
        "action":"settle", "evidence":"fixture:corrected-contract-and-final-agreement"
    });
    let settled = ok(home.cli("check", &settle));
    assert_eq!(settled, home.tool("commitment_check", &settle));
    assert_eq!(settled["commitment"]["status"], "settled");
    assert!(settled["commitment"]["next_check"].is_null());
    let history = ok(home.cli("history", &json!({"commitment_id":stable_id})));
    assert_eq!(history["decisions"].as_array().unwrap().len(), 6);
    assert_eq!(history["updates"].as_array().unwrap().len(), 2);
    assert_eq!(
        history,
        home.tool("commitment_history", &json!({"commitment_id":stable_id}))
    );
    let reopened = ok(home.tool("get_today", &early_day));
    assert_eq!(
        reopened["day"], closed_snapshot["day"],
        "closed day timestamp must be immutable"
    );
    assert_eq!(reopened["tasks"], closed_snapshot["tasks"]);
    assert_eq!(reopened["events"], closed_snapshot["events"]);
    assert_eq!(ok(home.tool("close_day", &early_day))["closed"], true);
}

#[test]
fn due_confirmation_blocks_close_and_stale_question_cannot_apply() {
    let home = Home::new();
    let day = json!({"date":"2026-10-02"});
    let commitment = ok(home.cli(
        "register",
        &register("due-review", "Due review", "2026-10-02T00:00:00+09:00"),
    ))["commitment"]
        .clone();
    let id = commitment["id"].clone();
    let before = ok(home.cli("list", &json!({"at":"2026-10-01T14:59:59Z"})));
    assert!(before["questions"].as_array().unwrap().is_empty());
    let at = ok(home.tool("commitment_list", &json!({"at":"2026-10-01T15:00:00Z"})));
    assert!(at["questions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|q| q["target_id"] == id));
    let closed = ok(home.tool("close_day", &day));
    assert_eq!(closed["closed"], false);
    let stale_action = json!({
        "op_id":"outdated-question", "commitment_id":id,
        "expected_revision":commitment["revision"], "confirmed":true,
        "action":"settle", "evidence":"fixture:old-screen"
    });
    let defer = json!({
        "op_id":"newer-confirmation", "commitment_id":id,
        "expected_revision":commitment["revision"], "confirmed":true,
        "action":"defer", "next_check":"2099-01-12T09:00:00+09:00",
        "evidence":"fixture:confirmed-another-follow-up"
    });
    ok(home.cli("check", &defer));
    let stale_error = home.tool("commitment_check", &stale_action);
    assert!(
        stale_error["error"].as_str().unwrap().contains("stale"),
        "{stale_error}"
    );
    assert_eq!(stale_error, home.cli("check", &stale_action));
    assert_eq!(ok(home.tool("close_day", &day))["closed"], true);
    let snapshot = ok(home.tool("get_today", &day));
    // A later registered obligation must not rewrite the already closed day.
    ok(home.cli(
        "register",
        &register(
            "later-obligation",
            "Later due review",
            "2026-10-02T00:00:00+09:00",
        ),
    ));
    assert_eq!(ok(home.tool("close_day", &day))["closed"], true);
    let reopened = ok(home.tool("get_today", &day));
    assert_eq!(reopened["day"], snapshot["day"]);
}

#[test]
fn same_title_requests_ambiguous_old_and_cancelled_updates_are_safe() {
    let home = Home::new();
    let mut first_input = register("same-title-a", "API review", "2099-01-07T09:00:00+09:00");
    first_input["source_ref"] = json!("fixture:thread-A");
    let mut second_input = register("same-title-b", "API review", "2099-01-07T09:00:00+09:00");
    second_input["counterparty"] = json!("sample-reviewer-B");
    second_input["source_ref"] = json!("fixture:thread-B");
    let mut first = ok(home.cli("register", &first_input))["commitment"].clone();
    let second = ok(home.tool("commitment_register", &second_input))["commitment"].clone();
    assert_ne!(first["id"], second["id"]);
    let ambiguous = json!({
        "source":"fixture", "external_id":"unknown-thread", "kind":"additional_info",
        "occurred_at":"2026-10-03T01:00:00Z", "request":"API review",
        "evidence":"fixture:sender-not-determined"
    });
    let proposal = ok(home.cli("ingest", &ambiguous));
    assert_eq!(proposal["candidates"].as_array().unwrap().len(), 2);
    let missing_target = json!({
        "op_id":"do-not-guess", "update_id":proposal["update"]["id"],
        "expected_revision":1, "confirmed":true
    });
    let error = home.cli("apply", &missing_target);
    assert!(
        error["error"].as_str().unwrap().contains("ambiguous"),
        "{error}"
    );
    assert_eq!(error, home.tool("commitment_apply", &missing_target));
    let selected = apply("person-selected-A", &proposal["update"], &first);
    first = ok(home.tool("commitment_apply", &selected))["commitment"].clone();

    let mut deadline = ingest(
        "newest-deadline",
        "deadline_change",
        &first,
        "2026-10-03T02:00:00Z",
    );
    deadline["reply_due"] = json!("2099-01-12T09:00:00+09:00");
    let proposal = ok(home.cli("ingest", &deadline));
    first = ok(home.cli("apply", &apply("apply-newest", &proposal["update"], &first)))
        ["commitment"]
        .clone();
    let mut old = ingest(
        "older-deadline",
        "deadline_change",
        &first,
        "2026-10-03T00:00:00Z",
    );
    old["reply_due"] = json!("2099-01-06T09:00:00+09:00");
    let old_proposal = ok(home.tool("commitment_ingest", &old));
    let error = home.cli(
        "apply",
        &apply("reject-old-overwrite", &old_proposal["update"], &first),
    );
    assert!(error["error"].as_str().unwrap().contains("old"), "{error}");
    let mut reject = apply("person-rejects-old", &old_proposal["update"], &first);
    reject["action"] = json!("reject");
    reject["evidence"] = json!("fixture:older-notice-superseded");
    first = ok(home.tool("commitment_apply", &reject))["commitment"].clone();
    assert_eq!(first["reply_due"], "2099-01-12T00:00:00.000000000Z");

    let cancel = ingest("cancel-A", "cancellation", &first, "2026-10-03T03:00:00Z");
    let cancellation = ok(home.cli("ingest", &cancel));
    let mut cancellation_args = apply("cancel-explicit", &cancellation["update"], &first);
    cancellation_args["confirmed"] = json!(false);
    let error = home.tool("commitment_apply", &cancellation_args);
    assert!(error.get("error").is_some(), "{error}");
    assert_eq!(error, home.cli("apply", &cancellation_args));
    let before = ok(home.cli("list", &json!({"at":"2026-10-03T04:00:00Z"})));
    assert_eq!(before["commitments"][0]["status"], "waiting");
    cancellation_args["confirmed"] = json!(true);
    let cancelled = ok(home.cli("apply", &cancellation_args));
    assert_eq!(cancelled["commitment"]["status"], "cancelled");
    assert!(cancelled["commitment"]["next_check"].is_null());
    assert_eq!(cancelled, home.tool("commitment_apply", &cancellation_args));
    let after = ok(home.cli("list", &json!({"at":"2026-10-03T04:00:00Z"})));
    let untouched = after["commitments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == second["id"])
        .unwrap();
    assert_eq!(
        untouched, &second,
        "same title second request must be unchanged"
    );

    let mut conflict = cancel.clone();
    conflict["evidence"] = json!("fixture:different-content-for-same-event");
    let error = home.tool("commitment_ingest", &conflict);
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("identity conflict"),
        "{error}"
    );
}

#[test]
fn partial_and_failed_sync_are_explicit_and_do_not_change_promises() {
    let home = Home::new();
    let commitment = ok(home.cli(
        "register",
        &register(
            "sync-review",
            "Review despite unavailable source",
            "2099-01-07T09:00:00+09:00",
        ),
    ))["commitment"]
        .clone();
    for (index, status) in ["partial", "failed", "unavailable"].iter().enumerate() {
        let input = json!({
            "op_id":format!("sync-{index}"), "source":"fixture", "status":status,
            "at":format!("2026-10-03T0{index}:00:00Z"),
            "evidence":format!("fixture:{status}-no-complete-observation")
        });
        let sync = ok(home.cli("sync", &input));
        assert_eq!(sync, home.tool("commitment_sync", &input));
        assert_eq!(sync["sync"]["status"], *status);
    }
    let list = ok(home.cli("list", &json!({"at":"2026-10-03T04:00:00Z"})));
    assert_eq!(list["commitments"][0], commitment);
    assert_eq!(list["sync"].as_array().unwrap().len(), 3);
    assert_eq!(list["commitments"][0]["status"], "waiting");
    assert!(list["commitments"][0]["reply"].is_null());
}

#[test]
fn dependency_follows_carried_task_and_reply_does_not_finish_it() {
    let home = Home::new();
    let task = ok(home.tool(
        "add_task",
        &json!({"title":"Implement reviewed contract","date":"2099-01-06"}),
    ));
    let mut input = register(
        "blocking-review",
        "Confirm contract",
        "2099-01-12T09:00:00+09:00",
    );
    input["related_task_ids"] = json!([task["id"]]);
    input["blocked_task_ids"] = json!([task["id"]]);
    let mut commitment = ok(home.cli("register", &input))["commitment"].clone();
    let carried = ok(home.tool(
        "carry_over",
        &json!({"id":task["id"],"reason":"Waiting for review","to":"2099-01-07"}),
    ));
    let list = ok(home.cli("list", &json!({"at":"2026-10-03T00:00:00Z"})));
    assert_eq!(list["commitments"][0]["id"], commitment["id"]);
    assert!(list["commitments"][0]["related_task_ids"]
        .as_array()
        .unwrap()
        .contains(&carried["id"]));
    assert!(list["commitments"][0]["blocked_task_ids"]
        .as_array()
        .unwrap()
        .contains(&carried["id"]));
    let next = home
        .command()
        .args(["next", "--date", "2099-01-07"])
        .output()
        .unwrap();
    assert!(next.status.success());
    assert!(!String::from_utf8_lossy(&next.stdout).contains(carried["id"].as_str().unwrap()));

    let reply = ingest(
        "blocking-reply",
        "reply",
        &commitment,
        "2026-10-03T01:00:00Z",
    );
    let proposal = ok(home.tool("commitment_ingest", &reply));
    commitment = ok(home.cli(
        "apply",
        &apply("apply-blocking-reply", &proposal["update"], &commitment),
    ))["commitment"]
        .clone();
    assert_eq!(commitment["status"], "replied");
    let day = ok(home.tool("get_today", &json!({"date":"2099-01-07"})));
    assert_eq!(day["tasks"][0]["state"], "planned");
    let next = home
        .command()
        .args(["next", "--date", "2099-01-07"])
        .output()
        .unwrap();
    assert!(next.status.success());
    assert!(String::from_utf8_lossy(&next.stdout).contains(carried["id"].as_str().unwrap()));
    assert_eq!(
        ok(home.tool("get_today", &json!({"date":"2099-01-06"})))["tasks"][0]["state"],
        "carried"
    );
}

#[test]
fn fixture_intake_proposes_once_and_reports_identity_conflict_as_partial() {
    let home = Home::new();
    let directory = home.0.join("fixture");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("mails.json"), "[]").unwrap();
    std::fs::write(directory.join("events.json"), "[]").unwrap();
    let row = json!({
        "source":"fixture", "external_id":"fixture-request-1", "kind":"new_request",
        "occurred_at":"2026-10-03T01:00:00Z", "request":"Fixture request",
        "counterparty":"sample-reviewer", "next_check":"2099-01-12T09:00:00+09:00",
        "evidence":"fixture:explicit-request"
    });
    std::fs::write(
        directory.join("commitments.json"),
        json!([row.clone()]).to_string(),
    )
    .unwrap();
    for _ in 0..2 {
        let output = home
            .command()
            .args(["intake", "fixture"])
            .arg(&directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let list = ok(home.cli("list", &json!({"at":"2026-10-03T02:00:00Z"})));
    assert!(
        list["commitments"].as_array().unwrap().is_empty(),
        "import does not imply person consent"
    );
    assert_eq!(list["pending_updates"].as_array().unwrap().len(), 1);
    assert!(list["sync"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["status"] == "success"));
    let update = &list["pending_updates"][0];
    let commitment = ok(home.tool(
        "commitment_apply",
        &json!({
            "op_id":"person-approves-fixture", "update_id":update["id"],
            "expected_revision":0, "confirmed":true
        }),
    ))["commitment"]
        .clone();
    let mut conflict = row;
    conflict["evidence"] = json!("fixture:contradictory-content-for-same-event");
    std::fs::write(
        directory.join("commitments.json"),
        json!([conflict]).to_string(),
    )
    .unwrap();
    let output = home
        .command()
        .args(["intake", "fixture"])
        .arg(&directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "adapter returns structured partial result"
    );
    let list = ok(home.cli("list", &json!({"at":"2026-10-03T02:00:00Z"})));
    assert_eq!(list["commitments"][0], commitment);
    assert!(list["pending_updates"].as_array().unwrap().is_empty());
    assert!(
        list["sync"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["status"] == "partial"
                && s["evidence"]
                    .as_str()
                    .unwrap()
                    .contains("identity conflict")),
        "{list}"
    );
}

#[test]
fn future_wait_keeps_blocked_backlog_quiet_and_day_can_close() {
    let home = Home::new();
    let task = ok(home.tool("add_task", &json!({"title":"Work after external review","backlog":true})));
    let mut input = register("backlog-wait", "Wait for review", "2100-01-07T09:00:00+09:00");
    input["blocked_task_ids"] = json!([task["id"]]);
    let commitment = ok(home.cli("register", &input))["commitment"].clone();
    for tool in ["plan_day", "check_in"] {
        let view = ok(home.tool(tool, &json!({"date":"2099-01-06"})));
        let questions = view["questions"].as_array().unwrap();
        assert!(!questions.iter().any(|q| q["target_id"] == task["id"] || q["target_id"] == commitment["id"]), "{view}");
    }
    assert_eq!(ok(home.tool("close_day", &json!({"date":"2099-01-06"})))["closed"], true);
    let later = ok(home.cli("list", &json!({"at":"2100-01-07T00:00:00Z"})));
    let question = &later["questions"][0];
    assert!(question["options"][0]["tool"].is_null());
    for option in question["options"].as_array().unwrap().iter().skip(1) {
        assert!(option["args"].get("confirmed").is_none());
        assert!(option["needs"].as_array().unwrap().contains(&json!("confirmed")));
    }
}

#[test]
fn unreadable_fixture_records_failure_without_claiming_no_reply() {
    let home = Home::new();
    let missing = home.0.join("missing-fixture");
    let result = home.command().args(["intake","fixture"]).arg(&missing).output().unwrap();
    assert!(!result.status.success());
    let list = ok(home.cli("list", &json!({})));
    assert_eq!(list["sync"][0]["status"], "failed");
    assert!(list["pending_updates"].as_array().unwrap().is_empty());
    assert!(list["commitments"].as_array().unwrap().is_empty());
}
