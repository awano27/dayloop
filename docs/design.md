# dayloop 設計書

要件は [requirements.md](requirements.md) に書いた。本書は、その要件を今のプロセスとデータの形に対応させる。

## 1. 全体

```mermaid
flowchart TD
  sources["入口"]
  inbox["候補"]
  graph{"判断グラフに枝がある?"}
  jev["Jev keep または skip"]
  ledger["台帳"]
  day{"その日の未確定はゼロ?"}

  sources --> inbox --> graph
  graph -->|ある| ledger
  graph -->|ない| jev
  jev -->|keep かつ確度が足りる| ledger
  jev -->|skip または呼べない| inbox
  ledger --> day
  day -->|残る| sources
  day -->|ゼロ| shut["日を閉じる"]
```

入口は届いたときだけ動く。台帳への書き込みは、人の回答、既知の枝、証跡による完了、Jev の keep のいずれかだけである。

## 2. プロセス

単一の Windows 実行ファイルである。状態は SQLite 1ファイルに置く。場所は `DAYLOOP_HOME`、無ければ `%LOCALAPPDATA%\dayloop` である。

| 経路 | 役割 |
|---|---|
| CLI | `plan` `next` `check` `close` `retro` と、個別の `add` `done` `carry` `revise` |
| MCP | stdout は改行区切りの JSON-RPC。ツールは台帳操作と `sync_sources` |
| `serve` | 1分ごとに時刻を見て、平日の朝昼夕と金曜の振り返りを非対話で実行する |
| ブラウザ | `intake browse` のときだけ Edge を開く。朝の `plan` では開かない |

`plan`、`check`、`close`、`next` の前に `intake::link` が走る。Outlook、トークンがある GitHub、Jira、Teams、組織名がある DevOps API、`inbox` フォルダを順に見る。ブラウザはここには含まれない。

## 3. 台帳

`tasks` は予定と状態を持つ。状態は backlog、planned、in_progress、done、not_done、carried、dropped である。`days` は計画の確定時刻と日の終了時刻を持つ。`candidates` は入口が置いた候補で、採用か却下まで残る。`rejected_refs` は同じ `source_ref` を再提示しない。`events` はその日の予定である。

日を閉じる処理は、その日の planned と in_progress が0件のときだけ `closed_at` を書く。持ち越しは元のタスクを carried にし、次の営業日に新しいタスクを作る。`carried_count` が3のオープンなタスクは、期限変更なしの再持ち越しを拒否する。

人の回答と `revise` は判断グラフへ枝を書く。枝のキーは件名、状況、送信者の順に具体から粗い。適用時もその順で探す。古い枝は残し、有効な枝だけを使う。

タスクの並びは `order` が決める。`next` は証跡の適用と既知の枝の適用のあと、残ったオープンな先頭を返す。

## 4. 入口

すべて候補の `source` と `source_ref` に落ちる。同じ `source_ref` は一度だけ入れる。

| 実装 | 読み方 | 候補の source |
|---|---|---|
| `intake/minutes.rs` と `intake/recap.rs` | 渡した文章、または `inbox` の txt、md、html。アクションの節だけ | meeting |
| `browse.rs` | Edge の別プロファイル。Gmail、Outlook on the web、Teams、DevOps Boards | mail、teams、ticket |
| `outlook_com.rs` | クラシック Outlook の受信トレイと予定 | outlook |
| `graph_office.rs` | COM が失敗し、Graph のトークンがあるときのメールと予定 | mail、meeting |
| `github.rs` | 割り当てられた Issue と PR | github |
| `jira.rs` | 割り当てられた未完了 | ticket |
| `devops.rs` | WIQL で `@Me` の作業項目 | ticket |
| `chat.rs` | Graph の直近チャット | teams |
| `screen.rs` | 前面の窓は Windows の API で特定する。本文は `ui_read` だけを許可した実行ファイルで、要素を指定せず読む | outlook、teams |

ブラウザのプロファイルは `%LOCALAPPDATA%\dayloop\browser` である。Edge はコマンド終了後も残るよう、ジョブから切り離して起動する。接続はローカルのリモートデバッグポート 9333 である。一覧は文書と iframe の `tr.zA`、`role=option`、`role=listitem`、`role=row` を見る。

開く件数は最大5件である。開いた件名と本文の先頭を Jev に渡し、選択肢は `keep` と `skip` だけである。言語選択画面は3つ以上の言語名で判定し、候補にしない。Jev が呼べないときは、設定のキーワードが本文か件名にあるものだけを残す。

DevOps のブラウザ URL は `https://dev.azure.com/pcedx/pcedx-1/_workitems/recentlyupdated/` である。API の組織名とは別に持つ。

作業の枝は `graph_nodes.kind = step` である。台帳を開くと、無いものだけ書く。題名だけの画面は取得失敗、それ以外の窓も取得失敗、Boards のツールバーは題名から外す、GitHub は `gh` のログインを使う、DevOps API の 401 は止める、画面の本文は送らない、言語の選択画面は候補にしない。人が `dayloop steps set` で替えると、種まきは上書きしない。同じブラウザの件名は `browse:サイト:件名` の枝になり、2回目は Jev を呼ばない。確度が下限未満の答えは枝にしない。同じ画面の本文は `capture:text:` にハッシュを付けた枝 `reuse` になり、次の取り込みは保存済みの評価を使う。

`dayloop capture` は `intake::link` から呼ばない。読むのは前面の1窓で、プロセスが Outlook か Teams のときだけ本文を取る。それ以外の窓は本文を読まず、取得失敗として残す。空の文章と、ボタン名だけの文章も取得失敗である。依頼が無かった、とは書かない。実行ファイルは `wincli` か `Sbroenne.WindowsMcp.exe` で、後者は `--tools ui_read` だけで起動する。クリック、入力、送信のツールは渡さない。

取れた本文は `screen_captures` に置く。`screen_evals` は設問版 `screen-v1`、モデル名、送ったかどうか、5つの選択、確度、理由コード、原文の引用を1行で持つ。`screen_decisions` は採用、修正、保留を追記する。修正しても評価の行は消さない。候補の `source_ref` は `screen:` に記録の id を付けたもので、画面の要素 id ではない。Jev への送信は `--send` のときだけである。鍵が無い、呼び出しに失敗したときは評価を保留し、本文は手元に残す。

## 5. 証跡による完了

`observe` が、オープンなタスクの `source_ref` を見て完了にする。

| source_ref | 完了になる条件 |
|---|---|
| `github:pr:owner/repo#番号` | `merged` が true |
| `github:issue:owner/repo#番号` | state が closed |
| `jira:KEY` | statusCategory が done |
| `devops:組織/プロジェクト#id` | 状態が Closed、Done、Completed、Removed、Resolved |
| `outlook:cal:` または `graph:cal:` で終わる準備 | 対応する予定の終了時刻を過ぎている |

鍵が無い、API が失敗した、状態が open のときは何もしない。

## 6. Jev

`HttpDecider` が TypeSafe の system-one へ POST する。質問は `pick` 1つで、選択肢は criteria のキーである。応答の `answers.pick.choice` と `confidence` を読む。

ブラウザでは確度 0.5 以上の `keep` だけを候補にする。台帳の未知の節では、閉じた選択肢かつ確度が下限以上のときだけ枝と状態を書く。下限が空なら 0.5 を使う。テスト中はネットワークへ出ない。

## 7. MCP と常駐

MCP のツールは、取得、計画、確定、追加、状態変更、分割、候補、Markdown、取り込みである。質問は `options[].tool` に解決先のツール名を1つ持つ。`close_day` は未確定が残ると閉じない。

常駐は `serve-state.json` にその日の実行を記録する。通知はトースト、失敗時は `notify.txt` である。ログは `serve.log` で、10MB を超えると1世代残して回す。

## 8. 失敗の扱い

| 状況 | 動作 |
|---|---|
| 入口の鍵が無い | 1行出して終了コード 0。他の入口は続ける |
| ブラウザの一覧が空 | 窓を残して終了コード 0。候補は作らない |
| サイト名が不正 | 終了コード 1 |
| Jev が呼べない | ブラウザはキーワード判定に落とす。台帳の未知の節は人に残す |
| 持ち越し上限 | 例外にせず、分割、取り下げ、期限変更の質問を返す |

## 9. 今の限界

- ブラウザの作業項目は、ツールバーの文言を題名から外して保存する
- デスクトップの Outlook と Teams は、窓全体と子要素の名前を読む。本文が題名だけなら取得失敗になる。依頼を開いていない画面では本文は返らない
- GitHub は `gh` のログインで読める。この PC の割り当ては 0 件だった。Graph と Jira は資格情報が無い。DevOps API は `az` の資格情報で 401 になる
- 確度 0.5 は20件の測定結果ではない
