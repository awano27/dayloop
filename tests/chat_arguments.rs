use dayloop::{model::State, store::Store, tools};
use serde_json::json;
use std::path::PathBuf;
struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("dayloop-chat-args-{}", ulid::Ulid::new())))
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn wrong_types_and_unknown_parameters_cannot_mutate_the_ledger() {
    let home = Home::new();
    let store = Store::open_at(home.0.join("dayloop.db")).unwrap();
    let wrong_date = tools::dispatch(&store, "confirm_plan", &json!({"date":42}));
    assert!(wrong_date.get("error").is_some());
    assert!(store.get_day(&dayloop::util::today()).unwrap().is_none());
    let extra = tools::dispatch(
        &store,
        "add_task",
        &json!({"title":"採用していない","backlog":true,"complete":true}),
    );
    assert!(extra.get("error").is_some());
    let wrong_bool = tools::dispatch(
        &store,
        "add_task",
        &json!({"title":"採用していない","backlog":"false"}),
    );
    assert!(wrong_bool.get("error").is_some());
    assert!(store.backlog().unwrap().is_empty());
}

#[test]
fn a_malformed_action_list_is_not_partially_applied() {
    let home = Home::new();
    let store = Store::open_at(home.0.join("dayloop.db")).unwrap();
    let task = store
        .add_task("大きな仕事", None, None, "manual", None, Some("2026-09-07"))
        .unwrap();
    let result = tools::dispatch(
        &store,
        "split_task",
        &json!({"id":task.id,"titles":["一つ目",false,"二つ目"],"reason":"分割","to":"2026-09-08"}),
    );
    assert!(result.get("error").is_some());
    assert_eq!(store.get_task(&task.id).unwrap().state, State::Planned);
    assert!(store.tasks_for_day("2026-09-08").unwrap().is_empty());
}

#[test]
fn an_explicit_empty_date_never_falls_back_to_today() {
    let home = Home::new();
    let store = Store::open_at(home.0.join("dayloop.db")).unwrap();
    for date in ["", "   "] {
        assert!(
            tools::dispatch(&store, "confirm_plan", &json!({"date":date}))
                .get("error")
                .is_some()
        );
        assert!(store.get_day(&dayloop::util::today()).unwrap().is_none());
    }
}
