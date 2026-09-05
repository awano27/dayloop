# dayloop

**What.** A local CLI that runs a daily plan → check → close → retro loop for tasks.
**Why.** Zero-forgetting is enforced by ledger invariants, not by an LLM.
**Requirements.** Windows 10/11; no admin rights. Classic Outlook is optional (COM intake).

Repository: https://github.com/OWNER/dayloop

1日のタスクを **計画 → 途中確認 → 確定 → 振り返り** で回す CLI。
「できたのか・できていないのか・忘れているものは無いか」を、LLM ではなく台帳の不変条件で保証します。

- 管理者権限なしで動く単一 exe（書き込み先は `%LOCALAPPDATA%\dayloop` のみ）
- LLM は M365 Copilot / GitHub Copilot / ローカル LLM のどれでも。無くても動く
- 当日分は Markdown にも書き出され、手で編集して取り込める

## 不変条件（コードで強制）

1. 今日のタスクが全件「完了・未完了・持ち越し・取り下げ」になるまで、その日は閉じられない
2. 未完了・持ち越し・取り下げには理由が必須
3. 3回持ち越したタスクは、分割・取り下げ・期限変更のどれかを選ぶまで再持ち越しできない
4. 外部から拾った候補は、採用か却下するまで消えない（7日放置で強調表示）
5. 前日を閉じていなければ、翌朝の計画は前日の確定から始まる

## 1日の流れ

```bash
dayloop plan     # 朝: 前日の未確定 → 候補 → 未計画 → 今日の予定を確定
dayloop check    # 昼: 未着手タスクをどうするか
dayloop close    # 夕: 全件確定して日を閉じる（未確定が残ると閉じない）
dayloop retro    # 週末: 完了率・持ち越し上位・メモ
```

途中の操作:

```bash
dayloop add "仕様書レビュー" --due 2026-09-05
dayloop add "いつかやる調査" --backlog
dayloop start 4CFD6BRC
dayloop done 4CFD6BRC --evidence https://github.com/org/repo/pull/12
dayloop notdone 4CFD6BRC --reason "レビュー待ち"
dayloop carry 4CFD6BRC --reason "領収書未着"          # 次の営業日へ
dayloop carry 4CFD6BRC --reason "期限見直し" --reschedule 2026-09-10
dayloop split 4CFD6BRC --reason "大きすぎる" --into "領収書を集める" --into "申請入力"
dayloop drop 4CFD6BRC --reason "不要になった"
dayloop candidates add "田中さんに返信" --source teams --ref teams:msg:123
dayloop today
dayloop doctor   # この PC で何が使えるか（exe 制御・Outlook COM・Edge・schtasks・ローカル LLM）
```

ID は表示されている末尾8文字で指定できます。

## 台帳の診断・訂正・バックアップ

```bash
dayloop ledger check --json
dayloop backup
dayloop reopen --date 2026-09-05 --reason "追加作業が判明したため"
```

`ledger check` は既存のDBを読み取り専用で検査します。DBが無い場合は作成せず、未知の新しい形式や必要なテーブルの欠落もエラーにします。閉鎖済みの日に未完了タスクがあるなどの矛盾は、修復せずに報告します。

閉鎖済みの日には、タスク追加・候補採用・持ち越し・分割先指定ができません。訂正が必要なら本人が理由付きで再開します。再開は日時と理由を履歴に保存し、日次の閉鎖・計画確定を解除します。タスクの完了結果は変更しません。

予定済みタスクの移動には理由付きの `carry` を使います。MCPの `schedule_task` は未計画タスク専用です。同じ期限の再指定では持ち越し回数をリセットしません。タスク0件の日を含め、前の未閉鎖日が残っている間は次の日の計画を確定できません。

既存の台帳を初めて開くときは、`backups/` に移行前の整合したコピーを保存してからスキーマを更新します。移行に失敗した場合はDB変更を取り消します。`backup` は同じデータ領域の `backups/` にコピーを作り、`--output` で領域内の別名も指定できます。既存ファイルは上書きしません。

Markdownの取込は全行を一括で反映します。不正なIDや別日のタスクが含まれる場合、途中までの追加・完了も取り消します。

## 終了コード

| コード | 意味 |
|---|---|
| 0 | 完了 |
| 1 | エラー |
| 2 | 本人の回答待ち（`--yes` や非対話実行で未回答が残った） |

定時実行（タスクスケジューラ等）では `--yes` を付け、終了コード 2 のときに通知を出す運用にします。常駐させる場合は下の `serve` を使います。

## MCP

```bash
dayloop mcp
```

stdio で MCP サーバーが立ちます。設定例は [docs/mcp-clients.md](docs/mcp-clients.md)（VS Code / LM Studio / Claude Desktop）。ツールは `get_today` / `plan_day` / `close_day` など仕様書 §6 と同じ群です。`close_day` は open が残ると閉じず、`questions` を返します。

## 常駐と定時実行

```bash
dayloop config init          # %LOCALAPPDATA%\dayloop\config.toml を生成
dayloop serve                # 前面で常駐。1分ごとに時刻を見て非対話実行
dayloop serve --quiet        # コンソールを出さない
dayloop startup install      # ログオン時に serve --quiet を起動（HKCU Run）
dayloop startup status
dayloop startup remove
```

既定の時刻は plan 08:30 / check 13:00 / close 18:00 / retro `Fri 18:30`、平日のみ。回答が必要ならトースト（失敗時は `notify.txt`）を出し、`serve.log` に残します。PC が寝ていて時刻を過ぎていた場合は、復帰後の最初のチェックで未実行フェーズを1回ずつ実行します。plan / check の直前に Outlook 取り込みを試み、失敗はログだけです。

## 取り込み（Outlook）

```bash
dayloop intake outlook              # COM で受信トレイ・予定表を読む
dayloop intake outlook --since 3d --dry-run
dayloop intake fixture fixtures/outlook   # デモ用 JSON
```

クラシック Outlook が入っていれば管理者権限なしで Candidate と当日の予定に流れます。新しい Outlook（olk.exe）や未インストールでは 1 行メッセージで終了コード 0 です。本文・メールアドレスは既定で読みません。

`config.toml` の `[intake]`:

```toml
[intake]
outlook = true
lookback_days = 3
read_body = false
keywords = ["お願い", "ご確認", "please"]
important_senders = []
meeting_prep = true
meeting_prep_only_required = true
```

## Markdown

`%LOCALAPPDATA%\dayloop\days\YYYY-MM-DD.md` に当日分が書き出されます。
`- [ ]` を `- [x]` にすると `dayloop import` で完了になり、`## 今日のタスク` の下に `- [ ] 新しい行` を足すと新規タスクになります。

## 環境変数

| 変数 | 用途 |
|---|---|
| `DAYLOOP_HOME` | データの場所を変える（既定は `%LOCALAPPDATA%\dayloop`） |
| `DAYLOOP_LLM_ENDPOINT` | OpenAI 互換エンドポイント（段階2以降で使用。未設定でも動く） |
| `DAYLOOP_INTERACTIVE=1` | パイプ入力でも対話プロンプトを出す（テスト・ラッパー用） |

## ビルド

```bash
cargo build --release
```

`target/release/dayloop.exe` を任意のユーザーフォルダに置くだけで動きます。

## インストール

1. [Releases](https://github.com/OWNER/dayloop/releases) から zip をダウンロードして展開する
2. `dayloop.exe` を任意のユーザーフォルダに置く（管理者権限は不要）
3. SmartScreen が出たら「詳細情報 → 実行」で通る（管理者昇格は不要）
4. 書き込み先は `%LOCALAPPDATA%\dayloop` のみ

## 動作条件

- Windows 10 / 11
- 旧 Outlook（クラシック）がある場合のみ COM 取り込みが有効。無くても計画・確認・クローズ・MCP・常駐は動く

## セキュリティ

- ネットワーク送信なし
- パスワード・トークンを扱わない
- Outlook への書き込みなし（送信・既読化・フラグ変更をしない）
- 本文・メールアドレスは既定で読まない

判断の記録は [docs/decisions.md](docs/decisions.md)。

## ロードマップ

仕様は [../dayloop-spec.md](../dayloop-spec.md)。段階3 まで（CLI + MCP + 常駐 + Outlook COM 取り込み）は実装済み。段階4 以降（VS Code 拡張版、会議・アラート・勤怠、M365 Copilot 接続）は予定です。
