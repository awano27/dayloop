# 議事録・チケット・Teams の入口 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 議事録から決定事項を出し、閉じた集合で候補か否かを決める。チケットと Teams はフィクスチャ JSON から、既存の候補箱へ入れる。

**Architecture:** 抽出が文を出し、`classify` が `Action` / `Info` / `Ignore` を決める。既定の抽出は見出しと接頭辞。モデル抽出は同じ分類を通り、既定は off。候補になったあとは、既存の判断グラフと、完了証跡の `source_ref` に乗る。実サービスのログインは入れない。

**Tech Stack:** Rust, clap, serde_json。基準は `1db379706a833392a315bd153f43ca34aad55b88`。証跡計画のあとに入れる。

---

### Task 1: 議事録の行

**Files:**
- Create: `src/intake/minutes.rs`
- Modify: `src/intake/mod.rs`
- Test: `src/intake/minutes.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_split_action_info_and_noise() {
        let text = "決定: 見積を直す\n共有: 来週は休会\nこんにちは\nTODO：会場を押さえる\n";
        let lines = classify(text);
        assert_eq!(lines, vec![
            MinuteLine::Action("見積を直す".into()),
            MinuteLine::Info,
            MinuteLine::Ignore,
            MinuteLine::Action("会場を押さえる".into()),
        ]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --lib intake::minutes::tests -- --nocapture`

Expected: FAIL。モジュールが無い。

- [ ] **Step 3: Write minimal implementation**

接頭辞は行頭の `決定` `TODO` `アクション` `共有` `情報`。区切りは `:` または `：`。前後の空白は落とす。空の本文は `Ignore`。

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MinuteLine {
    Action(String),
    Info,
    Ignore,
}

pub fn classify(text: &str) -> Vec<MinuteLine> {
    text.lines().map(classify_line).collect()
}

fn classify_line(line: &str) -> MinuteLine {
    let line = line.trim();
    for (prefix, action) in [
        ("決定", true),
        ("TODO", true),
        ("アクション", true),
        ("共有", false),
        ("情報", false),
    ] {
        let Some(rest) = line.strip_prefix(prefix) else { continue };
        let rest = rest.trim_start();
        let body = if let Some(rest) = rest.strip_prefix(':') {
            rest.trim()
        } else if let Some(rest) = rest.strip_prefix('：') {
            rest.trim()
        } else {
            continue;
        };
        if body.is_empty() {
            return MinuteLine::Ignore;
        }
        return if action { MinuteLine::Action(body.to_string()) } else { MinuteLine::Info };
    }
    MinuteLine::Ignore
}
```

`src/intake/mod.rs` に `pub mod minutes;`。

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --lib intake::minutes::tests -- --nocapture`

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add src/intake/minutes.rs src/intake/mod.rs
git commit -m "feat: classify meeting notes into actions and shared lines"
```

---

### Task 2: 議事録を候補にする

**Files:**
- Modify: `src/intake/minutes.rs`
- Modify: `src/main.rs` の `IntakeCmd`
- Test: `tests/intake_minutes.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn actions_become_meeting_candidates_and_info_does_not() {
    let home = Home::new();
    let store = home.store();
    let text = "決定: 見積を直す\n共有: 来週は休会\n雑談\n";
    let report = dayloop::intake::minutes::ingest(&store, text).unwrap();
    assert_eq!(report.actions, 1);
    assert_eq!(report.info, 1);
    assert_eq!(report.ignored, 1);
    let cands = store.open_candidates().unwrap();
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].title, "見積を直す");
    assert_eq!(cands[0].source, "meeting");
    assert!(cands[0].source_ref.as_deref().unwrap().starts_with("minutes:"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test intake_minutes -- --nocapture`

Expected: FAIL。`ingest` が無い。

- [ ] **Step 3: Write minimal implementation**

```rust
pub struct MinuteReport { pub actions: usize, pub info: usize, pub ignored: usize }

pub fn ingest(store: &Store, text: &str) -> Result<MinuteReport> {
    let mut report = MinuteReport { actions: 0, info: 0, ignored: 0 };
    for (i, line) in classify(text).into_iter().enumerate() {
        match line {
            MinuteLine::Action(title) => {
                let source_ref = format!("minutes:{i}:{title}");
                if store.add_candidate(&title, "meeting", Some(&source_ref))?.is_some() {
                    report.actions += 1;
                }
            }
            MinuteLine::Info => report.info += 1,
            MinuteLine::Ignore => report.ignored += 1,
        }
    }
    Ok(report)
}
```

`IntakeCmd` に足す。

```rust
    /// 決定 / TODO / アクション の行を会議候補にする
    Note { file: std::path::PathBuf },
```

`Cmd::Intake` の match に足す。

```rust
IntakeCmd::Note { file } => {
    let text = std::fs::read_to_string(&file)?;
    let r = dayloop::intake::minutes::ingest(&store, &text)?;
    println!("議事録: 候補 {} 件、共有 {} 件、無視 {} 件", r.actions, r.info, r.ignored);
    0
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test intake_minutes -- --nocapture`

Expected: PASS。共有行は候補にならない。

- [ ] **Step 5: Commit**

```bash
git add src/intake/minutes.rs src/main.rs tests/intake_minutes.rs
git commit -m "feat: ingest marked meeting lines as meeting candidates"
```

---

### Task 3: チケットと Teams のフィクスチャ

**Files:**
- Create: `src/intake/tickets.rs`
- Create: `src/intake/teams.rs`
- Modify: `src/main.rs`
- Test: `tests/intake_fixtures.rs`

- [ ] **Step 1: Write the failing test**

チケット JSON:

```json
[{"source_ref":"ticket:ABC-1","title":"請求書を送る","state":"open"}]
```

Teams JSON:

```json
[{"source_ref":"teams:msg:1","title":"本番の確認をお願いします"}]
```

```rust
#[test]
fn ticket_and_teams_fixtures_become_candidates() {
    let home = Home::new();
    let store = home.store();
    dayloop::intake::tickets::ingest(&store, r#"[{"source_ref":"ticket:ABC-1","title":"請求書を送る","state":"open"}]"#).unwrap();
    dayloop::intake::teams::ingest(&store, r#"[{"source_ref":"teams:msg:1","title":"本番の確認をお願いします"}]"#).unwrap();
    let mut sources: Vec<_> = store.open_candidates().unwrap().into_iter().map(|c| c.source).collect();
    sources.sort();
    assert_eq!(sources, vec!["teams".to_string(), "ticket".to_string()]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test intake_fixtures -- --nocapture`

Expected: FAIL。

- [ ] **Step 3: Write minimal implementation**

`src/intake/tickets.rs`:

```rust
use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::store::Store;

#[derive(Deserialize)]
struct TicketRow {
    source_ref: String,
    title: String,
    state: String,
}

pub fn ingest(store: &Store, text: &str) -> Result<usize> {
    let rows: Vec<TicketRow> = serde_json::from_str(text)?;
    let mut n = 0;
    for row in rows {
        if row.title.trim().is_empty() || row.source_ref.trim().is_empty() {
            return Err(anyhow!("チケットの title と source_ref が必要です"));
        }
        match row.state.as_str() {
            "open" | "closed" => {}
            other => return Err(anyhow!("チケットの state が未知です: {other}")),
        }
        if store.add_candidate(row.title.trim(), "ticket", Some(&row.source_ref))?.is_some() {
            n += 1;
        }
    }
    Ok(n)
}
```

`state` は検証するだけで、候補の状態にはしない。閉じたチケットの完了は証跡計画の `observe::apply` が行う。

`src/intake/teams.rs`:

```rust
use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::store::Store;

#[derive(Deserialize)]
struct TeamsRow {
    source_ref: String,
    title: String,
}

pub fn ingest(store: &Store, text: &str) -> Result<usize> {
    let rows: Vec<TeamsRow> = serde_json::from_str(text)?;
    let mut n = 0;
    for row in rows {
        if row.title.trim().is_empty() || row.source_ref.trim().is_empty() {
            return Err(anyhow!("Teams の title と source_ref が必要です"));
        }
        if store.add_candidate(row.title.trim(), "teams", Some(&row.source_ref))?.is_some() {
            n += 1;
        }
    }
    Ok(n)
}
```

本文用のフィールドが JSON にあっても `TeamsRow` は読まない。

`src/intake/mod.rs` に `pub mod tickets;` と `pub mod teams;`。

`IntakeCmd` に足す。

```rust
    Tickets { file: std::path::PathBuf },
    Teams { file: std::path::PathBuf },
```

```rust
IntakeCmd::Tickets { file } => {
    let text = std::fs::read_to_string(&file)?;
    let n = dayloop::intake::tickets::ingest(&store, &text)?;
    println!("チケット候補: {n} 件");
    0
}
IntakeCmd::Teams { file } => {
    let text = std::fs::read_to_string(&file)?;
    let n = dayloop::intake::teams::ingest(&store, &text)?;
    println!("Teams 候補: {n} 件");
    0
}
```

閉じたチケットを候補採用したあとに証跡が `closed` を返すと `done` になる、という結合は次のテストで見る。

```rust
#[test]
fn accepted_ticket_closes_when_observation_is_closed() {
    let home = Home::new();
    let store = home.store();
    let date = "2026-09-22";
    dayloop::intake::tickets::ingest(&store, r#"[{"source_ref":"ticket:ABC-1","title":"請求書を送る","state":"closed"}]"#).unwrap();
    let c = store.open_candidates().unwrap().remove(0);
    store.accept_candidate(&c.id, Some(date)).unwrap();
    let map = dayloop::observe::parse_fixture(r#"[{"source_ref":"ticket:ABC-1","state":"closed"}]"#).unwrap();
    dayloop::observe::apply(&store, date, &map).unwrap();
    let tasks = store.tasks_for_day(date).unwrap();
    assert_eq!(tasks[0].state, dayloop::model::State::Done);
    assert_eq!(tasks[0].source_ref.as_deref(), Some("ticket:ABC-1"));
}
```

このテストは証跡計画が入っていることが前提。`observe::apply` がまだ無ければ、このテストだけを証跡計画のあとに移す。

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test intake_fixtures -- --nocapture`

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add src/intake/tickets.rs src/intake/teams.rs src/intake/mod.rs src/main.rs tests/intake_fixtures.rs
git commit -m "feat: ingest ticket and Teams fixtures into the candidate box"
```

---

### Task 4: 見出しから決定事項を出す

**Files:**
- Modify: `src/intake/minutes.rs`
- Test: `tests/intake_minutes.rs`

接頭辞の行は Task 1 の閉じた集合のまま。見出しの箇条書きは、その前段で決定事項の文にする。

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn decision_heading_becomes_a_candidate_and_share_heading_does_not() {
    let home = Home::new();
    let store = home.store();
    let text = "\
決定事項
- 見積を直す
共有事項
- 来週は休会
雑談
";
    let report = dayloop::intake::minutes::ingest(&store, text).unwrap();
    assert_eq!(report.actions, 1);
    assert_eq!(report.info, 1);
    let cands = store.open_candidates().unwrap();
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].title, "見積を直す");
    assert_eq!(cands[0].source, "meeting");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --test intake_minutes decision_heading_becomes_a_candidate_and_share_heading_does_not -- --nocapture`

Expected: FAIL。候補が 0 件。

- [ ] **Step 3: Write minimal implementation**

`read_minutes` を足し、`ingest` の `classify(text)` を `read_minutes(text)` に替える。

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section { None, Decision, Share }

pub fn read_minutes(text: &str) -> Vec<MinuteLine> {
    let mut section = Section::None;
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == "決定事項" || line == "# 決定事項" {
            section = Section::Decision;
            continue;
        }
        if line == "共有事項" || line == "# 共有事項" {
            section = Section::Share;
            continue;
        }
        if line.starts_with('#') {
            section = Section::None;
            continue;
        }
        match classify_line(line) {
            MinuteLine::Ignore => {}
            other => {
                out.push(other);
                continue;
            }
        }
        let Some(body) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) else {
            continue;
        };
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        match section {
            Section::Decision => out.push(MinuteLine::Action(body.to_string())),
            Section::Share => out.push(MinuteLine::Info),
            Section::None => out.push(MinuteLine::Ignore),
        }
    }
    out
}
```

Task 1 の接頭辞テストは `classify` のまま残す。`ingest` は `read_minutes` を通るので、`決定:` の行も今まで通り候補になる。

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --test intake_minutes -- --nocapture`

Expected: PASS。共有の箇条書きは候補にならない。

- [ ] **Step 5: Commit**

```bash
git add src/intake/minutes.rs tests/intake_minutes.rs
git commit -m "feat: turn decision headings into meeting candidates"
```

---

### Task 5: モデルの出力も同じ集合を通す

**Files:**
- Modify: `src/intake/minutes.rs`
- Modify: `src/config.rs`
- Test: `src/intake/minutes.rs`

- [ ] **Step 1: Write the failing test**

```rust
struct Script { lines: Vec<String> }

impl dayloop::intake::minutes::DecisionSource for Script {
    fn statements(&mut self, _text: &str) -> Vec<String> {
        self.lines.clone()
    }
}

#[test]
fn model_lines_outside_the_closed_set_are_dropped() {
    let mut source = Script {
        lines: vec!["決定: 見積を直す".into(), "来週よろしく".into()],
    };
    let lines = dayloop::intake::minutes::from_source(&mut source, "本文");
    assert_eq!(lines, vec![MinuteLine::Action("見積を直す".into())]);
}
```

このテストは `src/intake/minutes.rs` の `mod tests` に置く。

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --offline --lib intake::minutes::tests::model_lines_outside_the_closed_set_are_dropped -- --nocapture`

Expected: FAIL。`DecisionSource` が無い。

- [ ] **Step 3: Write minimal implementation**

```rust
pub trait DecisionSource {
    fn statements(&mut self, text: &str) -> Vec<String>;
}

pub fn from_source(source: &mut dyn DecisionSource, text: &str) -> Vec<MinuteLine> {
    source
        .statements(text)
        .iter()
        .filter_map(|line| match classify_line(line) {
            MinuteLine::Ignore => None,
            other => Some(other),
        })
        .collect()
}
```

`Config` に `#[serde(default)] pub minutes: MinutesConfig` を足す。既定は `generator = "rules"`。`DEFAULT_TOML` に次を足す。

```toml
[minutes]
generator = "rules"
```

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct MinutesConfig {
    #[serde(default = "default_minutes_generator")]
    pub generator: String,
}
fn default_minutes_generator() -> String { "rules".into() }
```

`generator` が `rules` のとき `ingest` は `read_minutes` を使う。`llm` のときは `from_source` の結果だけを候補にする。HTTP の実装は `HttpDecider` と同じく、route が空なら空の配列を返し、返った行は `from_source` で落とす。CI は `Script` だけを使う。実 API は呼ばない。

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --offline --lib intake::minutes:: -- --nocapture`

Expected: PASS。`来週よろしく` は候補にならない。

- [ ] **Step 5: Commit**

```bash
git add src/intake/minutes.rs src/config.rs
git commit -m "feat: filter generated meeting lines through the closed set"
```
