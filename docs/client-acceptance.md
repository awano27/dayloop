# クライアント受入れチェックリスト

このチェックリストは、MCP stdio 契約、実クライアント、開示制御、外部サービスを分けて記録します。試験ではクライアントを起動する環境に、一意で空の `DAYLOOP_HOME` を設定します。実ユーザーの台帳、外部サービス、認証情報は使いません。実施していないことや、環境がないことは `Unknown` と記録します。

## 記録欄

| 項目 | 記録 |
|---|---|
| dayloop のコミット/SHA |  |
| Windows / 実行ファイルの絶対パス |  |
| クライアント環境に設定した `DAYLOOP_HOME` |  |
| LM Studio のバージョンと GUI の有無 |  |
| VS Code / GitHub Copilot 拡張のバージョン |  |
| 実行日時（JST） |  |
| 判定（Pass / Fail / Unknown） |  |

## 1. 隔離した基本契約

- [ ] クライアント環境の `DAYLOOP_HOME` は一意な空フォルダで、実データ領域を参照していない。
- [ ] `initialize`、`tools/list`、ツール呼び出しを改行区切り JSON-RPC で実行できる。
- [ ] stdout にログが混ざらず、エラーは JSON-RPC またはツール結果で判定できる。
- [ ] `plan_day` が日を準備し、`list_reviews` が対象日のレビュー集合を返す。`prepare_day` を MCP ツールとして呼んでいない。
- [ ] `close_day` は未処理タスクまたは必須レビューがあると閉じず、必要な `questions[]` を返す。
- [ ] 回答を与えずに会話を終了しても、タスク、候補、レビュー、計画確定、日次閉鎖が変わらない。
- [ ] `reopen_day` は日付と空でない理由を必要とし、本人の明示指示なしには呼ばれない。

## 2. 実際のクライアント

### LM Studio

LM Studio GUI がこの PC にある場合にだけ実施します。公式の設定導線は Program タブの **Install > Edit mcp.json** です。([LM Studio: Use MCP Servers](https://lmstudio.ai/docs/app/mcp))

- [ ] GUI とバージョンを記録した。
- [ ] クライアント環境の `DAYLOOP_HOME` を設定した上で、`dayloop mcp --profile local` を stdio サーバーとして登録した。
- [ ] クライアント表示またはログで、dayloop の起動と `tools/list` の結果を確認した。
- [ ] チャットから `plan_day` または `list_reviews` を呼び、未回答の質問で台帳が変わらないことを確認した。
- [ ] GUI がない、起動できない、ツール名を確認できない場合は `Unknown` と記録した。

### VS Code / GitHub Copilot

VS Code の stdio MCP 設定は `.vscode/mcp.json` またはユーザープロファイルの MCP 設定に置きます。([VS Code: MCP configuration reference](https://code.visualstudio.com/docs/agents/reference/mcp-configuration))

- [ ] VS Code と GitHub Copilot 拡張のバージョンを記録した。
- [ ] 隔離ワークスペースで `dayloop mcp --profile github-copilot` を登録し、そのクライアント環境に `DAYLOOP_HOME` を設定した。
- [ ] 隔離した `config.toml` で `[ai.github_copilot] enabled = true` を明示した。
- [ ] Copilot チャットから `plan_day` または `list_reviews` を呼べた。
- [ ] `enabled = false` またはセクションなしでは起動が拒否され、隔離台帳が変わらない。
- [ ] Copilot のサインイン、組織ポリシー、モデル選択を確認できない場合は `Unknown` と記録した。

カスタムホストや CLI ハーネスの成功は MCP stdio の補助確認です。LM Studio GUI や VS Code/GitHub Copilot の実クライアント成功、ローカルモデルの推論、外部サービス接続の成功とは区別します。

## 3. Cloud プロファイルの開示制御

- [ ] `github-copilot` は `enabled=false` のとき開始時に拒否される。
- [ ] opt-in 後の既定出力はメタデータとタイトルを含み、`source_ref`、根拠、ノート、`body`、`excerpt` を含まない。
- [ ] `allow_source_refs`、`allow_evidence`、`allow_notes`、`allow_bodies` は個別に有効化したときだけ対象を出力する。
- [ ] `allow_bodies` は `body` と `excerpt` を同時に制御し、`[intake] read_body` とは独立している。
- [ ] 応答のフィルタが `questions[].options[].args` まで再帰的に適用され、許可されない機微フィールドを残さない。
- [ ] allowlist にない未知フィールドを出力しない。
- [ ] クラウドチャットへ直接貼り付けた文章は dayloop で制御できないことを受入れ記録に残した。

## 4. レビュー、ノート、ルーティン

- [ ] `teams`、`outlook`、`attendance`、`tasks`、`alerts`、`meeting_prep`、`meeting_results` が `list_reviews` で扱える。
- [ ] `record_review` は `confirmed` を記録でき、`needs_action` はタスクまたは候補へのリンクなしでは受理しない。
- [ ] `not_checked` は空理由を拒否し、理由付きでも翌日のレビュー発生を無効化しない。
- [ ] `configure_reviews` は次に初めて準備する日へ適用し、既存日のレビューを変更しない。
- [ ] `ingest_note` は `ACTION:`、`TODO:`、`宿題:`、`対応:` の明示行だけを候補にし、引用行とコードフェンス内を無視する。
- [ ] ノート本文から担当者、期限、根拠を推測せず、「要確認」を自動追加しない。
- [ ] 同じ明示的 `source_ref` と同内容の再取込は安定し、内容を改訂する場合は別の参照が必要である。
- [ ] 50 ACTION、タイトル 512 バイト、本文 1 MiB の上限を確認した。
- [ ] `add_routine`、`list_routines`、`set_routine_enabled` が本人の明示回答に従い、日付ごとの発生を重複作成しない。
- [ ] `save_retro` は本人が確認した内容だけを保存する。

## 判定の書き方

「MCP stdio 契約 Pass」「LM Studio GUI Unknown」「VS Code/GitHub Copilot 実クライアント Unknown」「Outlook/Teams 等の実サービス Unknown」のように、根拠を分けて記録します。実クライアント GUI や外部サービスの利用成功、連続日数の運用実績は、このチェックリストだけでは証明しません。
