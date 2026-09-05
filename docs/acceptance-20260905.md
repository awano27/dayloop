# Windows・LM Studio受入れ実施記録

確認日: 2026-09-05 JST。製品コードの対象コミット: `627fcee04d035590e70e8e7238c628de3487704b`。
実行ファイル SHA-256: `C1160AE5C480A05386EE42C3B3E10FCB73990695E55C0BEDA94EA3077270B6DE`。

## 判定

| 項目 | 判定と根拠 |
|---|---|
| Windowsの二重serve起動 | Pass。同じ専用データ領域への第2プロセスは終了コード1と「既に実行中」で拒否された |
| serveプロセスの停止・再起動 | Pass。起動ログを待ち、4フェーズの保留日・件数が一致した |
| 再起動後の台帳保持 | Pass。製品のbackupで得た前後2つのSQLiteスナップショットの論理内容が一致した |
| LM StudioでのMCP登録 | Pass。実GUIに専用接続が表示され、モデルへのツール提供元登録をLM Studioログで確認した |
| MCPのデータ領域分離 | Pass。サーバー設定のenv.DAYLOOP_HOMEに指定した専用フォルダへ台帳が作られた |
| LM Studioチャットからの日次完走 | 未完了。最初の推論中にアプリが終了し、ツール実行結果を得ていない |
| VS Code / GitHub Copilot実クライアント | Unknown。今回未実施 |
| 実際のスリープ復帰・PC再起動・HKCU自動起動 | Unknown。今回未実施 |
| 通知の目視・クリーンWindows・5営業日の実利用 | Unknown。今回未実施 |

## Windowsで確認したこと

テストごとに空の絶対パスを作り、すべてのdayloop子プロセスへ `DAYLOOP_HOME` を明示した。通知はファイル出力のみ、Outlook取得は無効。土曜日を合成設定の稼働日とし、既に経過した時刻で初回の保留状態を作った。OSの日付・時刻は変更していない。

再起動前後で `plan`、`check`、`close`、`retro` の `pending_on` はすべて非nullの `2026-09-05`、件数は順に `8 / 7 / 7 / 1` で一致した。SQLiteの比較には製品backupを使用し、WALを含む内容を取得した。2つの `.dump` のSHA-256はいずれも `7E37C30EEAC10A7FF6B60ACF0BA5CBCD6ACFB988B7DAAE37BCE155ED422EDC64` だった。起動したdayloopとSQLite検査プロセスの残留はなかった。

これは同一PC上のプロセス再起動試験であり、OS再起動やスリープ復帰、5営業日の利用実績を証明しない。

## LM Studioで確認したこと

- 既存のLM Studio `0.4.16+1` を起動し、専用接続 `dayloop-acceptance-20260905` を追加した。追加前のmcpServersは空で、変更前ファイルをバックアップした。
- 既存の `google/gemma-4-12b-qat` を使用。実際の読み込み設定はcontext length `65536`、parallel `4` だった。
- 実GUIから「2026-09-05のplan_dayを実行して質問を提示し、回答を待つ」という合成の依頼を送信した。
- LM Studioログでツール提供元の登録、3254トークンの入力処理（80.42秒）、約3.22トークン/秒の生成を確認した。
- 23:15:52に推論が停止し、その後GUIとモデルが見えなくなった。アプリ終了の原因は未確定で、手動終了だったか確認待ち。メモリ関連のWindowsイベントはあるが、終了原因の証明にはならない。
- ツールによる業務操作は観測していない。読み取り専用SQLite検査で、試行前後の論理SHAが一致した。タスク・候補・日次確認はいずれも0件のままだった。

再試行には、読み込みコンテキストと並列数を抑えるか、既存の軽量モデルを使う。終了が利用者の意図によるものか確認した上で実チャット試験を続ける。MCP登録の成功を日次業務の完走として扱わない。

## 証跡と残件

ローカルの詳細証跡は `target/verification/client-acceptance-20260905/` 以下にある。

- `windows-process/WINDOWS_PROCESS_REPORT.md`
- `windows-process/run-20260905-231250-25faae3a/result.json`
- `lmstudio-230714/attempt-1-result.json`
- `lmstudio-230714/ledger-initial.json` / `ledger-after-exit.json`
- `lmstudio-230714/lmstudio-relevant-log.txt`

外部サービス接続、クラウドへの業務データ送信、スタートアップ登録は行っていない。以前の検証で既定データ領域へ誤作成したseedの削除は、今回も実施していない。
