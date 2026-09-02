//! MCP server over stdio. JSON-RPC 2.0, one JSON object per line (no headers).
//! stdout is reserved for protocol; logs go to stderr.

use std::io::{self, BufRead, Write};

use anyhow::Result;
use serde_json::{json, Value};

use crate::store::Store;
use crate::tools;

pub fn run() -> Result<()> {
    let store = Store::open()?;
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut stdout = io::stdout();
    loop {
        let msg = match read_msg(&mut reader) {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(e) => {
                eprintln!("mcp read: {e}");
                break;
            }
        };
        if let Some(resp) = handle(&store, &msg) {
            if let Err(e) = write_msg(&mut stdout, &resp) {
                eprintln!("mcp write: {e}");
                break;
            }
        }
    }
    Ok(())
}

fn handle(store: &Store, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    let is_notification = id.is_none() || method.starts_with("notifications/");

    match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2025-06-18");
            let version = if requested == "2024-11-05" {
                "2024-11-05"
            } else {
                "2025-06-18"
            };
            Some(ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": {
                        "name": "dayloop",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
            ))
        }
        "notifications/initialized" | "initialized" => None,
        "ping" => Some(ok(id, json!({}))),
        "tools/list" => Some(ok(id, json!({ "tools": tools::list() }))),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let result = tools::dispatch(store, name, &args);
            let is_error = match result.get("error") {
                Some(Value::String(s)) if s == "carry_blocked" => false,
                Some(_) => true,
                None => false,
            };
            Some(ok(
                id,
                json!({
                    "content": [{ "type": "text", "text": result.to_string() }],
                    "isError": is_error
                }),
            ))
        }
        _ if is_notification => None,
        _ => Some(err(id, -32601, &format!("Method not found: {method}"))),
    }
}

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn err(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message }
    })
}

fn read_msg<R: BufRead>(r: &mut R) -> io::Result<Option<Value>> {
    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str(trimmed) {
            Ok(v) => return Ok(Some(v)),
            Err(e) => {
                eprintln!("mcp read: invalid JSON line: {e}");
            }
        }
    }
}

fn write_msg<W: Write>(w: &mut W, v: &Value) -> io::Result<()> {
    let s = serde_json::to_string(v)?;
    if s.contains('\n') {
        eprintln!("mcp write: JSON contains newline, not sending");
        return Ok(());
    }
    writeln!(w, "{s}")?;
    w.flush()
}
