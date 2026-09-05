# dayloop

PdM・PjM・エンジニアの日々の仕事を、**計画 → 確認 → 終了 → 振り返り**で回すローカル業務台帳です。AIチャット、CLI、Markdownから同じ台帳を使います。

[Repository](https://github.com/awano27/dayloop) · [チャット設定](docs/mcp-clients.md) · [会話レシピ](docs/chat-workflows.md) · [実環境の受入れ](docs/client-acceptance.md) · [実装計画](PLAN.md)

現在は exe 版の開発ベータです。Teams、Outlook、勤怠、タスク、アラート、会議準備、会議結果の**7カテゴリを確認し、対応を候補・タスクに残す**共通機能を備えています。外部サービスの自動取得はクラシックOutlook COMのみ実装しています。実Outlook接続、実AIクライアント、5営業日の継続利用は別の受入れ項目です。

## 始める

Rustでビルドした `target/release/dayloop.exe`、または開発者から渡されたベータZIPをユーザーフォルダへ置きます。公開Releaseが存在することや、企業のアプリ制御で許可されることは前提にしません。

```powershell
.\dayloop.exe setup
.\dayloop.exe doctor
.\dayloop.exe plan
```

実行時にPowerShell、Node、追加ブラウザ、管理者権限は必要ありません。上のPowerShellは入力例です。データは `%LOCALAPPDATA%\dayloop`、または明示した `DAYLOOP_HOME` に保存します。初期設定は既存の `config.toml` を上書きせず、Outlook接続とクラウドAI開示は初期状態で無効です。

## 一日の操作

```text
dayloop plan
dayloop check
dayloop close
dayloop retro
```

- 朝は前の未閉鎖日、候補、未計画タスクを確認し、本人の回答で計画を確定します。
- 昼は未着手と日次確認を見直します。
- 夕方はタスクの結果と7カテゴリの回答を記録します。未回答があれば日を閉じません。
- 週次は完了率、持ち越し、確認履歴、振り返りメモを見ます。

```text
dayloop add "仕様レビュー" --date 2026-09-07 --due 2026-09-08
dayloop start <ID>
dayloop done <ID> --evidence "レビュー結果を本人が確認"
dayloop notdone <ID> --reason "回答待ち"
dayloop carry <ID> --reason "確認資料が未着" --to 2026-09-08
dayloop drop <ID> --reason "対応不要と確認"
dayloop split <ID> --reason "作業を分ける" --into "資料を集める" --into "内容を確認する"
```

既存のタスク操作は完全IDまたは一意な末尾IDを使えます。日次確認への関連付け、出典参照、定期タスクの変更は完全IDを指定します。

## 日次確認・定期タスク・会議メモ

| 結果 | 意味 |
|---|---|
| `pending` | 回答待ち。必須なら日を閉じられない |
| `confirmed` | 本人が確認済みと回答 |
| `needs_action` | 保存済みタスクか未却下候補の完全IDを関連付ける |
| `not_checked` | 未確認の理由を残す。翌日の確認は無効化しない |

```text
dayloop reviews list --date 2026-09-07
dayloop reviews record attendance confirmed --date 2026-09-07
dayloop reviews record alerts not_checked --reason "今日は接続できない" --date 2026-09-07
dayloop reviews record meeting_results needs_action --candidate-id <完全ID> --date 2026-09-07
dayloop routines add "勤怠の確認" --weekdays Mon,Tue,Wed,Thu,Fri --starts-on 2026-09-07
dayloop routines list
dayloop routines disable <完全ID>
dayloop note --category meeting_results --title "定例会議" --file meeting.txt --meeting-id meeting-123
dayloop retro --date 2026-09-07 --note "次週は依頼の期限を先に確認する"
```

`meeting.txt` はUTF-8で、`ACTION: 資料を確認する`、`TODO:`、`宿題:`、`対応:` の行を候補にします。コードブロックと引用行は除外し、本文の命令を実行しません。採用や完了は別の本人回答が必要です。最大50候補、各タイトル512バイト、本文1MiBです。同じ資料の再取り込みで候補IDと却下を維持します。`--source-ref` で指定した参照の内容は不変で、改訂資料には新しい参照を付けます。

定期タスクは `plan` / `today` / `check` / `close` などで日を準備した時に、曜日と日付ごとに一度だけ生成します。未生成の過去日は直近30日まで知らせ、自動で完了扱いにしません。

必須カテゴリの変更は、次に初めて準備する日から適用します。既存の確認は消しません。

```text
dayloop reviews configure --categories teams,outlook,attendance,tasks,alerts,meeting_prep,meeting_results
```

タスクだけで運用する場合は `dayloop reviews configure --none` を明示指定できます。旧形式から移行した既存日には新しい必須確認を遡って付けません。

## 台帳が守ること

未確定タスクや必須確認が残る日は閉じられません。未完了・持ち越し・取り下げには理由が必要です。3回持ち越した後は分割・取り下げ・実際の期限変更を選ぶまで再持ち越しできません。同じ期限の再指定では回数をリセットしません。

閉鎖日へのタスク追加・候補採用・移動は拒否します。訂正は理由付きで再開し、タスク結果は保持します。前の未閉鎖日は空の日も含めて、翌日の計画確定を妨げます。

```text
dayloop ledger check --json
dayloop backup
dayloop reopen --date 2026-09-07 --reason "追加作業が判明したため"
```

診断は読み取り専用で、自動修復しません。移行前には `backups/` へ整合したSQLiteコピーを保存し、失敗時はDBの変更を取り消します。未知の新形式や必要なテーブルの欠落は拒否します。手動バックアップ先はデータ領域内に限定し、既存ファイルは上書きしません。

## AIチャットと開示

```text
dayloop mcp --profile local
dayloop mcp --profile github-copilot
```

stdio MCPは `plan_day`、`record_review`、`ingest_note`、`close_day` などを提供します。GitHub Copilotプロファイルは `[ai.github_copilot] enabled=true` の明示設定が必要です。本文・抜粋・出典参照・根拠・ノートは別の開示スイッチで制御します。ローカルプロファイルは全情報を返すため、ローカルAI用に使います。

dayloopはクライアントの身元や通信を強制管理しません。クラウドのチャット欄へ直接貼った情報はdayloopの制御対象外です。LM StudioとVS Codeの設定例は[接続ガイド](docs/mcp-clients.md)を参照してください。M365 Copilot用の接続は未実装です。

## 常駐と通知

```text
dayloop serve
dayloop serve --quiet
dayloop startup install
dayloop startup status
dayloop startup remove
```

既定は平日08:30、13:00、18:00です。金曜の週次確認は18:00にまとめます。復帰時は経過した確認をまとめて通知し、回答待ちは次の時間帯で再評価します。通知したことを計画確定・完了・日次終了と扱いません。同じデータ領域での常駐二重起動を拒否します。

Windows通知はWin32のバルーン通知です。APIが受け付けても、人に表示された・読まれたとは証明しません。失敗時は `notify.txt` へ保存し、ファイル保存にも失敗した場合はエラーを記録します。`config.toml` の `[notify] method="file"` または `"none"` も選べます。評価状態は `serve-state.json`、運転記録は `serve.log` に保存します。

`dayloop doctor --notify-test` は明示的に検証通知を送ります。通常の `doctor` はデータを書き込まず、接続や通知を開始しません。

自動起動は本人が `startup install` を実行した時だけHKCU Runへ登録します。別の登録は上書き・削除せず、Startupフォルダへの代替スクリプトも作りません。登録時の実行ファイルとデータ領域を記憶します。

## Outlookと取得結果

接続する場合だけ、`config.toml` の `[intake] outlook=true` を設定します。本文取得は別途 `read_body=true` が必要です。

```text
dayloop intake outlook --since 3d --dry-run
dayloop intake outlook
```

クラシックOutlook COMでメール・予定を読み、候補と出典を保存します。新しいOutlookや未接続環境では利用不可を報告します。送信・既読化・フラグ変更・勤怠打刻・アラート承認は行いません。

メールと予定を別々に扱い、一方の失敗で取得済みの他方を失いません。成功0件、一部取得、失敗、利用不可、無効を区別します。取得結果は本人の確認結果やタスク完了とは独立です。Teams・勤怠・アラートは現時点では手動確認・メモ取り込みを使います。

## Markdownと終了コード

`days/YYYY-MM-DD.md` は台帳の写しです。`今日のタスク` の `[ ]` を `[x]` に変えて `dayloop import` で完了を反映できます。同じ欄の新しい `- [ ]` 行は新規タスクになります。不正IDや他日タスクがある場合は取り込み全体を取り消します。日次確認の欄は表示用で、変更はCLI・チャットから記録します。

終了コードは `0` 完了、`1` エラー、`2` 本人の回答待ちです。`--yes` は自動承認ではなく、非対話で未回答を残す指定です。取得の部分失敗は状態を出力して正常にコマンドを終了する場合があるため、取得件数だけで成功を判断しないでください。

## 開発と配布準備

```text
cargo test --no-fail-fast
cargo clippy --all-targets -- -D warnings
cargo build --release --locked
```

Windowsの開発用 `scripts/package-beta.ps1` は、ソースコミット・実行ファイル・文書・チェックサムを含むローカルZIPを生成します。署名やGitHub Release公開は行いません。運用評価は[受入れ手順](docs/client-acceptance.md)に記録します。

Edge自動取得はexe版の受入れ後、exe不要VS Code版は契約が固まった後に進めます。Graph/M365は組織が許可したテナント・試験アカウントを用意してからです。クリーンWindows、企業ポリシー、コード署名、実クライアント接続、5営業日の実利用は、単体テストやCIの成功と区別します。
