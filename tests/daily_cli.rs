use dayloop::store::Store;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Home {
    fn command(&self, args: &[&str], input: Option<&str>, expected: i32) -> String {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_dayloop"));
        cmd.args(args)
            .env("DAYLOOP_HOME", &self.0)
            .env_remove("DAYLOOP_INTERACTIVE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if input.is_some() {
            cmd.env("DAYLOOP_INTERACTIVE", "1");
        }
        let mut child = cmd.spawn().unwrap();
        if let Some(text) = input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        } else {
            drop(child.stdin.take());
        }
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(expected),
            "{args:?}\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
}

#[test]
fn pdm_pjm_and_engineer_complete_explicit_daily_and_weekly_cli_workflows() {
    for (role, body) in [
        ("PdM", "ACTION: 顧客の要望を整理\n宿題: 判断事項を確認"),
        ("PjM", "ACTION: 依頼担当を確認\nTODO: 遅延の影響を確認"),
        (
            "Engineer",
            "ACTION: アラートを調査\n対応: レビュー結果を記録",
        ),
    ] {
        let home =
            Home(std::env::temp_dir().join(format!("dayloop-cli-{role}-{}", ulid::Ulid::new())));
        home.command(&["setup"], None, 0);
        home.command(&["plan", "--date", "2026-09-07", "--yes"], None, 2);
        let note: Value = serde_json::from_str(&home.command(
            &[
                "note",
                "--category",
                "meeting_results",
                "--title",
                role,
                "--body",
                body,
                "--meeting-id",
                "synthetic-meeting",
            ],
            None,
            0,
        ))
        .unwrap();
        assert_eq!(note["candidates"].as_array().unwrap().len(), 2);
        for c in note["candidates"].as_array().unwrap() {
            home.command(
                &[
                    "candidates",
                    "accept",
                    c["id"].as_str().unwrap(),
                    "--date",
                    "2026-09-07",
                ],
                None,
                0,
            );
        }
        home.command(&["plan", "--date", "2026-09-07"], Some("1\n"), 0);
        home.command(&["check", "--date", "2026-09-07", "--yes"], None, 2);
        let tasks = Store::open_at(home.0.join("dayloop.db"))
            .unwrap()
            .tasks_for_day("2026-09-07")
            .unwrap();
        assert_eq!(tasks.len(), 2);
        for t in tasks {
            home.command(
                &["done", &t.id, "--evidence", "synthetic:person-confirmed"],
                None,
                0,
            );
        }
        home.command(&["close", "--date", "2026-09-07", "--yes"], None, 2);
        for category in dayloop::business::Category::all() {
            home.command(
                &[
                    "reviews",
                    "record",
                    category.as_str(),
                    "not_checked",
                    "--reason",
                    "実サービス未接続の検証",
                    "--date",
                    "2026-09-07",
                ],
                None,
                0,
            );
        }
        home.command(&["close", "--date", "2026-09-07", "--yes"], None, 0);
        home.command(
            &[
                "retro",
                "--date",
                "2026-09-07",
                "--note",
                "翌週は依頼の期限を先に確認する",
            ],
            None,
            0,
        );
        let store = Store::open_at(home.0.join("dayloop.db")).unwrap();
        let day = store.get_day("2026-09-07").unwrap().unwrap();
        assert!(day.plan_confirmed_at.is_some() && day.closed_at.is_some());
        assert!(day.retro_note.is_some());
        assert!(store.integrity_issues().unwrap().is_empty());
    }
}
