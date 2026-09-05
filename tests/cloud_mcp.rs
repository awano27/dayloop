use dayloop::{model::State, store::Store};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dayloop-cloud-mcp-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn call(&self) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dayloop"))
            .args(["mcp", "--profile", "github-copilot"])
            .env("DAYLOOP_HOME", &self.0)
            .env_remove("DAYLOOP_LLM_ENDPOINT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        // A disabled profile can exit before stdin is consumed.
        let _ = writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})
        );
        let _ = writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_today","arguments":{"date":"2026-09-07"}}})
        );
        drop(stdin);
        child.wait_with_output().unwrap()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_disabled_cloud_connection_cannot_open_or_disclose_a_ledger() {
    let home = Home::new();
    let out = home.call();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(!home.0.join("dayloop.db").exists());
}

#[test]
fn real_stdio_transport_applies_cloud_filter_to_nested_local_data() {
    let home = Home::new();
    std::fs::write(
        home.0.join("config.toml"),
        "[ai.github_copilot]\nenabled=true",
    )
    .unwrap();
    let store = Store::open_at(home.0.join("dayloop.db")).unwrap();
    let task = store
        .add_task(
            "公開してよいタイトル",
            None,
            None,
            "manual",
            Some("SECRET-REF"),
            Some("2026-09-07"),
        )
        .unwrap();
    store
        .transition(&task.id, State::Done, None, Some("SECRET-EVIDENCE"))
        .unwrap();
    store.set_retro("2026-09-07", "SECRET-NOTE").unwrap();
    drop(store);
    let out = home.call();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.contains("SECRET"), "{text}");
    let replies: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let body: Value =
        serde_json::from_str(replies[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["tasks"][0]["title"], "公開してよいタイトル");
    assert_eq!(body["tasks"][0]["state"], "done");
}
