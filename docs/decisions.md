# 未決事項（段階2）

指示書ファイルは編集せず、判断はここに残す。以降の段階でも論点を追記する。

| 論点 | 選んだ扱い | 理由 |
|---|---|---|
| JSON-RPC フレーミング | 改行区切り JSON（1メッセージ = 1行、ヘッダ無し）。`Content-Length` は受け付けない | 第1弾では LSP 式 Content-Length を選んだが、MCP stdio の仕様は改行区切り。VS Code / LM Studio / Claude Desktop が読めない。第2弾で仕様に合わせ、互換モードは置かない（二つの読み方が混ざると欠陥をテストが見逃す） |
| 質問形式 | `resolve_with` を廃止し、`options[]` を `{label, tool, args, needs}` にした。`tool` は1ツール名か `null` | パイプ区切りだと LLM が「どの選択肢がどのツールか」を機械的に決められない。`args` は即渡し、`needs` は本人に追加質問する引数 |
| `serve --quiet` | `FreeConsole()`。`#![windows_subsystem = "windows"]` は使わない | 後者は CLI 全体が GUI サブシステムになり、`mcp` の stdio と通常の CLI 出力が壊れる。制約: ログオン時にコンソールが一瞬出ることがある |
