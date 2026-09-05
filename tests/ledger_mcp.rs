//! Exercise user-facing MCP operations against isolated ledgers, including missing answers.
use dayloop::{store::Store, tools};
use serde_json::json;
use std::path::PathBuf;

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("dayloop-ledger-mcp-{}", ulid::Ulid::new())))
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn mcp_guard_reopen_and_diagnosis_keep_the_ledger_contract() {
    let home = Home::new();
    let store = Store::open_at(home.0.join("dayloop.db")).unwrap();
    store.set_required_categories(&[]).unwrap();
    let day = json!({"date":"2026-09-07"});
    assert_eq!(tools::dispatch(&store, "close_day", &day)["closed"], true);
    assert!(tools::dispatch(
        &store,
        "add_task",
        &json!({"title":"遅れた追加","date":"2026-09-07"})
    )
    .get("error")
    .is_some());
    assert!(tools::dispatch(&store, "reopen_day", &day)
        .get("error")
        .is_some());
    assert!(store
        .get_day("2026-09-07")
        .unwrap()
        .unwrap()
        .closed_at
        .is_some());
    assert_eq!(
        tools::dispatch(
            &store,
            "reopen_day",
            &json!({"date":"2026-09-07","reason":"本人の訂正"})
        )["reopened"],
        true
    );
    let result = tools::dispatch(
        &store,
        "add_task",
        &json!({"title":"再開後の作業","date":"2026-09-07"}),
    );
    let id = result["id"].as_str().unwrap();
    assert!(tools::dispatch(
        &store,
        "schedule_task",
        &json!({"id":id,"date":"2026-09-08"})
    )
    .get("error")
    .is_some());
    assert!(
        tools::dispatch(&store, "confirm_plan", &json!({"date":"2026-09-08"}))
            .get("error")
            .is_some()
    );
    let waiting = tools::dispatch(&store, "close_day", &day);
    assert_eq!(waiting["closed"], false);
    assert!(!waiting["questions"][0]["options"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        store.get_task(id).unwrap().state,
        dayloop::model::State::Planned
    );
    assert_eq!(
        tools::dispatch(&store, "check_ledger", &json!({}))["ok"],
        true
    );
}
