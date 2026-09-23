# 要件ギャップの追加開発 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `docs/analysis/06-requirements-gap.md` の未実装を、動く順に5本の計画へ割る。

**Architecture:** 台帳が状態を書く。判断グラフが既知の節を質問なしで進める。Jev の出力だけでは枝も台帳も書かない。順番は Rust が決め、同順位の2択だけを Jev に渡す。完了を自動で付けるのは、チケットが閉じている、PR がマージされている、会議の終了時刻を過ぎている、のどれかを `source_ref` から読めたときだけ。

**見直し:** `docs/analysis/06-requirements-gap.md` を読み直して、次の3点を計画に戻した。会議終了も完了の証跡に入れる。議事録は、決定事項を出す層と、候補か否かを閉じた集合で決める層の両方を持つ。帯の下限が設定に無いあいだは未知の節を台帳へ書かず、下限があるときだけその帯以上を silent に適用する。

**Tech Stack:** Rust, rusqlite, chrono, clap, serde_json。既存の `cargo test --locked --offline`。

**基準コミット:** `cursor/dayloop-roadmap-f0c5` の `1db379706a833392a315bd153f43ca34aad55b88`（`docs: compare the daily requirement with what the code does today`）。この計画のコードはそこから枝を切る。

この作業ツリーの `codex/daily-workflow-beta` には `src/graph.rs` も `src/facts.rs` も無い。beta の7カテゴリ台帳へ、この差分を載せない。

要件の原文は基準コミットの `docs/analysis/06-requirements-gap.md`。

---

## すでに満たしているもの

| 要件 | 扱い |
| --- | --- |
| 翌日は前日の積み残しから確認する | 実装済み。`plan` が前日の未クローズを先に閉じ、open が残るあいだ `confirm_plan` は失敗する。この計画では触らない |
| 処分（持ち越し、取り下げ、棚、着手）は2回目から自動 | 実装済み。`graph::apply_known_tasks` / `apply_known_candidates` が CLI の儀式で動く |
| Jev が第一判断をしない | 意図した穴。`docs/jev-eval.md` の20件を測る前に、未知の節を台帳へ書かない |

## ファイル構成

| 計画 | 新しい責務 | 主なファイル |
| --- | --- | --- |
| [順番](2026-09-22-task-order.md) | 今日の並びと、同順位の2択 | `src/order.rs`、`src/engine.rs`、`src/graph.rs`、`src/main.rs` |
| [完了の証跡](2026-09-22-completion-evidence.md) | チケット、PR、終わった会議を `source_ref` から読み、そのときだけ silent に `done` | `src/observe.rs`、`src/config.rs` |
| [入口](2026-09-22-intake-sources.md) | 議事録は決定事項の抽出と閉じた分類。チケットと Teams はフィクスチャ | `src/intake/minutes.rs`、`tickets.rs`、`teams.rs` |
| [MCP の枝](2026-09-22-mcp-known-edges.md) | MCP の `plan_day` / `check_in` / `close_day` がビューの前に枝を適用する | `src/tools.rs` |
| [帯](2026-09-22-jev-band.md) | 20件の帯を数える。下限が空なら未知の節は書かない | `src/jev_eval.rs`、`src/jev.rs`、`src/main.rs` |

順番、証跡、入口、帯はこの順に積む。証跡の読み取りが無いと、チケットを候補にしただけでは完了にできない。MCP の枝は他の4本と依存がない。先に入れても後に入れてもよい。帯は最後に置き、既定の設定では未知の節を書かない。

## 固定した判断

1. 並びのキーは、未完了を先に、その中で期限超過、今日、期限なし、未来。同じ期限区分では持ち越し回数の多い順。その中では空きに収まるもの、空き不明、空きに収まらないもの。その中では見積の短い順。最後は `created_at`、同着は `id`。
2. 空きは `facts::fit_estimate` のまま。予定が無い日は空き 0 分にしない。
3. 同順位が隣接した最初の1組だけを Jev の2択にする。要件 2 が Jev の返答だけでは枝を作らないと決めているので、この2択もヒントに留める。枝になるのは `dayloop prefer <先> <後>` だけ。枝はランクをひっくり返さず、`created_at` の同着だけを入れ替える。
4. 完了の観測は3種類。`ticket:<キー>` が `closed`、`github:pr:<owner>/<repo>#<番号>` が `merged`、`outlook:cal:<id>:prep` の予定の終了時刻を過ぎている。チケットと PR の状態はフィクスチャ JSON から引く。会議の終了は台帳に既にある予定を読む。GitHub と Jira の実 API は、試験アカウントが無いのでこの5本には入れない。
5. 議事録は2層。先に決定事項の文を出し、そのあと閉じた集合で候補か否かを決める。既定の抽出は、`決定事項` 見出しの箇条書きと、`決定:` / `TODO:` / `アクション:` / `共有:` / `情報:` の行。`共有` と `情報` は候補にしない。モデル抽出は同じ関数のうしろに置き、既定は off。モデルの出力も同じ閉じた集合を通り、外れた行は捨てる。
6. `engine::plan_view` / `check_view` / `close_view` は読み取りのままにする。要件が名指ししている穴は、MCP がこのビューをそのまま返すことなので、適用は `src/tools.rs` の `plan_day` / `check_in` / `close_day` で、ビューを作る前に行う。`get_today` は適用しない。
7. `commit_confidence` が空のあいだ、未知の節は画面のヒントだけにする。値が入っているときだけ、その値以上の Choice を `revise_task` と同じ経路で台帳と枝へ書く。帯の計画は既定の設定ファイルへ数値を書かない。

## カバレッジ

| 要件文書 | 計画 |
| --- | --- |
| 1. 入口に議事録とチケットが無い。議事録は生成と閉じた集合 | 入口。Teams はフィクスチャまで |
| 2. 第一判断を Jev に渡す穴。帯のあとだけ silent 適用 | 帯。既定の `commit_confidence` は空 |
| 3. チケット、PR、終わった会議だけ完了を silent にする | 完了の証跡 |
| 4. 順番と段取りが無い | 順番 |
| 5. 翌日の積み直し | 変更なし |
| 差を埋める順 4. MCP が枝を通らない | MCP の枝 |

## 実行

各ファイルのタスクは、そのファイルだけでテストが通る。基準コミットから `codex/requirements-gap` を切り、上の表の順にコミットする。MCP の枝だけは独立なので、順番の前に入れてもよい。
