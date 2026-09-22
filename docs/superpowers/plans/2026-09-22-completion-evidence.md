# 完了の証跡 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** チケットが closed、PR が merged、会議準備の予定が終了時刻を過ぎている、のどれかだけを質問なしで `done` にする。

**Architecture:** チケットと PR は `source_ref` を鍵にフィクスチャ JSON から状態を引く。会議は `outlook:cal:<id>:prep` を台帳の予定と突き合わせる。空のフィクスチャはチケットと PR を Unknown のままにする。適用は `transition_silent` で、判断グラフの枝は生やさない。`decided_by` は `observe:<source_ref>`。

**Tech Stack:** Rust, serde_json, chrono。基準は `1db379706a833392a315bd153f43ca34aad55b88`。GitHub と Jira の実 API は入れない。

---

### Task 1: フィクスチャの読み取り

**Files:**
- Create: `src/observe.rs`
- Modify: `src/lib.rs`
- Test: `src/observe.rs`

- [x] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_and_merged_are_closed() {
        let text = r#"[
            {"source_ref":"ticket:ABC-1","state":"closed"},
            {"source_ref":"github:pr:dayloop#7","state":"merged"},
            {"source_ref":"ticket:ABC-2","state":"open"}
        ]"#;
        let map = parse_fixture(text).unwrap();
        assert_eq!(map.get("ticket:ABC-1"), Some(&Sight::Closed));
        assert_eq!(map.get("github:pr:dayloop#7"), Some(&Sight::Closed));
        assert_eq!(map.get("ticket:ABC-2"), Some(&Sight::Open));
    }

    #[test]
    fn unknown_state_is_rejected() {
        let text = r#"[{"source_ref":"ticket:ABC-3","state":"done"}]"#;
        assert!(parse_fixture(text).is_err());
    }
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --lib observe::tests -- --nocapture`

Expected: FAIL。モジュールが無い。

- [x] **Step 3: Write minimal implementation**

`src/lib.rs` に `pub mod observe;`。

```rust
use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sight {
    Closed,
    Open,
}

#[derive(Deserialize)]
struct Row {
    source_ref: String,
    state: String,
}

pub fn parse_fixture(text: &str) -> Result<BTreeMap<String, Sight>> {
    let rows: Vec<Row> = serde_json::from_str(text)?;
    let mut map = BTreeMap::new();
    for row in rows {
        let sight = match row.state.as_str() {
            "closed" | "merged" => Sight::Closed,
            "open" => Sight::Open,
            other => return Err(anyhow!("未知の観測状態: {other}")),
        };
        map.insert(row.source_ref, sight);
    }
    Ok(map)
}

pub fn lookup(map: &BTreeMap<String, Sight>, source_ref: Option<&str>) -> Option<Sight> {
    source_ref.and_then(|key| map.get(key).copied())
}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --lib observe::tests -- --nocapture`

Expected: PASS。

- [x] **Step 5: Commit**

```bash
git add src/observe.rs src/lib.rs
git commit -m "feat: parse closed and merged observations from a fixture"
```

---

### Task 2: 閉じた観測だけ silent に完了する

**Files:**
- Modify: `src/observe.rs`
- Modify: `src/store.rs`（`decided_by` を `observe:` で書く関数）
- Test: `tests/observe_apply.rs`

- [x] **Step 1: Write the failing test**

`Home` は `tests/order_pref.rs` と同じ形。ロック名は `OBSERVE_LOCK`。

```rust
#[test]
fn merged_ref_becomes_done_without_a_graph_edge() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    let t = store.add_task("PRを出す", Some(date), None, "ticket", Some("github:pr:dayloop#7"), Some(date)).unwrap();
    let open = store.add_task("まだ開いている", Some(date), None, "ticket", Some("ticket:ABC-2"), Some(date)).unwrap();
    let none = store.add_task("参照なし", Some(date), None, "manual", None, Some(date)).unwrap();
    let map = observe::parse_fixture(r#"[
        {"source_ref":"github:pr:dayloop#7","state":"merged"},
        {"source_ref":"ticket:ABC-2","state":"open"}
    ]"#).unwrap();
    let n = observe::apply(&store, date, &map).unwrap();
    assert_eq!(n, 1);
    let done = store.get_task(&t.id).unwrap();
    assert_eq!(done.state, State::Done);
    assert_eq!(done.evidence.as_deref(), Some("observe:github:pr:dayloop#7"));
    assert_eq!(done.decided_by.as_deref(), Some("observe:github:pr:dayloop#7"));
    assert_eq!(store.get_task(&open.id).unwrap().state, State::Planned);
    assert_eq!(store.get_task(&none.id).unwrap().state, State::Planned);
    let sit = graph::Situation::from_task(&done, "close");
    assert!(graph::resolve(&store, &sit).unwrap().is_none());
}
```

同じタイトルの別タスクを翌日に足し、`apply_known_tasks` がそれを `done` にしないことも、このテストの最後で確認する。

```rust
    let again = store.add_task("PRを出す", Some("2026-09-23"), None, "manual", None, Some("2026-09-23")).unwrap();
    assert_eq!(graph::apply_known_tasks(&store, "2026-09-23").unwrap(), 0);
    assert_eq!(store.get_task(&again.id).unwrap().state, State::Planned);
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test observe_apply -- --nocapture`

Expected: FAIL。`observe::apply` が無い。

- [x] **Step 3: Write minimal implementation**

`src/store.rs` に `mark_decided` の隣へ足す。`mark_decided` は `graph:` を付けるので使わない。

```rust
pub(crate) fn mark_observed(&self, id: &str, source_ref: &str) -> Result<()> {
    let marker = format!("observe:{source_ref}");
    self.conn.execute(
        "UPDATE tasks SET decided_by=?2, evidence=?3 WHERE id=?1",
        params![id, marker, marker],
    )?;
    Ok(())
}
```

`transition_silent` が先に `evidence` を書くので、`mark_observed` は完了のあとに呼ぶ。

`src/observe.rs`:

```rust
pub fn apply(store: &Store, date: &str, map: &BTreeMap<String, Sight>) -> Result<usize> {
    let mut n = 0;
    for t in store.open_tasks_for_day(date)? {
        if lookup(map, t.source_ref.as_deref()) != Some(Sight::Closed) {
            continue;
        }
        let key = t.source_ref.clone().unwrap_or_default();
        store.transition_silent(&t.id, crate::model::State::Done, None, Some(&format!("observe:{key}")))?;
        store.mark_observed(&t.id, &key)?;
        n += 1;
    }
    Ok(n)
}
```

`Store` を import する。`transition_silent` は `pub(crate)` なので、同じクレートの `observe` から呼べる。

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test observe_apply -- --nocapture`

Expected: PASS。枝は無く、参照の無いタスクは planned のまま。

- [x] **Step 5: Commit**

```bash
git add src/observe.rs src/store.rs tests/observe_apply.rs
git commit -m "feat: mark a task done only when its source ref is closed"
```

---

### Task 3: 儀式と MCP の前に適用する

**Files:**
- Modify: `src/config.rs`
- Modify: `src/rituals.rs`
- Modify: `src/tools.rs`
- Test: `tests/observe_apply.rs`

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn empty_config_observes_nothing() {
    let cfg = dayloop::config::Config::default();
    assert!(cfg.observe.fixture.is_empty());
    assert!(observe::load_map(&cfg.observe).unwrap().is_empty());
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test observe_apply empty_config_observes_nothing -- --nocapture`

Expected: FAIL。`Config.observe` が無い。

- [x] **Step 3: Write minimal implementation**

`Config` に `#[serde(default)] pub observe: ObserveConfig`。既定は `fixture = ""`。`DEFAULT_TOML` に次を足す。

```toml
[observe]
fixture = ""
```

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct ObserveConfig {
    #[serde(default)]
    pub fixture: String,
}
impl Default for ObserveConfig {
    fn default() -> Self { Self { fixture: String::new() } }
}
```

`observe::load_map` は fixture が空なら空マップ、パスがあればそのファイルを `parse_fixture` する。ファイルが読めなければエラーを返し、タスクは変更しない。

`plan_ritual`、`check_ritual`、`close_ritual` で `graph::apply_known_*` の直前に `observe::apply(store, date, &observe::load_map(&config::load().observe)?)?` を置く。

`tools.rs` の `plan_day`、`check_in`、`close_day` も、ビューを作る前に同じ順で呼ぶ。`get_today` では呼ばない。

MCP 側の枝適用そのものは `2026-09-22-mcp-known-edges.md` の仕事である。このタスクでは観測の適用だけを足す。まだ枝適用が MCP に無いなら、観測だけをビューの前に置く。

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test observe_apply -- --nocapture`

Expected: PASS。

`cargo test --locked --offline --test growth` で、フィクスチャ未設定の既存日次が完了に変わらないことを確認する。

- [x] **Step 5: Commit**

```bash
git add src/config.rs src/observe.rs src/rituals.rs src/tools.rs tests/observe_apply.rs
git commit -m "feat: apply source observations before the daily questions"
```

---

### Task 4: 終わった会議

**Files:**
- Modify: `src/observe.rs`
- Modify: `src/rituals.rs`
- Modify: `src/tools.rs`
- Test: `tests/observe_apply.rs`

会議準備の `source_ref` は、既存の `event_prep_hit` が付ける `outlook:cal:<entry_id>:prep` である。終了は台帳の `events.end` を読む。フィクスチャには書かない。

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn finished_meeting_prep_is_done_and_a_future_one_is_not() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    store.upsert_events(date, vec![dayloop::model::Event {
        entry_id: "cal-1".into(),
        date: date.into(),
        start: "2026-09-22T10:00:00+09:00".into(),
        end: "2026-09-22T11:00:00+09:00".into(),
        subject: "週次".into(),
        location: None,
        organizer: None,
        is_organizer: true,
        source: "outlook".into(),
        synced_at: "2026-09-22T09:00:00+09:00".into(),
    }]).unwrap();
    let prep = store.add_task(
        "会議準備: 週次",
        Some(date),
        Some(30),
        "outlook",
        Some("outlook:cal:cal-1:prep"),
        Some(date),
    ).unwrap();
    let early = observe::apply_day(&store, date, &std::collections::BTreeMap::new(), "10:30").unwrap();
    assert_eq!(early, 0);
    assert_eq!(store.get_task(&prep.id).unwrap().state, State::Planned);
    let late = observe::apply_day(&store, date, &std::collections::BTreeMap::new(), "12:00").unwrap();
    assert_eq!(late, 1);
    let done = store.get_task(&prep.id).unwrap();
    assert_eq!(done.state, State::Done);
    assert_eq!(done.decided_by.as_deref(), Some("observe:outlook:cal:cal-1:prep"));
    let sit = graph::Situation::from_task(&done, "close");
    assert!(graph::resolve(&store, &sit).unwrap().is_none());
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test observe_apply finished_meeting_prep_is_done_and_a_future_one_is_not -- --nocapture`

Expected: FAIL。`apply_day` が無い。

- [x] **Step 3: Write minimal implementation**

`src/observe.rs` に足す。終了時刻が読めない予定は完了にしない。

```rust
pub fn meeting_ended(source_ref: &str, events: &[crate::model::Event], now_hhmm: &str) -> bool {
    let Some(rest) = source_ref.strip_prefix("outlook:cal:") else { return false };
    let Some(id) = rest.strip_suffix(":prep") else { return false };
    let Some((_, now)) = crate::facts::span("00:00", now_hhmm) else { return false };
    events.iter().any(|event| {
        if event.entry_id != id {
            return false;
        }
        match crate::facts::span("00:00", &event.end) {
            Some((_, end)) => end <= now,
            None => false,
        }
    })
}

pub fn apply_day(
    store: &Store,
    date: &str,
    map: &BTreeMap<String, Sight>,
    now_hhmm: &str,
) -> Result<usize> {
    let mut n = apply(store, date, map)?;
    let events = store.events_for_day(date)?;
    for t in store.open_tasks_for_day(date)? {
        let Some(source_ref) = t.source_ref.as_deref() else { continue };
        if !meeting_ended(source_ref, &events, now_hhmm) {
            continue;
        }
        let marker = format!("observe:{source_ref}");
        store.transition_silent(&t.id, crate::model::State::Done, None, Some(&marker))?;
        store.mark_observed(&t.id, source_ref)?;
        n += 1;
    }
    Ok(n)
}
```

`plan_ritual`、`check_ritual`、`close_ritual` と、`tools.rs` の `plan_day`、`check_in`、`close_day` は、Task 3 の `observe::apply` を `observe::apply_day` に替える。時刻は `chrono::Local::now().format("%H:%M")` の文字列。テストは時刻を引数で渡すので、儀式だけが現在時刻を使う。

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test observe_apply -- --nocapture`

Expected: PASS。10:30 では planned のまま、12:00 で done。枝は無い。

- [x] **Step 5: Commit**

```bash
git add src/observe.rs src/rituals.rs src/tools.rs tests/observe_apply.rs
git commit -m "feat: complete meeting prep after the stored event has ended"
```
