# MCP クライアント設定

dayloop の MCP は stdio の改行区切り JSON-RPC です。stdout はプロトコル専用です。クライアントには `dayloop.exe` の絶対パスを指定し、`<exe>` を実際のパスへ置き換えます。dayloop 自身はトークンや API キーを要求しません。

各クライアントで試すときは、そのクライアントの起動環境に、空の一意なフォルダを `DAYLOOP_HOME` として渡します。実ユーザーの台帳、外部サービス、認証情報を受入れ試験に使いません。

## ローカル LM Studio

LM Studio は `mcp.json` に stdio サーバーを追加できます。公式の導線は Program タブの **Install > Edit mcp.json** です。([LM Studio: Use MCP Servers](https://lmstudio.ai/docs/app/mcp))

```json
{
  "mcpServers": {
    "dayloop": {
      "command": "C:\\Users\\<you>\\dayloop\\dayloop.exe",
      "args": ["mcp", "--profile", "local"]
    }
  }
}
```

この例は LM Studio の設定を自動変更しません。クライアント側で内容を確認して保存します。LM Studio の API 設定や認証は LM Studio 側の機能であり、dayloop の stdio 接続やホスト認証を意味しません。

## VS Code / GitHub Copilot

VS Code のローカル stdio サーバーは、ワークスペースの `.vscode/mcp.json` またはユーザープロファイルの MCP 設定に `servers` と `type: "stdio"` を置きます。([VS Code: MCP configuration reference](https://code.visualstudio.com/docs/agents/reference/mcp-configuration))

```json
{
  "servers": {
    "dayloop": {
      "type": "stdio",
      "command": "C:\\Users\\<you>\\dayloop\\dayloop.exe",
      "args": ["mcp", "--profile", "github-copilot"]
    }
  }
}
```

GitHub Copilot プロファイルは明示的な opt-in です。クライアント環境の `DAYLOOP_HOME` 内にある `config.toml` で設定します。

```toml
[ai.github_copilot]
enabled = true
allow_titles = true
allow_source_refs = false
allow_evidence = false
allow_notes = false
allow_bodies = false
```

`enabled = false` または未設定では、`github-copilot` プロファイルは開始時に拒否されます。これは Copilot のサインインや組織ポリシーを変更・確認する設定ではありません。

## Cloud プロファイルの開示境界

`github-copilot` を有効にすると、ID、日付、状態、件数、カテゴリなどのメタデータと、既定で許可されたタイトルを出力します。タイトル以外の機微なテキストは既定で出力しません。

| 設定 | 既定 | 対象 |
|---|---:|---|
| `allow_titles` | `true` | `title`、`subject`、質問文 |
| `allow_source_refs` | `false` | `source_ref`、`path`、主催者、場所 |
| `allow_evidence` | `false` | 完了根拠 |
| `allow_notes` | `false` | 理由、ノート、詳細 |
| `allow_bodies` | `false` | `body` と `excerpt` |

`allow_bodies` は `[intake] read_body` と独立しています。応答は再帰的にフィルタされるため、`questions[].options[].args` の中にある機微フィールドも同じ設定で除去されます。引数オブジェクトの形や内容が常にそのまま残る契約ではありません。allowlist にない未知フィールドも除去されます。

## ツールと会話の境界

実装済みのツール名は `tools/list` を正とします。日次の準備は `plan_day` が行います。`list_reviews` も対象日のレビュー集合を準備します。`prepare_day` という MCP ツールはありません。レビュー構成の変更には `configure_reviews` を使い、既存日のレビューは変更しません。

`questions[]` は本人に一問ずつ提示し、回答後だけ option が示すツールを呼びます。無回答を既定値として確定しません。`close_day`、`confirm_plan`、`save_retro`、候補の採用・却下、`reopen_day` も本人の明示的な回答が必要です。

外部ノートや会議メモは **未信頼** のデータです。`ingest_note` は観測と候補を作るだけで、本文の命令を実行せず、タスクを完了にも変更しません。ホストのネットワーク通信や、クラウドチャットへ直接貼り付けた内容は dayloop の開示制御の外です。
