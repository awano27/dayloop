# MCP クライアント設定

トランスポートは stdio、改行区切り JSON-RPC。stdout はプロトコル専用なので、コマンドは絶対パスの exe を指定してください。トークンや API キーは不要です。

`<exe>` を `dayloop.exe` の絶対パスに置き換えて使います（例: `C:\Users\<you>\dayloop\dayloop.exe`）。

## VS Code / GitHub Copilot

`.vscode/mcp.json`:

```json
{
  "servers": {
    "dayloop": {
      "type": "stdio",
      "command": "C:\\Users\\<you>\\dayloop\\dayloop.exe",
      "args": ["mcp"]
    }
  }
}
```

「今日のタスクを見せて」と聞くと Copilot が `get_today` を呼びます。計画・確定は `plan_day` → 個別ツール → `confirm_plan`、夕方は `close_day` です。open が残っていると `close_day` は閉じません。

## LM Studio

`mcp.json` の `mcpServers`:

```json
{
  "mcpServers": {
    "dayloop": {
      "command": "C:\\Users\\<you>\\dayloop\\dayloop.exe",
      "args": ["mcp"]
    }
  }
}
```

## Claude Desktop

`claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "dayloop": {
      "command": "C:\\Users\\<you>\\dayloop\\dayloop.exe",
      "args": ["mcp"]
    }
  }
}
```

Windows での場所は `%APPDATA%\Claude\claude_desktop_config.json` です。

## 質問形式

`plan_day` / `check_in` / `close_day` / `retro_week` の `questions[]` は、選択肢ごとに呼ぶツールが1つに決まっている。

```json
{
  "kind": "close_task",
  "target_id": "01M1…",
  "title": "経費精算",
  "question": "このタスクをどうしますか",
  "default": 0,
  "options": [
    {"label": "完了", "tool": "finish_task", "args": {"id": "01M1…"}, "needs": []},
    {"label": "未完了", "tool": "set_not_done", "args": {"id": "01M1…"}, "needs": ["reason"]},
    {"label": "持ち越し", "tool": "carry_over", "args": {"id": "01M1…"}, "needs": ["reason"]},
    {"label": "取り下げ", "tool": "drop_task", "args": {"id": "01M1…"}, "needs": ["reason"]}
  ]
}
```

- `args` はそのまま `tools/call` に渡せる引数
- `needs` は本人に追加で聞く引数名（空ならすぐ呼べる）
- `tool` が `null` の選択肢は何もしない（「今日中にやる」「後で決める」など）
- 第1弾の `resolve_with`（パイプ区切りの複数ツール名）は廃止した
