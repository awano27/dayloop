use dayloop::{
    business::{Category, ReviewOutcome},
    store::Store,
    tools,
};
use serde_json::json;
use std::path::PathBuf;
struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn home() -> Home {
    Home(std::env::temp_dir().join(format!("dayloop-mirror-{}", ulid::Ulid::new())))
}

#[test]
fn committed_mcp_operations_report_mirror_warning_instead_of_a_false_failure() {
    let h = home();
    let store = Store::open_at(h.0.join("dayloop.db")).unwrap();
    store.prepare_day("2026-09-07").unwrap();
    std::fs::create_dir_all(h.0.join("days/2026-09-07.md")).unwrap();
    let review = tools::dispatch(
        &store,
        "record_review",
        &json!({"date":"2026-09-07","category":"tasks","outcome":"confirmed"}),
    );
    assert!(review.get("error").is_none());
    assert_eq!(review["warnings"][0]["code"], "markdown_export_failed");
    assert_eq!(
        store
            .review_history("2026-09-07", Category::Tasks)
            .unwrap()
            .len(),
        2
    );
    for c in Category::all() {
        store
            .record_review("2026-09-07", c, ReviewOutcome::Confirmed, None, None, None)
            .unwrap();
    }
    for (name, args, key) in [
        (
            "confirm_plan",
            json!({"date":"2026-09-07"}),
            "plan_confirmed_at",
        ),
        ("close_day", json!({"date":"2026-09-07"}), "closed"),
        (
            "save_retro",
            json!({"date":"2026-09-07","note":"本人の振り返り"}),
            "saved",
        ),
        (
            "reopen_day",
            json!({"date":"2026-09-07","reason":"本人の訂正"}),
            "reopened",
        ),
    ] {
        let result = tools::dispatch(&store, name, &args);
        assert!(result.get("error").is_none(), "{name}: {result}");
        assert!(!result[key].is_null());
        assert_eq!(result["warnings"][0]["code"], "markdown_export_failed");
    }
    let day = store.get_day("2026-09-07").unwrap().unwrap();
    assert!(day.closed_at.is_none());
    assert_eq!(day.retro_note.as_deref(), Some("本人の振り返り"));
    assert!(
        tools::dispatch(&store, "export_markdown", &json!({"date":"2026-09-07"}))
            .get("error")
            .is_some()
    );
}

#[test]
fn cli_success_and_warning_match_a_committed_task_transition() {
    let h = home();
    let store = Store::open_at(h.0.join("dayloop.db")).unwrap();
    let task = store
        .add_task("作業", None, None, "manual", None, Some("2026-09-07"))
        .unwrap();
    std::fs::create_dir_all(h.0.join("days/2026-09-07.md")).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_dayloop"))
        .args(["done", &task.id])
        .env("DAYLOOP_HOME", &h.0)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("反映されています"));
    assert_eq!(
        store.get_task(&task.id).unwrap().state,
        dayloop::model::State::Done
    );
}
