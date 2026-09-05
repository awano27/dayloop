use dayloop::{store::Store, tools};
use serde_json::{json, Value};
use std::path::PathBuf;
struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("dayloop-workflow-{}", ulid::Ulid::new())))
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn call(s: &Store, n: &str, a: Value) -> Value {
    let v = tools::dispatch(s, n, &a);
    assert!(v.get("error").is_none(), "{n}: {v}");
    v
}

#[test]
fn seven_reviews_block_close_until_explicit_answers_then_weekly_note_is_saved() {
    let h = Home::new();
    let s = Store::open_at(h.0.join("dayloop.db")).unwrap();
    let date = json!({"date":"2026-09-07"});
    let v = call(&s, "plan_day", date.clone());
    assert_eq!(v["reviews"].as_array().unwrap().len(), 7);
    let v = call(&s, "close_day", date.clone());
    assert_eq!(v["closed"], false);
    assert_eq!(v["questions"].as_array().unwrap().len(), 7);
    assert!(s
        .get_day("2026-09-07")
        .unwrap()
        .unwrap()
        .closed_at
        .is_none());
    for c in dayloop::business::Category::all() {
        call(
            &s,
            "record_review",
            json!({"date":"2026-09-07","category":c.as_str(),"outcome":"not_checked","reason":"検証用。実サービスは未接続"}),
        );
    }
    call(&s, "confirm_plan", date.clone());
    assert_eq!(call(&s, "close_day", date.clone())["closed"], true);
    call(
        &s,
        "save_retro",
        json!({"date":"2026-09-07","note":"会議の宿題は翌日までに候補へ登録する"}),
    );
    let retro = call(&s, "retro_week", date);
    assert_eq!(
        retro["notes"][0]["retro_note"],
        "会議の宿題は翌日までに候補へ登録する"
    );
    assert_eq!(
        call(&s, "plan_day", json!({"date":"2026-09-08"}))["reviews"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
}

#[test]
fn previous_empty_day_has_a_close_question_and_routines_generate_once() {
    let h = Home::new();
    let s = Store::open_at(h.0.join("dayloop.db")).unwrap();
    call(
        &s,
        "add_routine",
        json!({"title":"勤怠確認","weekdays":["Mon","Tue"],"starts_on":"2026-09-07"}),
    );
    let first = call(&s, "plan_day", json!({"date":"2026-09-07"}));
    assert_eq!(first["planned"].as_array().unwrap().len(), 1);
    assert_eq!(
        call(&s, "plan_day", json!({"date":"2026-09-07"}))["planned"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let v = call(&s, "plan_day", json!({"date":"2026-09-08"}));
    assert!(v["questions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|q| q["kind"] == "close_previous_day"));
    assert!(tools::dispatch(
        &s,
        "record_review",
        &json!({"date":"2026-09-08","category":"tasks","outcome":"not_checked"})
    )
    .get("error")
    .is_some());
    assert!(
        tools::dispatch(&s, "save_retro", &json!({"date":"2026-09-08","note":"  "}))
            .get("error")
            .is_some()
    );
}
