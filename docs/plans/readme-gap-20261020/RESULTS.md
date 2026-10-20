# README要件との差分修正結果（2026-10-20）

## 対象と保護

- リポジトリ: `C:\Users\awano\新規プロダクト\dayloop`
- 開始ブランチ: `codex/requirements-gap`
- 開始HEAD: `5d6a2358ee17e8d4703dd6b81ceb91af8f8f6ffc`
- 開始時の変更は未追跡の `docs/issues.md` のみ。内容は変更せず、開始時と検証時のSHA256は `66C74C67D7510F5D8DD2CAA1B57980F8ACA77EA409724AE00F56E4E94B071ACF` で一致。
- READMEの要件は変更していない。stage、commit、push、公開、設定置換は行っていない。

## 再現と修正

| 項目 | READMEの対象要件 | 修正前の実行再現 | 修正後の動作 |
|---|---|---|---|
| 1 日誌保全 | 「閉じた日の再確認」で元のclosed_atと日誌を維持 | 閉じた日に手書き追記、候補・約束を追加してCLI checkすると日誌バイト列が変化 | 通常更新は終了済み日誌を保全。CLI check/reclose、MCP check_inで質問を維持し、CLIは無回答なら終了コード2。明示exportは従来どおり再出力可能 |
| 2 前日の初回終了 | 「1日の流れ」で残件を確認して終了しMarkdownを書き出す | planから前日の残件に回答しても日誌は未終了のまま。出力を失敗させるとclosed_atだけが残る | 初回終了時の出力を共通処理へ集約。複数の前日、既に残件を解決済みの日も出力。書出し失敗はエラーとして返し、closed_atをロールバックして再試行可能 |
| 3 外部取込 | 取得失敗・部分取得を「依頼なし」の根拠にしない | 取得済み2件の2件目の保存をDBトリガーで失敗させると1件目だけが残る。保存エラーをNoneへ変換する経路も確認 | GitHub/Jira/Teams/DevOpsはバッチ単位でロールバック。共通実行経路でfetch/store失敗を保持。未設定は未実行、実行して0件はsuccess。同期記録保存失敗も元のエラーに追記 |
| 4 受信フォルダー | 同じく、失敗・部分取得を隠して依頼なしと扱わない | 不正UTF-8や保存失敗がok=true、inboxがファイルの場合は結果自体が消える | enumerate/read/storeを区別。ファイル単位で保存し、成功したファイルを維持。追加・重複・成功ファイル・失敗ファイル件数を出力。再実行で重複しない |

日誌系は新規4テスト、取込系は新規4テストを最初に実行して失敗を確認してから実装した。MCPでも不正UTF-8を成功として返す状態を修正前に確認した。初回出力失敗からの再試行、明示export、全4アダプターのロールバック、同期記録自体の失敗などの追加回帰テストも実施した。

レビューで、WALモードの読取トランザクションでは通常更新と終了処理が競合することも確認した。競合テストを失敗させた後、通常日誌更新の「未終了確認＋書出し」をBEGIN IMMEDIATEで直列化し、同じテストを成功させた。

## 変更ファイルと責務

| ファイル | 理由 |
|---|---|
| `src/markdown.rs` | 通常更新専用のexport_if_openを追加。明示exportは維持 |
| `src/store.rs` | 初回終了と日誌書出しを同じ終了処理へ集約。書出しエラー時の終了ロールバック、通常更新との排他 |
| `src/rituals.rs` | closeおよびplanからの各前日終了を共通処理へ接続 |
| `src/main.rs`, `src/tools.rs`, `src/serve.rs` | CLI/MCP/常駐の通常export経路を保護し、二重書出しを除去 |
| `src/intake/mod.rs` | 保存失敗の握り潰し除去、受信フォルダーの失敗伝達、件数・運用状態・安全なエラー種別、同期記録失敗の追記 |
| `src/intake/github_intake.rs`, `chat_live.rs`, `devops_live.rs`, `jira_live.rs` | 共通の原子的バッチ保存を使用。Jiraの連携機能追加なし |
| `src/intake/minutes.rs` | 議事録のファイル単位ロールバックと重複件数 |
| `tests/commitment_regressions.rs`, `tests/invariants.rs` | 日誌保全、複数前日終了、未回答、失敗・再試行、明示export、WAL競合 |
| `tests/intake_pipeline.rs` | 取得済みデータの保存障害、不正UTF-8・混在・列挙障害、再実行、約束状態不変 |
| `tests/mcp_stdio.rs` | 実プロセスの結果・同期台帳・二重障害・エラー秘匿、CLIとの意味の整合。親の認証情報を継承しない起動ヘルパー |

## CLI/MCPの整合

- 初回終了は同じclose_day_with_journalを利用する。出力失敗はCLIでエラー、MCPではツールエラーとして返す。終了済みの日の再確認では、閉じた事実と現在の回答待ちを分ける。
- MCP plan_dayは既存どおり質問を返す経路であり、CLIの対話を自動実行しない。前日の回答・close_day、現在のplan_revisionを使ったconfirm_planという本人確認を維持する。
- 取込のCLI表示とMCP JSONは同じSyncResultを使う。JSONのsourcesにstatus、error、候補追加・重複・予定・成功/失敗ファイル件数を返す。
- 未設定アダプターはsourcesに含めず、実行した正常0件はsuccess、失敗はfailed、保存済み項目または処理成功ファイルがある失敗はpartialとする。
- MCP sync_sourcesのisError=falseは応答を正常に返せた意味で、取込成功を表さない。取込の成否はsourcesのok/statusとerrorsで判断する既存契約を維持する。
- 同期台帳は取得範囲の完全性を保守的に扱うため、運用上successでもpartialを記録する。今回、この区別を完全取得へ変更していない。

## 検証条件と限界

- 固有の一時DAYLOOP_HOME、合成データ、SQLiteトリガーによる障害注入を使用。本番データ不使用。
- 子プロセスは外部認証・gh探索を遮断し、Jev/Outlookを無効化。全体テスト実行プロセスからも外部認証情報を除去。
- 今日/未来/過去の確認時刻、古いplan_revision拒否、同名タスクへの完了自動適用防止、返答だけでは完了にしないこと、約束変更の本人確認・版番号、captureの当回send同意、未回答・yesによる回答捏造防止は既存回帰テストを維持。
- NOT_RUN: 実サービス接続、Outlook/Teams実機、各MCPクライアント、インストーラー、公開、master統合、CI。
- ファイル書込みとSQLiteコミットを跨ぐクラッシュ耐性まで保証する変更ではない。通常の書出し失敗・再試行と並行する通常更新は検証対象。プロセス強制終了や、書出し成功後のDBコミット障害はNOT_RUN。

## 最終結果

| 対象 | 状態 | 検証結果 |
|---|---|---|
| 修正1：終了済み日誌保全 | DONE | 日誌バイト列・closed_at保全、未回答質問、明示export、WAL競合を検証 |
| 修正2：planからの前日出力 | DONE | 複数日・解決済み前日・未回答・書出し失敗・再試行を検証 |
| 修正3：外部取込保存失敗 | DONE | 共通アダプター失敗伝達、全4アダプターのロールバック、約束不変、同期記録の失敗伝達を検証 |
| 修正4：受信フォルダー失敗 | DONE | 不正UTF-8・混在・0件・列挙・保存・重複抑止・CLI/MCP表示を検証 |
| 外部実機・実クライアント・公開 | NOT_RUN | 今回の対象外 |

- `cargo test --locked --offline --no-fail-fast`: **167 passed / 0 failed**、終了コード0。
- うち主な関連テスト: commitment_regressions 15、invariants 9、intake_pipeline 8、intake_minutes 2、mcp_stdio 6、SyncResult単体4。
- `git diff --check`: 終了コード0。CRLFを将来GitがLFへ正規化する旨の通知のみ。空白エラーなし。
- 全体ログ: `target/readme-gap-20261020/cargo-test.log`（Git対象外のローカル検証成果物）。
- 最終テストに既存失敗・新規失敗はなし。修正途中の再現用失敗と、テストデータ/アサーションの調整は最終成功と分離して扱った。
- Jev: shadowでhigh推奨、設定変更なし。最終判断補助はテスト成功を支持した一方、要約だけでは一部の実装主張を裏付け不足と判定。Codexが対象差分・実行済み回帰テストで確認して完了判断した。
