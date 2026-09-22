# MCP でも既知の枝を適用する Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `plan_day` / `check_in` / `close_day` が、CLI の儀式と同じく、質問を作る前に既知の枝を台帳へ適用する。

**Architecture:** 要件が名指ししている `engine::plan_view` / `check_view` / `close_view` は、読み取りのまま残す。MCP はその戻りを質問として出すので、適用は `src/tools.rs` の `plan_day` / `check_in` / `close_day` で、ビューを呼ぶ前に行う。`get_today` は適用しない。

**Tech Stack:** 既存の `graph::apply_known_tasks` と `apply_known_candidates`。基準は `1db379706a833392a315bd153f43ca34aad55b88`。

---

### Task 1: `close_day` が件名の枝を適用する

**Files:**
- Modify: `src/tools.rs`
- Test: `tests/mcp_edges.rs`

- [ ] **Step 1: Write the failing test**

`Home` は `tests/growth.rs` と同じ隔離。ロック名は `MCP_EDGE_LOCK`。

```rust
#[test]
fn close_day_applies_a_known_title_and_closes() {
    let home = Home::new();
    let store = home.store();
    let first = store.add_task("経費精算", None, None, "manual", None, Some("2026-09-22")).unwrap();
    store.transition(&first.id, State::NotDone, Some("no_time"), None).unwrap();
    let second = store.add_task("経費精算", None, None, "manual", None, Some("2026-09-23")).unwrap();
    let v = tools::dispatch(&store, "close_day", &serde_json::json!({"date":"2026-09-23"}));
    assert_eq!(v["closed"], true);
    assert!(v["questions"].as_array().unwrap().is_empty());
    let second = store.get_task(&second.id).unwrap();
    assert_eq!(second.state, State::NotDone);
    assert!(second.decided_by.unwrap().starts_with("graph:"));
}
```

`transition` は人の操作なので枝を書く。2件目は `close_day` の中の `apply_known_tasks` が `transition_silent` で未完了にする。

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test mcp_edges close_day_applies_a_known_title_and_closes -- --nocapture`

Expected: FAIL。`closed` が false で、質問に「経費精算」が残る。

- [ ] **Step 3: Write minimal implementation**

`tools.rs` の `close_day` 分岐の先頭で、open を数える前に呼ぶ。

```rust
"close_day" => {
    let d = date_arg(args)?;
    crate::graph::apply_known_tasks(store, &d)?;
    let open = store.open_tasks_for_day(&d)?;
    if !open.is_empty() {
        return engine::close_view(store, &d);
    }
    match store.close_day(&d)? {
        Ok(()) => {
            markdown::export(store, &d)?;
            Ok(json!({ "closed": true, "open": [], "questions": [] }))
        }
        Err(rest) => Ok(json!({
            "closed": false,
            "open": rest,
            "questions": rest.iter().map(engine::Question::close_task).collect::<Vec<_>>(),
        })),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test mcp_edges close_day_applies_a_known_title_and_closes -- --nocapture`

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add src/tools.rs tests/mcp_edges.rs
git commit -m "fix: apply known task edges before MCP close_day"
```

---

### Task 2: `check_in` と `plan_day`

**Files:**
- Modify: `src/tools.rs`
- Test: `tests/mcp_edges.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn plan_day_shelves_a_known_candidate_without_asking() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    graph::record(&store, &graph::Situation::intake("週報", "outlook", date), "shelve", None).unwrap();
    store.add_candidate("週報", "outlook", Some("mail:1")).unwrap();
    let v = tools::dispatch(&store, "plan_day", &serde_json::json!({"date": date}));
    let questions = v["questions"].as_array().unwrap();
    assert!(questions.iter().all(|q| q["title"] != "週報"));
    assert!(store.open_candidates().unwrap().is_empty());
}

#[test]
fn check_in_drops_a_known_title_before_questions() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    let t = store.add_task("不要な依頼", None, None, "manual", None, Some(date)).unwrap();
    store.transition(&t.id, State::Dropped, Some("not_needed"), None).unwrap();
    store.add_task("不要な依頼", None, None, "manual", None, Some(date)).unwrap();
    let v = tools::dispatch(&store, "check_in", &serde_json::json!({"date": date}));
    assert!(v["questions"].as_array().unwrap().is_empty());
}

#[test]
fn get_today_does_not_apply_edges() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-23";
    let first = store.add_task("経費精算", None, None, "manual", None, Some("2026-09-22")).unwrap();
    store.transition(&first.id, State::NotDone, Some("no_time"), None).unwrap();
    let second = store.add_task("経費精算", None, None, "manual", None, Some(date)).unwrap();
    tools::dispatch(&store, "get_today", &serde_json::json!({"date": date}));
    assert_eq!(store.get_task(&second.id).unwrap().state, State::Planned);
}
```

2件目の「不要な依頼」は、1件目を `Dropped` にした時点で件名の枝ができる。`check_in` は残った open にその枝を適用する。1件目は既に終端なので対象外。

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test mcp_edges plan_day_shelves_a_known_candidate_without_asking -- --nocapture`

Expected: FAIL。`plan_day` の質問に「週報」が残る。

- [ ] **Step 3: Write minimal implementation**

```rust
"plan_day" => {
    let d = date_arg(args)?;
    crate::graph::apply_known_candidates(store, &d)?;
    crate::graph::apply_known_tasks(store, &d)?;
    engine::plan_view(store, &d)
}
"check_in" => {
    let d = date_arg(args)?;
    crate::graph::apply_known_tasks(store, &d)?;
    engine::check_view(store, &d)
}
```

`get_today` は `engine::today_view` のまま。

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test mcp_edges -- --nocapture`

Expected: PASS。3件と Task 1 の1件。

`cargo test --locked --offline --test mcp_stdio --test growth` も通す。

- [ ] **Step 5: Commit**

```bash
git add src/tools.rs tests/mcp_edges.rs
git commit -m "fix: apply known edges in MCP plan and check"
```
