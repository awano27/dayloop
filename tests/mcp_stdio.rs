//! Spawn `dayloop mcp` and drive initialize → tools/list → add_task → close_day.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

const REQUIRED_TOOLS: &[&str] = &[
    "get_today",
    "plan_day",
    "confirm_plan",
    "add_task",
    "schedule_task",
    "start_task",
    "finish_task",
    "set_not_done",
    "carry_over",
    "drop_task",
    "split_task",
    "check_in",
    "close_day",
    "retro_week",
    "list_candidates",
    "add_candidate",
    "accept_candidate",
    "reject_candidate",
    "export_markdown",
    "import_markdown",
];

fn frame(v: &Value) -> Vec<u8> {
    let s = serde_json::to_string(v).unwrap();
    format!("{s}\n").into_bytes()
}

fn complete_text(buf: &[u8]) -> &str {
    match std::str::from_utf8(buf) {
        Ok(t) => t,
        Err(e) => std::str::from_utf8(&buf[..e.valid_up_to()]).unwrap_or(""),
    }
}

fn parse_frames(buf: &[u8]) -> Vec<Value> {
    let text = complete_text(buf);
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if !text.ends_with('\n') {
        lines.pop();
    }
    lines
        .into_iter()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("line is not JSON: {e}: {l}")))
        .collect()
}

fn assert_ndjson(buf: &[u8]) {
    let raw = complete_text(buf);
    assert!(
        !raw.contains("Content-Length"),
        "stdout must not contain Content-Length: {raw}"
    );
    for line in raw.lines().filter(|l| !l.is_empty()) {
        if !raw.ends_with('\n') && line == raw.lines().last().unwrap_or("") {
            continue;
        }
        assert!(line.starts_with('{'), "each line must be one JSON object: {line}");
        let _: Value = serde_json::from_str(line).unwrap();
    }
}

fn tool_text(msg: &Value) -> Value {
    let text = msg["result"]["content"][0]["text"].as_str().expect("text");
    serde_json::from_str(text).unwrap()
}

#[test]
fn mcp_initialize_list_add_and_close_stays_open() {
    let home = std::env::temp_dir().join(format!(
        "dayloop-mcp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_dayloop"))
        .arg("mcp")
        .env("DAYLOOP_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }
        })))
        .unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        })))
        .unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        })))
        .unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "add_task",
                "arguments": { "title": "残すタスク", "date": "2026-09-03" }
            }
        })))
        .unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "close_day",
                "arguments": { "date": "2026-09-03" }
            }
        })))
        .unwrap();
    drop(stdin);

    let mut buf = Vec::new();
    let start = std::time::Instant::now();
    let mut tmp = [0u8; 4096];
    while start.elapsed() < Duration::from_secs(15) {
        match stdout.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
        let frames = parse_frames(&buf);
        if frames.len() >= 4 {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&home);

    assert_ndjson(&buf);
    let frames = parse_frames(&buf);
    assert!(
        frames.len() >= 4,
        "expected 4 RPC replies, got {}: {}",
        frames.len(),
        String::from_utf8_lossy(&buf)
    );

    let init = &frames[0];
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");

    let list = &frames[1];
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for want in REQUIRED_TOOLS {
        assert!(names.contains(want), "missing tool {want} in {names:?}");
    }

    let added = tool_text(&frames[2]);
    assert_eq!(added["title"], "残すタスク");
    assert_eq!(added["state"], "planned");

    let closed = tool_text(&frames[3]);
    assert_eq!(closed["closed"], false);
    assert!(!closed["questions"].as_array().unwrap().is_empty());
    assert_eq!(closed["questions"][0]["kind"], "close_task");
}

#[test]
fn mcp_carry_over_returns_carry_blocked_not_exception() {
    let home = std::env::temp_dir().join(format!(
        "dayloop-mcp-carry-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_dayloop"))
        .arg("mcp")
        .env("DAYLOOP_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name":"t","version":"0"} }
        })))
        .unwrap();
    stdin
        .write_all(&frame(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})))
        .unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "add_task", "arguments": { "title": "経費", "date": "2026-09-01" } }
        })))
        .unwrap();
    drop(stdin);

    let mut buf = Vec::new();
    stdout.read_to_end(&mut buf).ok();
    let _ = child.kill();
    let _ = child.wait();

    assert_ndjson(&buf);
    let frames = parse_frames(&buf);
    assert_eq!(frames[0]["result"]["protocolVersion"], "2024-11-05");
    let task = tool_text(&frames[1]);
    let mut id = task["id"].as_str().unwrap().to_string();

    let dates = ["2026-09-02", "2026-09-03", "2026-09-04"];
    for (i, to) in dates.iter().enumerate() {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dayloop"))
            .arg("mcp")
            .env("DAYLOOP_HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        stdin
            .write_all(&frame(&json!({
                "jsonrpc":"2.0","id":1,"method":"initialize",
                "params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}
            })))
            .unwrap();
        stdin
            .write_all(&frame(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})))
            .unwrap();
        stdin
            .write_all(&frame(&json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"carry_over","arguments":{"id":id,"reason":format!("{}", i+1),"to":to}}
            })))
            .unwrap();
        drop(stdin);
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).ok();
        let _ = child.kill();
        let _ = child.wait();
        assert_ndjson(&buf);
        let frames = parse_frames(&buf);
        let body = tool_text(&frames[1]);
        assert!(body.get("error").is_none(), "carry {i} failed: {body}");
        id = body["id"].as_str().unwrap().to_string();
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_dayloop"))
        .arg("mcp")
        .env("DAYLOOP_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}
        })))
        .unwrap();
    stdin
        .write_all(&frame(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})))
        .unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"carry_over","arguments":{"id":id,"reason":"4","to":"2026-09-07"}}
        })))
        .unwrap();
    drop(stdin);
    let mut buf = Vec::new();
    stdout.read_to_end(&mut buf).ok();
    let _ = child.kill();
    let _ = child.wait();
    assert_ndjson(&buf);
    let frames = parse_frames(&buf);
    assert_eq!(frames[1]["result"]["isError"], false);
    let body = tool_text(&frames[1]);
    assert_eq!(body["error"], "carry_blocked");
    assert_eq!(body["question"]["kind"], "carry_blocked");

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn mcp_close_day_options_are_structured() {
    let home = std::env::temp_dir().join(format!(
        "dayloop-mcp-opts-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_dayloop"))
        .arg("mcp")
        .env("DAYLOOP_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }
        })))
        .unwrap();
    stdin
        .write_all(&frame(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})))
        .unwrap();
    stdin
        .write_all(&frame(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})))
        .unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "add_task",
                "arguments": { "title": "経費精算", "date": "2026-09-03" }
            }
        })))
        .unwrap();
    stdin
        .write_all(&frame(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "close_day",
                "arguments": { "date": "2026-09-03" }
            }
        })))
        .unwrap();
    drop(stdin);

    let mut buf = Vec::new();
    stdout.read_to_end(&mut buf).ok();
    let _ = child.kill();
    let _ = child.wait();
    assert_ndjson(&buf);
    let frames = parse_frames(&buf);

    let names: Vec<&str> = frames[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();

    let closed = tool_text(&frames[3]);
    let q = &closed["questions"][0];
    assert_eq!(q["kind"], "close_task");
    assert!(q.get("resolve_with").is_none());
    let opts = q["options"].as_array().unwrap();
    assert_eq!(opts.len(), 4);
    let want = [
        ("完了", Some("finish_task"), &[] as &[&str]),
        ("未完了", Some("set_not_done"), &["reason"]),
        ("持ち越し", Some("carry_over"), &["reason"]),
        ("取り下げ", Some("drop_task"), &["reason"]),
    ];
    for (i, (label, tool, needs)) in want.iter().enumerate() {
        assert_eq!(opts[i]["label"], *label);
        assert!(opts[i].get("args").is_some());
        assert!(opts[i]["args"].get("id").is_some());
        let needs_v: Vec<&str> = opts[i]["needs"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        assert_eq!(needs_v, *needs);
        match tool {
            None => assert!(opts[i]["tool"].is_null()),
            Some(t) => {
                assert_eq!(opts[i]["tool"], *t);
                assert!(names.contains(t), "tool {t} not in tools/list: {names:?}");
            }
        }
    }

    let _ = std::fs::remove_dir_all(&home);
}
