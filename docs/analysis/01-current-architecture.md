# dayloop 現状分析（段階3）

対象コミット: `2cf1434`（`stage 3: Outlook COM intake, extraction rules, events, sync_sources`）
バージョン: `0.1.0`（Rust 2021、単一バイナリ）

## プロダクトの約束

1日を **計画 → 途中確認 → 確定 → 振り返り** で閉じる。
「できた / できていない / 忘れている」は LLM の記憶ではなく、SQLite 台帳の不変条件で保証する。
LLM は無くても動く。データ書き込み先は `%LOCALAPPDATA%\dayloop`（`DAYLOOP_HOME` で変更可）。管理者権限は不要。

## モジュール

| モジュール | 責任 |
|---|---|
| `src/model.rs` | 状態・タスク・日・候補・予定の型 |
| `src/store.rs` | SQLite。不変条件の書き込み側 |
| `src/engine.rs` | 副作用なしのスナップショットと `questions` |
| `src/rituals.rs` | CLI の対話ループ |
| `src/tools.rs` / `src/mcp.rs` | MCP ツールと stdio JSON-RPC（1行1メッセージ） |
| `src/serve.rs` | 1分間隔の定時実行。未実行フェーズを復帰後に1回 |
| `src/intake/` | `Source` トレイト。Outlook COM とフィクスチャ |
| `src/markdown.rs` | 当日 Markdown の書き出しと手編集の取り込み |
| `src/doctor.rs` | ロックダウン PC で何が使えるかの読み取り診断 |
| `src/startup.rs` / `src/notify.rs` | HKCU Run とトースト（失敗時 `notify.txt`） |

終了コードは 0 完了 / 1 エラー / 2 本人の回答待ち。`serve` は `--yes` 相当（`Ui::new(true)`）で動き、未回答は通知してその日の再実行はしない。

## 状態

`backlog` → `planned` → `in_progress` → `done` / `not_done` / `carried` / `dropped`

- open は `planned` と `in_progress` のみ。日を閉じる条件は open が 0 件。
- 持ち越しは元タスクを `carried` にし、次の営業日へ新しい `planned` を作る。`carried_count` を引き継ぐ。
- 3回到達後の再持ち越しは `CarryBlocked`。解消は分割・取り下げ・期限変更（期限変更は回数を 0 に戻す）。
- 候補は `open` のまま残る。同じ `source_ref` の却下は `rejected_refs` で再提示しない。7日で強調。

## 不変条件と実装箇所

| # | 内容 | 強制している場所 |
|---|---|---|
| 1 | open が残る日は閉じない | `Store::close_day` |
| 2 | 未完了・持ち越し・取り下げは理由必須 | `Store::transition` / `carry_over` |
| 3 | 3回持ち越しは分割・取り下げ・期限変更まで再持ち越し不可 | `Store::carry_over` |
| 4 | 候補は採用か却下まで消えない。7日で強調 | `add_candidate` の重複拒否、`engine` の stale 表示 |
| 5 | 前日が未クローズなら朝の計画は前日の確定から | CLI の `plan_ritual` は前日 `close_ritual` が終わるまで今日の確定に進まない |

テストは `tests/invariants.rs` が 1〜5 を各1本。取り込みは `tests/intake_rules.rs` と `tests/intake_pipeline.rs`。MCP は `tests/mcp_stdio.rs`。

## 1日の入口

- CLI: `plan` / `check` / `close` / `retro` と個別操作（`add` `start` `done` `carry` `split` など）
- MCP: `get_today` `plan_day` `confirm_plan` `check_in` `close_day` `retro_week` ほか。`questions[].options[]` は `{label, tool, args, needs}`。`tool: null` は何もしない
- 定時: 既定 plan 08:30 / check 13:00 / close 18:00 / retro `Fri 18:30`、平日のみ。plan と check の前に Outlook 取り込み。失敗はログのみ
- Markdown: `%LOCALAPPDATA%\dayloop\days\YYYY-MM-DD.md`。`- [x]` は完了、新しい `- [ ]` は追加

## 取り込み

`Source` はメールと予定を返す。ルールは先勝ち。

1. フラグ付き → 候補
2. キーワード（件名、`read_body` のとき本文先頭） → 候補
3. 未読かつ `important_senders` → 候補（空リストならこの規則は無効）
4. 24時間以内の会議で、主催者または出欠必須（設定による） → 会議準備の候補

予定そのものは `events` テーブルへその日分を置き換える。本文とメールアドレスは既定で読まない。COM は `windows` クレートの IDispatch。PowerShell は使わない。

`docs/decisions.md` にある通り、クラシック Outlook の実機 COM は未検証。開発時に確認できたのはコンパイルと、ProgID が無いときの終了コード 0。新しい Outlook（`olk.exe`）は段階4の Edge 経由、と `doctor` が案内する。

## ロードマップ上の位置

README は段階3まで実装済みとし、その後を VS Code 拡張、会議・アラート・勤怠、M365 Copilot の順にしている。
`DAYLOOP_LLM_ENDPOINT` は doctor が疎通を見るだけで、呼び出し実装は無い。
