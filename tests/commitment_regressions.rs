//! Process regressions for temporal deadlines, historical closure and plan snapshots.
//! Every invocation uses a fresh synthetic ledger and isolated child environment.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use rusqlite::Connection;
use serde_json::{json, Value};

const PAST_DAY: &str = "2026-10-02";
const FUTURE: &str = "2099-01-12T09:00:00+09:00";

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dayloop-regression-{}", ulid::Ulid::new()));
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
            .env("PATH", "")
            .stdin(Stdio::null());
        for key in [
            "TYPESAFE_API_KEY",
            "DAYLOOP_JEV_API_KEY",
            "DAYLOOP_LINK_LIVE",
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
            "DAYLOOP_INTERACTIVE",
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
        let body: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
            panic!(
                "CLI {action}: {e}; status={}; stdout={}; stderr={}",
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
            json!({"jsonrpc":"2.0", "id":1, "method":"tools/call",
            "params":{"name":name, "arguments":args}})
        )
        .unwrap();
        drop(input);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rpc: Value = serde_json::from_slice(&output.stdout).unwrap();
        let body: Value =
            serde_json::from_str(rpc["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            rpc["result"]["isError"],
            body.get("error").is_some(),
            "{rpc}"
        );
        body
    }

    fn register(&self, op_id: &str, next_check: &str) -> Value {
        let next_check = if next_check.contains('T') {
            next_check.to_owned()
        } else {
            format!("{next_check}T00:00:00+09:00")
        };
        ok(self.cli(
            "register",
            &json!({
                "op_id":op_id, "request":"Review this promise", "counterparty":"fixture-person",
                "reply_due":"2099-01-20T09:00:00+09:00", "next_check":next_check,
                "source":"manual", "evidence":"fixture:explicit-promise"
            }),
        ))
    }

    fn ritual(&self, verb: &str, date: &str, yes: bool) -> std::process::Output {
        let mut command = self.command();
        if yes {
            command.arg("--yes");
        }
        command.args([verb, "--date", date]).output().unwrap()
    }

    fn day_state(&self, date: &str) -> (Option<String>, Option<String>, Option<String>) {
        let connection = Connection::open(self.0.join("dayloop.db")).unwrap();
        connection
            .query_row(
                "SELECT plan_confirmed_at, closed_at, retro_note FROM days WHERE date=?1",
                [date],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ok(value: Value) -> Value {
    assert!(value.get("error").is_none(), "{value}");
    value
}

fn defer_args(commitment: &Value, op: &str) -> Value {
    json!({"op_id":op, "commitment_id":commitment["id"],
        "expected_revision":commitment["revision"], "confirmed":true, "action":"defer",
        "next_check":FUTURE, "evidence":"fixture:person-selected-follow-up"})
}

#[test]
fn past_afternoon_day_deadline_is_due_in_check_in_and_cli() {
    let home = Home::new();
    let commitment =
        home.register("past-afternoon", "2026-10-02T15:00:00+09:00")["commitment"].clone();
    assert!(commitment["id"].is_string(), "{commitment}");
    let view = ok(home.tool("check_in", &json!({"date":PAST_DAY})));
    assert!(
        view["questions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q["target_id"] == commitment["id"]),
        "{view}"
    );
    let output = home.ritual("check", PAST_DAY, true);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("未回答"));
}

#[test]
fn repeated_close_keeps_historical_day_and_exposes_later_due_question() {
    let home = Home::new();
    let initially_closed = ok(home.tool("close_day", &json!({"date":PAST_DAY})));
    assert_eq!(initially_closed["closed"], true);
    let initial_cli = home.ritual("close", PAST_DAY, true);
    assert!(
        initial_cli.status.success(),
        "{}",
        String::from_utf8_lossy(&initial_cli.stdout)
    );
    let initial_day_state = home.day_state(PAST_DAY);
    let markdown_path = home.0.join("days").join(format!("{PAST_DAY}.md"));
    let initial_markdown = std::fs::read(&markdown_path).unwrap();
    let before = ok(home.tool("get_today", &json!({"date":PAST_DAY})));
    let registered = home.register("after-close", "2026-10-02");
    let commitment = &registered["commitment"];
    assert!(commitment["id"].is_string(), "{registered}");

    let repeated = ok(home.tool("close_day", &json!({"date":PAST_DAY})));
    assert_eq!(repeated["closed"], true);
    assert_eq!(repeated["already_closed"], true);
    assert!(
        repeated["pending_count"].as_u64().unwrap() > 0,
        "{repeated}"
    );
    assert!(
        repeated["questions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q["target_id"] == commitment["id"]),
        "{repeated}"
    );
    let cli_pending = home.ritual("close", PAST_DAY, true);
    assert_eq!(
        cli_pending.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&cli_pending.stdout)
    );
    assert!(String::from_utf8_lossy(&cli_pending.stdout).contains("クローズ済み"));

    let decision = defer_args(commitment, "explicit-follow-up");
    ok(home.tool("commitment_check", &decision));
    let history_after_decision = ok(home.tool(
        "commitment_history",
        &json!({"commitment_id":commitment["id"]}),
    ));
    assert_eq!(
        history_after_decision["decisions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let settled = ok(home.tool("close_day", &json!({"date":PAST_DAY})));
    assert_eq!(settled["closed"], true);
    assert_eq!(settled["already_closed"], true);
    assert_eq!(settled["pending_count"], 0);
    let cli_done = home.ritual("close", PAST_DAY, true);
    assert!(
        cli_done.status.success(),
        "{}",
        String::from_utf8_lossy(&cli_done.stdout)
    );
    let after = ok(home.tool("get_today", &json!({"date":PAST_DAY})));
    assert_eq!(after["day"], before["day"]);
    assert_eq!(after["tasks"], before["tasks"]);
    assert_eq!(after["events"], before["events"]);
    assert_eq!(home.day_state(PAST_DAY), initial_day_state);
    assert_eq!(std::fs::read(markdown_path).unwrap(), initial_markdown);
    let history = ok(home.tool(
        "commitment_history",
        &json!({"commitment_id":commitment["id"]}),
    ));
    assert_eq!(
        history, history_after_decision,
        "repeated closure must not append decisions"
    );
}

#[test]
fn plan_confirmation_requires_pending_promise_and_update_answers_first() {
    let home = Home::new();
    let commitment = home.register("plan-due", "2026-10-02")["commitment"].clone();
    let update = json!({"source":"fixture", "external_id":"new-promise", "kind":"new_request",
        "occurred_at":"2026-10-03T00:00:00Z", "request":"A proposed new review",
        "counterparty":"fixture-person", "next_check":FUTURE, "evidence":"fixture:proposal"});
    let proposal = ok(home.tool("commitment_ingest", &update));

    let view = ok(home.tool("plan_day", &json!({"date":"2026-10-04"})));
    let questions = view["questions"].as_array().unwrap();
    let commitment_index = questions
        .iter()
        .position(|q| q["target_id"] == commitment["id"])
        .unwrap();
    let proposal_index = questions
        .iter()
        .position(|q| q["target_id"] == proposal["update"]["id"])
        .unwrap();
    let confirm_index = questions
        .iter()
        .position(|q| q["kind"] == "confirm_plan")
        .unwrap();
    assert!(
        commitment_index < confirm_index && proposal_index < confirm_index,
        "{view}"
    );

    let cli = home.ritual("plan", "2026-10-04", true);
    assert_eq!(
        cli.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&cli.stdout)
    );
    assert!(String::from_utf8_lossy(&cli.stdout).contains("未回答"));
    let revision = view["plan_revision"].clone();
    let blocked = home.tool(
        "confirm_plan",
        &json!({"date":"2026-10-04", "plan_revision":revision}),
    );
    assert!(blocked.get("error").is_some(), "{blocked}");
    let unresolved = ok(home.tool("get_today", &json!({"date":"2026-10-04"})));
    assert!(
        unresolved["day"]["plan_confirmed_at"].is_null(),
        "{unresolved}"
    );

    ok(home.tool(
        "commitment_check",
        &defer_args(&commitment, "resolve-before-plan"),
    ));
    let applied = ok(home.tool(
        "commitment_apply",
        &json!({
            "op_id":"approve-before-plan", "update_id":proposal["update"]["id"],
            "expected_revision":0, "confirmed":true
        }),
    ));
    assert!(applied["commitment"]["id"].is_string(), "{applied}");
    let refreshed = ok(home.tool("plan_day", &json!({"date":"2026-10-04"})));
    let fresh_revision = refreshed["plan_revision"].clone();
    let confirmed = ok(home.tool(
        "confirm_plan",
        &json!({"date":"2026-10-04", "plan_revision":fresh_revision}),
    ));
    assert!(confirmed["plan_confirmed_at"].is_string(), "{confirmed}");
}

#[test]
fn answered_prerequisites_allow_cli_and_mcp_plan_confirmation() {
    let home = Home::new();
    let commitment = home.register("plan-answer", "2026-10-02")["commitment"].clone();
    let decision = defer_args(&commitment, "defer-before-plan");
    ok(home.cli("check", &decision));
    let date = "2026-10-04";
    let view = ok(home.tool("plan_day", &json!({"date":date})));
    let revision = view["plan_revision"].clone();
    assert!(revision.is_string(), "{view}");
    let output = home.ritual("plan", date, true);
    assert_eq!(
        output.status.code(),
        Some(2),
        "confirmation still requires the person's explicit answer"
    );

    // The process prompt is the explicit consent path; answer the sole final confirmation.
    let mut command = home.command();
    let output = command
        .env("DAYLOOP_INTERACTIVE", "1")
        .args(["plan", "--date", date])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = output;
    child.stdin.as_mut().unwrap().write_all(b"1\n").unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("確定しました"));
    let confirmed = ok(home.tool("get_today", &json!({"date":date})));
    assert!(
        confirmed["day"]["plan_confirmed_at"].is_string(),
        "{confirmed}"
    );
}

#[test]
fn missing_and_stale_plan_revisions_never_confirm_a_changed_plan() {
    let home = Home::new();
    let date = "2026-10-04";
    let displayed = ok(home.tool("plan_day", &json!({"date":date})));
    let revision = displayed["plan_revision"].clone();
    assert!(revision.is_string(), "{displayed}");
    let missing = home.tool("confirm_plan", &json!({"date":date}));
    assert!(missing.get("error").is_some(), "{missing}");

    // A due promise created after display invalidates the token without changing confirmation state.
    home.register("stale-plan", "2026-10-02");
    let stale = home.tool(
        "confirm_plan",
        &json!({"date":date, "plan_revision":revision}),
    );
    assert!(stale.get("error").is_some(), "{stale}");
    let after = ok(home.tool("get_today", &json!({"date":date})));
    assert!(after["day"]["plan_confirmed_at"].is_null(), "{after}");

    let task_home = Home::new();
    let displayed = ok(task_home.tool("plan_day", &json!({"date":date})));
    let displayed_revision = displayed["plan_revision"].clone();
    ok(task_home.tool(
        "add_task",
        &json!({"title":"Added after display","date":date}),
    ));
    let stale = task_home.tool(
        "confirm_plan",
        &json!({"date":date, "plan_revision":displayed_revision}),
    );
    assert!(stale.get("error").is_some(), "{stale}");
    let unchanged = ok(task_home.tool("get_today", &json!({"date":date})));
    assert!(
        unchanged["day"]["plan_confirmed_at"].is_null(),
        "{unchanged}"
    );
    let refreshed = ok(task_home.tool("plan_day", &json!({"date":date})));
    let fresh_revision = refreshed["plan_revision"].clone();
    let confirmed = ok(task_home.tool(
        "confirm_plan",
        &json!({"date":date, "plan_revision":fresh_revision}),
    ));
    assert!(confirmed["plan_confirmed_at"].is_string(), "{confirmed}");
}

#[test]
fn future_date_does_not_advance_clock_and_process_replays_keep_identity() {
    let home = Home::new();
    let registered = home.register("future-check", FUTURE);
    let commitment = registered["commitment"].clone();
    assert!(commitment["id"].is_string(), "{registered}");
    let history = ok(home.tool(
        "commitment_history",
        &json!({"commitment_id":commitment["id"]}),
    ));
    let date = "2100-01-10";
    let mut first_revision = None;
    let mut first_questions = None;
    for _ in 0..2 {
        let replay = home.register("future-check", FUTURE);
        assert_eq!(replay["commitment"]["id"], commitment["id"]);
        assert_eq!(replay["commitment"]["revision"], commitment["revision"]);
        let view = ok(home.tool("plan_day", &json!({"date":date})));
        assert!(
            view["questions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|q| q["target_id"] != commitment["id"]),
            "{view}"
        );
        if let Some(revision) = &first_revision {
            assert_eq!(&view["plan_revision"], revision);
            assert_eq!(&view["questions"], first_questions.as_ref().unwrap());
        } else {
            first_revision = Some(view["plan_revision"].clone());
            first_questions = Some(view["questions"].clone());
        }
        let list = ok(home.cli("list", &json!({"at":"2026-10-03T00:00:00Z"})));
        assert_eq!(list["commitments"][0]["id"], commitment["id"]);
        assert!(list["questions"].as_array().unwrap().is_empty(), "{list}");
        assert_eq!(
            ok(home.tool(
                "commitment_history",
                &json!({"commitment_id":commitment["id"]})
            )),
            history
        );
    }
    let check = home.ritual("check", date, true);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    assert!(
        !has_interactive_question(&check.stdout),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    let close = home.ritual("close", date, true);
    assert!(
        close.status.success(),
        "{}",
        String::from_utf8_lossy(&close.stdout)
    );
    assert!(
        !has_interactive_question(&close.stdout),
        "{}",
        String::from_utf8_lossy(&close.stdout)
    );
}

fn has_interactive_question(output: &[u8]) -> bool {
    String::from_utf8_lossy(output)
        .lines()
        .any(|line| line.trim_start().starts_with("? "))
}

#[test]
fn cli_rechecks_the_plan_after_the_person_has_seen_the_confirmation() {
    let home = Home::new();
    let date = "2026-10-04";
    let mut child = home
        .command()
        .env("DAYLOOP_INTERACTIVE", "1")
        .args(["plan", "--date", date])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line.unwrap();
            if line.contains("この内容で確定しますか") {
                let _ = sender.send(());
            }
            text.push_str(&line);
            text.push('\n');
        }
        text
    });
    if receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        panic!(
            "confirmation prompt not reached: {}",
            reader.join().unwrap()
        );
    }
    // A separate real process can write while the person is answering. The
    // displayed snapshot must have released its read lock before this point.
    let added = home.tool(
        "add_task",
        &json!({"title":"Changed while answering","date":date}),
    );
    child.stdin.as_mut().unwrap().write_all(b"1\n").unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    let text = reader.join().unwrap();
    ok(added);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("予定の内容が変わった"), "{text}");
    let state = ok(home.tool("get_today", &json!({"date":date})));
    assert!(state["day"]["plan_confirmed_at"].is_null(), "{state}");
    let fresh = ok(home.tool("plan_day", &json!({"date":date})));
    let question = fresh["questions"].as_array().unwrap().last().unwrap();
    assert_eq!(question["kind"], "confirm_plan");
    ok(home.tool("confirm_plan", &question["options"][0]["args"]));
}

#[test]
fn future_commitment_changes_and_proposals_require_a_fresh_answered_plan() {
    let home = Home::new();
    let date = "2026-10-04";
    let original = ok(home.tool("plan_day", &json!({"date":date})));
    let c = home.register("new-future-promise", FUTURE)["commitment"].clone();
    let stale = home.tool(
        "confirm_plan",
        &json!({"date":date,"plan_revision":original["plan_revision"]}),
    );
    assert!(
        stale["error"]
            .as_str()
            .unwrap()
            .contains("予定の内容が変わった"),
        "{stale}"
    );
    let current = ok(home.tool("plan_day", &json!({"date":date})));
    let revision = current["plan_revision"].clone();
    ok(home.tool(
        "confirm_plan",
        &json!({"date":date,"plan_revision":revision}),
    ));
    // Replaying the same content after restart remains valid, not dependent on
    // generated question op IDs or plan_confirmed_at itself.
    ok(home.tool(
        "confirm_plan",
        &json!({"date":date,"plan_revision":revision}),
    ));
    let update = ok(home.tool(
        "commitment_ingest",
        &json!({"source":"fixture","external_id":"pending-deadline",
        "kind":"deadline_change","occurred_at":"2026-10-03T00:00:00Z","commitment_id":c["id"],
        "next_check":"2100-01-01T09:00:00+09:00","evidence":"fixture:proposed-change"}),
    ));
    let pending = ok(home.tool("plan_day", &json!({"date":date})));
    let before = home.day_state(date);
    let blocked = home.tool(
        "confirm_plan",
        &json!({"date":date,"plan_revision":pending["plan_revision"]}),
    );
    assert!(
        blocked["error"]
            .as_str()
            .unwrap()
            .contains("未回答の約束・変更確認"),
        "{blocked}"
    );
    assert_eq!(home.day_state(date), before);
    assert_eq!(home.ritual("plan", date, true).status.code(), Some(2));
    ok(home.tool("commitment_apply", &json!({"op_id":"explicit-reject","update_id":update["update"]["id"],
        "expected_revision":c["revision"],"confirmed":true,"action":"reject","evidence":"fixture:person-rejected-change"})));
    let refreshed = ok(home.tool("plan_day", &json!({"date":date})));
    ok(home.tool(
        "confirm_plan",
        &json!({"date":date,"plan_revision":refreshed["plan_revision"]}),
    ));
}

#[test]
fn changing_the_displayed_task_order_invalidates_the_confirmation() {
    let home = Home::new();
    let date = "2026-10-04";
    ok(home.tool("add_task", &json!({"title":"First review","date":date})));
    ok(home.tool("add_task", &json!({"title":"Second review","date":date})));
    let old = ok(home.tool("plan_day", &json!({"date":date})));
    let first = old["planned"][0]["title"].clone();
    let second = old["planned"][1]["title"].clone();
    for _ in 0..2 {
        ok(home.tool("prefer_order", &json!({"first":second,"second":first})));
    }
    let changed = ok(home.tool("plan_day", &json!({"date":date})));
    assert_eq!(changed["planned"][0]["title"], second);
    let stale = home.tool(
        "confirm_plan",
        &json!({"date":date,"plan_revision":old["plan_revision"]}),
    );
    assert!(stale.get("error").is_some(), "{stale}");
    ok(home.tool(
        "confirm_plan",
        &json!({"date":date,"plan_revision":changed["plan_revision"]}),
    ));
}
