use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const OLD_DAY: &str = "2026-10-14";
const NEW_DAY: &str = "2026-10-15";

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dayloop-learned-done-{}", ulid::Ulid::new()));
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
        command.env("DAYLOOP_HOME", &self.0).env("PATH", "");
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
            "DAYLOOP_WINCLI",
        ] {
            command.env_remove(key);
        }
        command
    }

    fn mcp(&self, tool: &str, arguments: Value) -> Value {
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
                "params":{"name":tool,"arguments":arguments}
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
        let rpc: Value = serde_json::from_slice(&output.stdout).unwrap();
        serde_json::from_str(rpc["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn seed_done_and_same_name_task(&self) -> String {
        let old = self.mcp(
            "add_task",
            json!({"title":"週次  レポート", "date":OLD_DAY}),
        );
        let done = self.mcp(
            "finish_task",
            json!({"id":old["id"], "evidence":"fixture:explicit-done"}),
        );
        assert_eq!(done["state"], "done");
        let new = self.mcp("add_task", json!({"title":"週次 レポート", "date":NEW_DAY}));
        new["id"].as_str().unwrap().to_owned()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_and_mcp_daily_flows_keep_same_title_open_until_person_answers() {
    let home = Home::new();
    let current_id = home.seed_done_and_same_name_task();

    let next = home
        .command()
        .args(["next", "--date", NEW_DAY])
        .output()
        .unwrap();
    assert!(
        next.status.success(),
        "{}",
        String::from_utf8_lossy(&next.stdout)
    );
    assert!(String::from_utf8_lossy(&next.stdout).contains("週次 レポート"));
    let check = home
        .command()
        .args(["check", "--date", NEW_DAY])
        .output()
        .unwrap();
    assert_eq!(
        check.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("未回答"));
    let close = home
        .command()
        .args(["close", "--date", NEW_DAY])
        .output()
        .unwrap();
    assert_eq!(
        close.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&close.stdout)
    );
    assert!(String::from_utf8_lossy(&close.stdout).contains("閉じられません"));

    let checked = home.mcp("check_in", json!({"date":NEW_DAY}));
    assert!(
        checked["questions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q["title"] == "週次 レポート"),
        "{checked}"
    );
    let closed = home.mcp("close_day", json!({"date":NEW_DAY}));
    assert_eq!(closed["closed"], false, "{closed}");
    assert_eq!(closed["pending_count"], 1, "{closed}");
    let pending = home.mcp("get_today", json!({"date":NEW_DAY}));
    assert!(
        pending["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == current_id && task["state"] == "planned"),
        "{pending}"
    );

    let answered = home.mcp(
        "finish_task",
        json!({"id":current_id,"evidence":"fixture:person-confirmed"}),
    );
    assert_eq!(answered["state"], "done");
    let closed = home.mcp("close_day", json!({"date":NEW_DAY}));
    assert_eq!(closed["closed"], true, "{closed}");
    assert_eq!(closed["pending_count"], 0, "{closed}");
}
