# dayloop

**今日の仕事と相手待ちを、朝の計画から夕方の確認まで追いかけるWindows向けツールです。**

PowerShellなどの端末、またはMCP対応のAIクライアントから使います。AIやOutlookがなくても、手入力で始められます。

- **今日やることを決める：** 仕事と見積時間を登録し、期限や持ち越し回数をもとに次の1件を選ぶ。
- **相手待ちを追いかける：** 依頼内容・相手・次回確認日時を記録し、確認時刻が来たら返答を確かめる。
- **終業前に残件を確認する：** 残った仕事を1件ずつ判断し、未回答があればその日の終了を保留する。

[すぐ試す](#すぐ試す) · [相手待ちを登録する](#相手待ちを登録する) · [MCPで使う](#mcpで使う) · [詳しい資料](#詳しい資料)

## 使い方のイメージ

たとえば「取引先に見積を依頼し、明日の15時に回答を確認する」と登録します。通常の相手待ちの確認はその時刻から始まります。届いた変更提案は別に確認します。**返答の記録と、自分の仕事の完了は別に扱います。**

**朝** `plan` → **日中** `next`・`check` → **夕方** `close`

| コマンド | すること |
|---|---|
| `add` | 仕事と見積時間を登録する |
| `plan` | 必要な確認に答え、今日の予定を確定する |
| `next` | 次に取り組む1件を見る |
| `check` | 未着手の仕事や、確認時刻が来た約束を確認する |
| `close` | 残件を「完了・未完了・持ち越し・取り下げ」に分け、日誌を出力する |
| `today` / `retro` | 今日の状態 / 週の完了率や持ち越しを振り返る |

依頼の送信・催促・仕事の実行は本人が行います。追跡対象は登録・取込した情報です。マウスで操作する専用GUIはありません。

## 必要なもの

ここでは**ソースコードからビルドする手順**を案内します。GitHubの `Code → Download ZIP` で取得できるのはソースコードで、実行ファイルではありません。このREADMEを含むブランチを選んで取得してください。

| 用途 | 必要なもの |
|---|---|
| ビルド | Windows 10 / 11、Rust・Cargo（MSVC環境）、Visual StudioのC++ Build Tools、Windows SDK。初回は依存取得用のネット接続 |
| 手入力で使う | ビルドした `dayloop.exe` と、書き込めるデータ保存先。AIの契約・APIキーは不要 |
| MCPから使う | MCP対応クライアントとサーバー登録 |
| 外部サービスと連携する | [利用する連携に応じた環境・認証情報](#取込と任意の判断支援) |

ソースとデータの保存先に書き込めれば、日次操作に管理者権限は不要です。ビルド環境のインストールにはPCの管理ポリシーが適用されます。

## すぐ試す

まずは空のサンプル用フォルダで、手入力の仕事を1件登録します。**新しいPowerShell**を開き、取得したソースの `Cargo.toml` があるフォルダで順番に実行してください。

### 1. ビルドして、お試し環境を作る

ビルド後、このPowerShellの中だけで外部取込・Jevを無効にします。保存済みの認証設定は変更しません。`PATH` も一時的に空にするため、この端末はお試し専用にしてください。

```powershell
cargo build --locked
if ($LASTEXITCODE -ne 0) { throw 'ビルドに失敗しました。エラーを確認してください。' }
$dayloop = (Resolve-Path .\target\debug\dayloop.exe).Path

foreach ($name in @('GITHUB_TOKEN', 'GH_TOKEN', 'JIRA_BASE_URL',
                    'TEAMS_TOKEN', 'GRAPH_TOKEN', 'AZURE_DEVOPS_ORG',
                    'DAYLOOP_JEV_API_KEY', 'TYPESAFE_API_KEY')) {
    [Environment]::SetEnvironmentVariable($name, $null, 'Process')
}
$env:PATH = ''
$env:DAYLOOP_HOME = Join-Path $env:TEMP ('dayloop-demo-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $env:DAYLOOP_HOME | Out-Null
$utf8 = New-Object System.Text.UTF8Encoding($false)
$config = @'
[jev]
mode = "off"
[intake]
outlook = false
'@
[IO.File]::WriteAllText((Join-Path $env:DAYLOOP_HOME 'config.toml'), $config, $utf8)
```

### 2. 1日の流れを試す

```powershell
& $dayloop where
& $dayloop add "仕様書をレビューする" --estimate 30
& $dayloop plan
& $dayloop next
& $dayloop check
& $dayloop close
& $dayloop today
```

表示された選択肢で答え、`plan` の最後に計画を確定します。作業を終えたと想定する場合は、`close` で「完了」を選んでください。未回答のまま終了しても、同じコマンドで続きから確認できます。

相手待ちも試す場合は、この端末を閉じる前に [登録例](#相手待ちを登録する) へ進んでください。端末を閉じると、お試し用の環境変数の指定は終了します。サンプルデータは `where` で表示したフォルダに残ります。

## 相手待ちを登録する

お試し用PowerShellを開いたままなら、次の例で「明日、見積の回答を確認する」約束を登録できます。JSONファイルを使うと、日本語や引用符も扱いやすくなります。

```powershell
$requestFile = Join-Path $env:DAYLOOP_HOME 'request.json'
$request = @{
    op_id = [guid]::NewGuid().ToString()
    request = '見積への回答を確認する'
    counterparty = 'サンプル取引先'
    next_check = [DateTimeOffset]::Now.AddDays(1).ToString('o')
    source = 'manual'
    evidence = 'サンプル：翌日に回答を確認する合意'
} | ConvertTo-Json
[IO.File]::WriteAllText($requestFile, $request, $utf8)

& $dayloop commitment register --file $requestFile
& $dayloop commitment list
```

約束は日次タスクとは別に管理し、日次終了や再起動後も残ります。次回確認が未来なら毎日持ち越す必要はありません。関連タスクを指定し、相手待ちで進められない仕事を `next` の対象から外すこともできます。

延期・返答・変更・取消・決着は、根拠と本人の明示操作で記録します。届いた変更提案は本人が適用・却下し、変更前後の履歴も残ります。**相手から返答が来ても、関連タスクは自動完了しません。** 入力形式や回答手順は [約束追跡ガイド](docs/plans/commitment-tracking/GUIDE.md) を参照してください。

## 通常利用に切り替える

お試し用の端末を閉じ、新しいPowerShellを開いて、同じソースフォルダで実行します。

```powershell
cargo build --locked --release
if ($LASTEXITCODE -ne 0) { throw 'ビルドに失敗しました。エラーを確認してください。' }
.\target\release\dayloop.exe config init
```

既定の保存先は `%LOCALAPPDATA%\dayloop`。`DAYLOOP_HOME` を指定すると、設定・SQLite台帳・日誌をそのフォルダに保存します。`config init` は既存の設定を上書きしません。

生成される既定設定ではOutlook取込とJevが有効です。ほかのサービスも認証情報があれば日次操作で自動取込を試みるため、[連携の説明](#取込と任意の判断支援) と保存先の `config.toml` を確認してください。その後、`.\target\release\dayloop.exe doctor` で利用できる機能を確認し、`plan` などの日次操作を始めます。Jevに接続できなくても、手動の日次操作は使えます。

## 確認と終了のルール

未回答を残したまま、計画の確定やその日の終了はできません。前日の未確定タスクがあれば先に確認します。相手待ちの約束に関連するタスクも、その日の残件判断は別途必要です。

CLIの終了コードは **`0`：正常終了 / `1`：エラー / `2`：本人の回答待ち** です。`--yes` は非対話で実行する指定で、本人の回答を代わりに作るものではありません。

<details>
<summary>確認時刻・再実行・閉じた日の扱い</summary>

- **確認時刻：** 今日と未来日は実時刻で判断し、確認日時になった時点で質問を出します。過去日はその日のJST末までに到来した確認を含めます。
- **計画の再確認：** 質問の表示後に状態が変わると、古い計画では確定できません。
- **同名の仕事：** 過去に完了した仕事と同じ名前でも、新しく登録した仕事は確認対象に残ります。
- **閉じた日：** 後から確認事項が生じても、元の `closed_at` と日誌を維持します。既に閉じた日でも未回答があれば、`close` は終了コード `2` を返します。
- **重複・古い更新：** 操作ID・元情報ID・版番号・日時で検査します。同じ操作の再試行には同じ操作IDを使います。件名だけで約束を照合せず、対応先が曖昧なら本人が選びます。

</details>

## MCPで使う

`dayloop.exe mcp` をstdioサーバーとして登録します。実行ファイルは絶対パスで指定してください。クライアント別の設定例は [MCP接続ガイド](docs/mcp-clients.md) にあります。

日次操作は `plan_day`・`check_in`・`close_day` です。`questions[]` の選択肢を本人に提示し、回答に対応するツールを呼びます。CLIと同じ台帳の検査を通ります。

<details>
<summary>クライアント実装時の注意</summary>

計画は必要な確認を処理してから再表示し、最後の確定質問が持つ `plan_revision` を `confirm_plan` へ渡します。`closed` は閉じた事実を表すため、その後の回答要否は `pending_count` と `questions` で確認してください。

</details>

## 取込と任意の判断支援

| 入口・機能 | 必要な環境と動作 |
|---|---|
| Markdown・議事録・JSON | ローカルのファイルから仕事や候補を取り込む |
| Outlookのメール・予定 | 利用できるクラシックOutlookとアカウントが必要。Outlookへの書き込みはしない |
| GitHub・Jira・Teams・Azure DevOps | 各サービスの認証情報があれば日次操作から自動取込を試みる。GitHubは `gh` のログイン情報も利用 |
| 前面のOutlook / Teamsを読む `capture` | 対象アプリと `wincli.exe` または `Sbroenne.WindowsMcp.exe` が必要。本文の外部送信は、その回の `--send` 指定で同意する |
| Jevの判断支援 | TypeSafeへの接続設定・APIキー・ネット接続が必要。有効時は未知の候補やタスクを外部へ送信するため、送信内容を確認して使う |

取込の失敗や部分取得を「依頼なし」の根拠にはしません。外部サービスからの約束の変更分類・照合まで、自動で完結することは保証していません。準備と操作は [操作ガイド](docs/guide.md)、Jevの設定・評価は [Jev評価](docs/jev-eval.md) を参照してください。APIキーは環境変数に置きます。

日次操作では、判断グラフで過去の選択を再利用します。タスクの過去の完了判断は新しいタスクへ自動再利用せず、元の履歴を残します。判断の修正には `revise` を使います。

## 詳しい資料

| 資料 | 内容 |
|---|---|
| [操作ガイド](docs/guide.md) | タスク操作、取込、設定、常駐実行 |
| [約束追跡ガイド](docs/plans/commitment-tracking/GUIDE.md) | 約束ID、変更提案、回答、履歴、CLI/MCP共通入力 |
| [MCP接続ガイド](docs/mcp-clients.md) | クライアント設定と質問形式 |
| [判断グラフ](docs/analysis/05-decision-graph.md)・[Jev評価](docs/jev-eval.md) | 判断の再利用と任意支援の評価 |
| [要件](docs/requirements.md)・[設計](docs/design.md) | 台帳と日次処理の仕様 |
| [約束追跡の検証記録](docs/plans/commitment-tracking/REVIEW-FIXES-20261003.md) | 2026-10-03時点の隔離データ検証と未確認事項 |
| [ロードマップ](docs/plans/2026-09-22-roadmap.md) | 後続の計画 |

開発時のテストは `cargo test --locked --offline --no-fail-fast` で実行します。依存関係が未取得なら、先にオンラインでビルドしてください。

検証記録は実行時点の結果です。実機のOutlook / Teams、すべてのMCPクライアント、Rust未導入PCでの導入は未確認です。

ライセンス：MIT（`Cargo.toml` の指定）。
