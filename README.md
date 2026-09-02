# dayloop

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

## 終了コード

| コード | 意味 |
|---|---|
| 0 | 完了 |
| 1 | エラー |
| 2 | 本人の回答待ち（`--yes` や非対話実行で未回答が残った） |

定時実行（タスクスケジューラ等）では `--yes` を付け、終了コード 2 のときに通知を出す運用にします。

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

## ロードマップ

仕様は [../dayloop-spec.md](../dayloop-spec.md)。段階1（このリポジトリ）で忘れゼロは成立しており、以降は MCP サーバー化、Outlook COM 取り込み、VS Code 拡張版、会議・アラート・勤怠、M365 Copilot 接続の順に入口を増やします。
