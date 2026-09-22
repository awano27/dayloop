# 未決事項（段階2）

指示書ファイルは編集せず、判断はここに残す。以降の段階でも論点を追記する。

| 論点 | 選んだ扱い | 理由 |
|---|---|---|
| JSON-RPC フレーミング | 改行区切り JSON（1メッセージ = 1行、ヘッダ無し）。`Content-Length` は受け付けない | 第1弾では LSP 式 Content-Length を選んだが、MCP stdio の仕様は改行区切り。VS Code / LM Studio / Claude Desktop が読めない。第2弾で仕様に合わせ、互換モードは置かない（二つの読み方が混ざると欠陥をテストが見逃す） |
| 質問形式 | `resolve_with` を廃止し、`options[]` を `{label, tool, args, needs}` にした。`tool` は1ツール名か `null` | パイプ区切りだと LLM が「どの選択肢がどのツールか」を機械的に決められない。`args` は即渡し、`needs` は本人に追加質問する引数 |
| `serve --quiet` | `FreeConsole()`。`#![windows_subsystem = "windows"]` は使わない | 後者は CLI 全体が GUI サブシステムになり、`mcp` の stdio と通常の CLI 出力が壊れる。制約: ログオン時にコンソールが一瞬出ることがある |

# 段階3

| 論点 | 選んだ扱い | 理由 |
|---|---|---|
| COM 実装 | `windows` クレートの IDispatch 遅延バインディング。PowerShell / VBScript は使わない | 実行ポリシーと cscript 廃止の影響を受けない。管理者権限・アプリ登録も不要 |
| ガード回避 | 既定では `EntryID/Subject/SenderName/ReceivedTime/UnRead/FlagStatus/FlagRequest/ConversationTopic` と予定の `Organizer` まで。アドレス帳系と本文は読まない。`[intake] read_body = true` のときだけ本文先頭 2000 文字 | Outlook のオブジェクトモデルガードが出ると無人の serve が止まる |
| Restrict の日付 | `'yyyy/MM/dd HH:mm'` | 指示どおり。日本語ロケールで弾かれる場合は未検証 |
| 空の important_senders | そのルールを無効 | 見せすぎを避ける（仕様書 §13） |
| フィクスチャ | `fixtures/outlook/intake.toml` を重ねて `important_senders = ["田中"]` | デモ 4 ルールを default config だけに依存させない |
| Outlook COM 実機 | **未検証**。開発 PC は COM 未検出・新しい Outlook のみ。コンパイルと ProgID 失敗時の終了コード 0 だけ確認 | 指示どおり実機なしで書く |
| フィクスチャの基準時刻注入 | `FixtureSource::load_at(dir, now)`。`load` は `Local::now()` を渡すだけ。テストは 09:00 固定 | `start_offset_minutes` が実行時刻に依存し、22:00 以降で翌日に落ちていた |

# 段階3.1以降

未決の論点は [plans/2026-09-22-roadmap.md](plans/2026-09-22-roadmap.md) の「決定を後に残すもの」。製品方針は、判断を Jev に寄せて本人は残りと確定の1回だけを見ること（[analysis/04-jev-product-value.md](analysis/04-jev-product-value.md)）。選んだあとに、この表へ行を足す。
