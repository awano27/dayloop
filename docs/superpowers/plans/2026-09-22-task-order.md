# 今日の並び Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 今日のタスクを期限・見積・空き・持ち越し回数で並べ、同順位の最初の1組だけを Jev の2択ヒントにする。

**Architecture:** `src/order.rs` が純関数で順位を決める。保存済みの `order` 枝は同順位の隣接1組だけを入れ替える。Jev はヒントで、枝を書かない。`dayloop prefer` が枝を書く。

**Tech Stack:** Rust, chrono, 既存の `facts` と `graph`。基準は `1db379706a833392a315bd153f43ca34aad55b88`。

---

### Task 1: 順位

**Files:**
- Create: `src/order.rs`
- Modify: `src/lib.rs`
- Test: `src/order.rs` の `mod tests`

- [x] **Step 1: Write the failing test**

`src/order.rs` の末尾に置く。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{State, Task};

    fn task(id: &str, title: &str, due: Option<&str>, estimate: Option<i64>, carried: i64, created: &str) -> Task {
        Task {
            id: id.into(),
            title: title.into(),
            source: "manual".into(),
            source_ref: None,
            due: due.map(|s| s.to_string()),
            estimate_min: estimate,
            plan_date: Some("2026-09-22".into()),
            state: State::Planned,
            state_reason: None,
            carried_count: carried,
            evidence: None,
            created_at: created.into(),
            closed_at: None,
            carried_from: None,
            proposed_state: None,
            proposed_reason_code: None,
            proposal_confidence: None,
            state_note: None,
            decided_by: None,
        }
    }

    #[test]
    fn overdue_before_today_before_future() {
        let spans = spans_hhmm(&[("10:00", "11:00")]);
        let tasks = vec![
            task("c", "未来", Some("2026-09-23"), Some(30), 0, "2026-09-22T01:00:00Z"),
            task("a", "超過", Some("2026-09-21"), Some(30), 0, "2026-09-22T03:00:00Z"),
            task("b", "今日", Some("2026-09-22"), Some(30), 0, "2026-09-22T02:00:00Z"),
        ];
        let ids: Vec<_> = sort_open("2026-09-22", tasks, &spans).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn higher_carry_wins_inside_the_same_due_class() {
        let tasks = vec![
            task("new", "新しい", Some("2026-09-21"), Some(30), 0, "2026-09-22T01:00:00Z"),
            task("old", "古い", Some("2026-09-21"), Some(30), 3, "2026-09-22T02:00:00Z"),
        ];
        let ids: Vec<_> = sort_open("2026-09-22", tasks, &[]).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["old", "new"]);
    }

    #[test]
    fn fits_before_unknown_before_over() {
        let spans = spans_hhmm(&[("09:00", "17:30")]);
        let tasks = vec![
            task("over", "入らない", Some("2026-09-22"), Some(60), 0, "2026-09-22T01:00:00Z"),
            task("fit", "入る", Some("2026-09-22"), Some(30), 0, "2026-09-22T02:00:00Z"),
            task("unk", "見積なし", Some("2026-09-22"), None, 0, "2026-09-22T00:00:00Z"),
        ];
        let ids: Vec<_> = sort_open("2026-09-22", tasks, &spans).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["fit", "unk", "over"]);
    }

    #[test]
    fn equal_rank_keeps_created_at_then_id() {
        let tasks = vec![
            task("b", "同着", Some("2026-09-22"), Some(30), 0, "2026-09-22T02:00:00Z"),
            task("a", "同着", Some("2026-09-22"), Some(30), 0, "2026-09-22T02:00:00Z"),
        ];
        let ids: Vec<_> = sort_open("2026-09-22", tasks, &[]).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --lib order::tests -- --nocapture`

Expected: FAIL。`order` がクレートに無い。

- [x] **Step 3: Write minimal implementation**

`src/lib.rs` の `pub mod facts;` の次に `pub mod order;` を足す。

`src/order.rs`:

```rust
//! Today's order. Rust ranks the work. Jev only hints on one tie.

use anyhow::Result;
use chrono::NaiveTime;

use crate::facts::{self, DueRelation, Fit};
use crate::model::{State, Task};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank {
    due_class: u8,
    carry_key: i64,
    fit_class: u8,
    estimate_key: i64,
}

pub fn spans_hhmm(pairs: &[(&str, &str)]) -> Vec<(NaiveTime, NaiveTime)> {
    pairs.iter().filter_map(|(a, b)| facts::span(a, b)).collect()
}

pub fn spans_from_events(events: &[crate::model::Event]) -> Vec<(NaiveTime, NaiveTime)> {
    events.iter().filter_map(|e| facts::span(&e.start, &e.end)).collect()
}

pub fn rank(today: &str, task: &Task, spans: &[(NaiveTime, NaiveTime)]) -> Rank {
    let due = facts::due_relation(today, task.due.as_deref());
    let due_class = match due {
        DueRelation::Overdue => 0,
        DueRelation::Today => 1,
        DueRelation::None => 2,
        DueRelation::Future => 3,
    };
    let fit = facts::fit_estimate(task.estimate_min, spans);
    let fit_class = match fit {
        Fit::Fits => 0,
        Fit::Unknown => 1,
        Fit::Over => 2,
    };
    Rank {
        due_class,
        carry_key: -task.carried_count,
        fit_class,
        estimate_key: task.estimate_min.unwrap_or(i64::MAX),
    }
}

pub fn sort_open(today: &str, mut tasks: Vec<Task>, spans: &[(NaiveTime, NaiveTime)]) -> Vec<Task> {
    tasks.sort_by(|a, b| {
        rank(today, a, spans)
            .cmp(&rank(today, b, spans))
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    tasks
}

pub fn sort_day(today: &str, mut tasks: Vec<Task>, spans: &[(NaiveTime, NaiveTime)]) -> Vec<Task> {
    tasks.sort_by(|a, b| {
        open_key(a.state)
            .cmp(&open_key(b.state))
            .then_with(|| rank(today, a, spans).cmp(&rank(today, b, spans)))
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    tasks
}

fn open_key(state: State) -> u8 {
    if state.is_open() { 0 } else { 1 }
}

pub fn first_open_tie<'a>(
    today: &str,
    tasks: &'a [Task],
    spans: &[(NaiveTime, NaiveTime)],
) -> Option<(&'a Task, &'a Task)> {
    let open: Vec<&Task> = tasks.iter().filter(|t| t.state.is_open()).collect();
    open.windows(2).find_map(|w| {
        if rank(today, w[0], spans) == rank(today, w[1], spans) {
            Some((w[0], w[1]))
        } else {
            None
        }
    })
}

pub fn day_tasks(store: &Store, date: &str) -> Result<Vec<Task>> {
    let tasks = store.tasks_for_day(date)?;
    let spans = spans_from_events(&store.events_for_day(date)?);
    Ok(sort_day(date, tasks, &spans))
}
```

`first_open_tie` と `day_tasks` は次のタスクで使う。このタスクではコンパイルが通ればよい。

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --lib order::tests -- --nocapture`

Expected: PASS。4件。

- [x] **Step 5: Commit**

```bash
git add src/order.rs src/lib.rs
git commit -m "feat: rank today's tasks by due, carry, fit, and estimate"
```

---

### Task 2: 同順位の保存済み優先

**Files:**
- Modify: `src/graph.rs`
- Modify: `src/order.rs`
- Test: `tests/order_pref.rs`

- [x] **Step 1: Write the failing test**

`tests/order_pref.rs`。`tests/growth.rs` の `Home` と同じ隔離を使う。ロック名は `ORDER_LOCK`。

```rust
use dayloop::graph;
use dayloop::model::State;
use dayloop::order;
use dayloop::store::Store;

static ORDER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Home { dir: std::path::PathBuf, _guard: std::sync::MutexGuard<'static, ()>, old: Option<String> }

impl Home {
    fn new() -> Self {
        let guard = ORDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("DAYLOOP_HOME").ok();
        let dir = std::env::temp_dir().join(format!("dayloop-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("DAYLOOP_HOME", &dir); }
        Self { dir, _guard: guard, old }
    }
    fn store(&self) -> Store { Store::open().unwrap() }
}

impl Drop for Home {
    fn drop(&mut self) {
        unsafe {
            match &self.old {
                Some(v) => std::env::set_var("DAYLOOP_HOME", v),
                None => std::env::remove_var("DAYLOOP_HOME"),
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn saved_preference_swaps_only_an_equal_rank_pair() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    store.add_task("甲", Some(date), Some(30), "manual", None, Some(date)).unwrap();
    store.add_task("乙", Some(date), Some(30), "manual", None, Some(date)).unwrap();
    graph::record_order(&store, "乙", "甲").unwrap();
    let titles: Vec<_> = order::day_tasks(&store, date).unwrap().into_iter().map(|t| t.title).collect();
    assert_eq!(titles, vec!["乙".to_string(), "甲".to_string()]);
}

#[test]
fn saved_preference_does_not_override_overdue() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    store.add_task("超過", Some("2026-09-21"), Some(30), "manual", None, Some(date)).unwrap();
    store.add_task("今日", Some(date), Some(30), "manual", None, Some(date)).unwrap();
    graph::record_order(&store, "今日", "超過").unwrap();
    let titles: Vec<_> = order::day_tasks(&store, date).unwrap().into_iter().map(|t| t.title).collect();
    assert_eq!(titles[0], "超過");
    assert!(store.open_tasks_for_day(date).unwrap().iter().all(|t| t.state == State::Planned));
}
```

`add_task` の引数順は `title, due, estimate_min, source, source_ref, plan_date`。

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test order_pref -- --nocapture`

Expected: FAIL。`record_order` が無い。

- [x] **Step 3: Write minimal implementation**

`src/graph.rs` に足す。`node_id` と `activate` は同じファイルの私有関数。

```rust
pub fn order_node_key(a: &str, b: &str) -> String {
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    format!("{x}\n{y}")
}

pub fn record_order(store: &Store, first: &str, second: &str) -> Result<()> {
    if first == second || first.trim().is_empty() || second.trim().is_empty() {
        anyhow::bail!("2つの違うタイトルが必要です");
    }
    let node = node_id(store, "order", &order_node_key(first, second))?;
    activate(store, &node, first, None)?;
    Ok(())
}

pub fn saved_first(store: &Store, a: &str, b: &str) -> Result<Option<String>> {
    let Some(edge) = active_edge(store, "order", &order_node_key(a, b))? else {
        return Ok(None);
    };
    if edge.to_choice == a || edge.to_choice == b {
        Ok(Some(edge.to_choice))
    } else {
        Ok(None)
    }
}
```

`src/order.rs` の `day_tasks` を差し替える。

```rust
pub fn day_tasks(store: &Store, date: &str) -> Result<Vec<Task>> {
    let tasks = store.tasks_for_day(date)?;
    let spans = spans_from_events(&store.events_for_day(date)?);
    let mut tasks = sort_day(date, tasks, &spans);
    apply_saved_first_tie(store, date, &spans, &mut tasks)?;
    Ok(tasks)
}

fn apply_saved_first_tie(
    store: &Store,
    today: &str,
    spans: &[(NaiveTime, NaiveTime)],
    tasks: &mut [Task],
) -> Result<()> {
    let pair = first_open_tie(today, tasks, spans).map(|(left, right)| {
        (left.id.clone(), right.id.clone(), left.title.clone(), right.title.clone())
    });
    let Some((left_id, right_id, left_title, right_title)) = pair else {
        return Ok(());
    };
    let Some(first) = crate::graph::saved_first(store, &left_title, &right_title)? else {
        return Ok(());
    };
    if first != right_title {
        return Ok(());
    }
    if let Some(i) = tasks.iter().position(|t| t.id == left_id) {
        if tasks.get(i + 1).map(|t| t.id == right_id).unwrap_or(false) {
            tasks.swap(i, i + 1);
        }
    }
    Ok(())
}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test order_pref -- --nocapture`

Expected: PASS。2件。

- [x] **Step 5: Commit**

```bash
git add src/graph.rs src/order.rs tests/order_pref.rs
git commit -m "feat: let a saved order edge break one equal-rank tie"
```

---

### Task 3: 表示と質問の順

**Files:**
- Modify: `src/engine.rs`（`today_view`、`plan_view`、`check_view`、`close_view`）
- Modify: `src/rituals.rs` の `print_day`
- Test: `tests/order_view.rs`

- [x] **Step 1: Write the failing test**

`tests/order_view.rs` は Task 2 と同じ `Home` を持つ。

```rust
#[test]
fn close_view_asks_overdue_first() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    store.add_task("今日", Some(date), Some(30), "manual", None, Some(date)).unwrap();
    store.add_task("超過", Some("2026-09-21"), Some(30), "manual", None, Some(date)).unwrap();
    let view = dayloop::engine::close_view(&store, date).unwrap();
    let first = view["questions"][0]["title"].as_str().unwrap();
    assert_eq!(first, "超過");
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test order_view close_view_asks_overdue_first -- --nocapture`

Expected: FAIL。先頭が `created_at` 順の「今日」。

- [x] **Step 3: Write minimal implementation**

`close_view` の `open` を `order::day_tasks` から取り、open だけを残す。

```rust
pub fn close_view(store: &Store, date: &str) -> Result<Value> {
    let open: Vec<Task> = crate::order::day_tasks(store, date)?
        .into_iter()
        .filter(|t| t.state.is_open())
        .collect();
    let questions: Vec<Question> = open.iter().map(Question::close_task).collect();
    Ok(json!({
        "closed": false,
        "open": open,
        "questions": questions,
    }))
}
```

`check_view` の `open` も同じ配列を使う。`untouched` は `State::Planned`、`doing` は `State::InProgress`。質問は `untouched` の並びのまま。

`today_view` と `plan_view` の `tasks` / `planned` は `order::day_tasks` の結果を使う。`plan_view` が前日の未クローズを歩く箇所は日付順のまま。各日の open タスク質問は、その日を `today` にした `order::day_tasks(store, d)` の open 順にする。

`print_day` の `let tasks = store.tasks_for_day(date)?` を `let tasks = crate::order::day_tasks(store, date)?` に変える。

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test order_view -- --nocapture`

Expected: PASS。

続けて `cargo test --locked --offline --lib --test growth --test invariants --test mcp_stdio`。質問順を `created_at` で固定しているテストが落ちたら、その期待を期限順へ直す。落ちたテスト名をコミットメッセージに書く。

- [x] **Step 5: Commit**

```bash
git add src/engine.rs src/rituals.rs tests/order_view.rs
git commit -m "feat: show and ask today's tasks in rank order"
```

---

### Task 4: `next` と `prefer`

**Files:**
- Modify: `src/main.rs`
- Modify: `src/order.rs`
- Test: `tests/order_cli.rs`

- [x] **Step 1: Write the failing test**

CLI を直接叩かず、ライブラリ関数を試す。

```rust
#[test]
fn next_open_is_the_first_ranked_task() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    store.add_task("今日", Some(date), Some(30), "manual", None, Some(date)).unwrap();
    let late = store.add_task("超過", Some("2026-09-21"), Some(30), "manual", None, Some(date)).unwrap();
    let next = order::next_open(&store, date).unwrap().unwrap();
    assert_eq!(next.id, late.id);
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test order_cli -- --nocapture`

Expected: FAIL。`next_open` が無い。

- [x] **Step 3: Write minimal implementation**

`src/order.rs`:

```rust
pub fn next_open(store: &Store, date: &str) -> Result<Option<Task>> {
    Ok(day_tasks(store, date)?.into_iter().find(|t| t.state.is_open()))
}
```

`src/main.rs` の `Cmd` に足す。

```rust
    /// 次に手を付ける未完了タスクを1件出す
    Next {
        #[arg(long)]
        date: Option<String>,
    },
    /// 同順位の2件について、先にやるタイトルを覚える
    Prefer {
        first: String,
        second: String,
    },
```

`run` の match に足す。日付の解決は `Cmd::Today` と同じ `resolve_date` を使う。

```rust
Cmd::Next { date } => {
    let date = resolve_date(date)?;
    match order::next_open(&store, &date)? {
        Some(t) => {
            println!("{}  {}", t.id, t.title);
            Ok(0)
        }
        None => {
            println!("{date} に未完了はありません");
            Ok(0)
        }
    }
}
Cmd::Prefer { first, second } => {
    graph::record_order(&store, &first, &second)?;
    println!("覚えました: 「{first}」を「{second}」より先");
    Ok(0)
}
```

`graph` が `main.rs` で未使用なら `use dayloop::graph;` ではなく、このファイルが既に `crate` 外の binary なので `use dayloop::graph;` を既存の import 群に足す。binary は `dayloop::` を使っている。既存の `use` に合わせる。

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test order_cli -- --nocapture`

Expected: PASS。

手動: `cargo run --locked --offline -- next --date 2026-09-22` は、台帳が空なら「未完了はありません」で終了コード 0。`DAYLOOP_HOME` は一時ディレクトリを指定する。

- [x] **Step 5: Commit**

```bash
git add src/order.rs src/main.rs tests/order_cli.rs
git commit -m "feat: add next and prefer commands for today's order"
```

---

### Task 5: 同順位のヒント

**Files:**
- Modify: `src/engine.rs` の `plan_view` と `check_view` と `close_view`
- Modify: `src/jev.rs`
- Modify: `src/rituals.rs` の `print_jev_hints`
- Test: `src/order.rs` と `tests/order_hint.rs`

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn tie_hint_returns_only_one_of_the_two_titles() {
    let mut decider = Script { choice: "乙".into(), calls: 0 };
    let hint = order::tie_hint(&mut decider, "甲", "乙");
    assert_eq!(hint, Some("乙".into()));
    assert_eq!(decider.calls, 1);
}

struct Script { choice: String, calls: usize }

impl crate::jev::Decider for Script {
    fn decide(&mut self, _state: &str, choices: &[String]) -> crate::jev::JevOutcome {
        self.calls += 1;
        assert_eq!(choices, ["甲".to_string(), "乙".to_string()]);
        crate::jev::JevOutcome::Answer { choice: self.choice.clone(), confidence: 1.0 }
    }
}
```

このテストは `src/order.rs` の `mod tests` に置く。`Decider` を実装するなら `order.rs` から `jev` を参照する。

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --lib tie_hint_returns_only_one_of_the_two_titles -- --nocapture`

Expected: FAIL。`tie_hint` が無い。

- [x] **Step 3: Write minimal implementation**

```rust
pub fn tie_hint(decider: &mut dyn crate::jev::Decider, a: &str, b: &str) -> Option<String> {
    let choices = vec![a.to_string(), b.to_string()];
    let state = format!("order a={a} b={b}");
    match decider.decide(&state, &choices) {
        crate::jev::JevOutcome::Answer { choice, .. } if choice == a || choice == b => Some(choice),
        _ => None,
    }
}
```

`src/engine.rs` に足す。同順位が無ければ `null`。`questions` には入れない。未回答でも `confirm_plan` と `close_day` をブロックしない。

```rust
fn order_tie_json(store: &Store, date: &str) -> Result<Value> {
    let tasks = crate::order::day_tasks(store, date)?;
    let spans = crate::order::spans_from_events(&store.events_for_day(date)?);
    let Some((a, b)) = crate::order::first_open_tie(date, &tasks, &spans) else {
        return Ok(Value::Null);
    };
    Ok(json!({
        "first_id": a.id,
        "first_title": a.title,
        "second_id": b.id,
        "second_title": b.title,
        "tool": "prefer_order",
    }))
}
```

3つの戻りへ `"order_tie": order_tie_json(store, date)?` を足す。`close_view` はこうなる。

```rust
Ok(json!({
    "closed": false,
    "open": open,
    "questions": questions,
    "order_tie": order_tie_json(store, date)?,
}))
```

`check_view` の JSON と `plan_view` の JSON にも同じキーを足す。`today_view` には足さない。

`src/tools.rs` の `list()` の末尾に足す。

```rust
tool(
    "prefer_order",
    "同順位の2件について、先にやるタイトルを覚える。",
    obj(
        &[
            ("first", schema("string", "先にやるタイトル")),
            ("second", schema("string", "あとでやるタイトル")),
        ],
        &["first", "second"],
    ),
),
```

`call` に足す。

```rust
"prefer_order" => {
    let first = req_str(args, "first")?;
    let second = req_str(args, "second")?;
    crate::graph::record_order(store, &first, &second)?;
    Ok(json!({ "first": first, "second": second }))
}
```

`print_jev_hints` は、既存の早期 return のあと、提案ループの次にこれを置く。`decider` はまだ生きている。`record_order` は呼ばない。同順位が無い、またはヒントが候補外なら何も出さない。

```rust
    let tasks = match crate::order::day_tasks(store, date) {
        Ok(tasks) => tasks,
        Err(_) => return,
    };
    let spans = crate::order::spans_from_events(&store.events_for_day(date).unwrap_or_default());
    if let Some((a, b)) = crate::order::first_open_tie(date, &tasks, &spans) {
        if let Some(choice) = crate::order::tie_hint(&mut decider, &a.title, &b.title) {
            let other = if choice == a.title { &b.title } else { &a.title };
            println!("  順番のヒント: 「{choice}」を先に。覚えるには prefer {choice} {other}");
        }
    }
```

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --lib order:: -- --nocapture`

Expected: PASS。

`cargo test --locked --offline --test growth` も通し、既存の Jev ヒント試験がヒント行以外で壊れていないことを見る。

- [x] **Step 5: Commit**

```bash
git add src/order.rs src/engine.rs src/rituals.rs src/tools.rs
git commit -m "feat: hint the first equal-rank pair without writing an edge"
```
