# レビュー3件の修正・今回の検証記録（2026-10-03）

対象は `codex/requirements-gap`、開始時HEADは `214add69e1b36ce2c1808759b6c2882123c87c01`。既存の未コミット実装を保護して、その上に修正を統合した。過去のREPORT.mdの成功報告は今回の検証として数えていない。後続の「きりのいいところでpushして」により、関連する既存実装と修正を現在のブランチへコミット・pushする。docs/issues.mdと隔離台帳・バックアップは含めない。

## 再現結果と修正

| 問題 | 修正前の隔離再現 | 修正後の隔離再現 |
| --- | --- | --- |
| 過去日の午後 | 2020-01-06 15:00 JSTの約束は、正確な時刻のcommitment_listで質問1件、check_inで0件、close_day成功 | check_inとclose_dayで質問1件、未回答のclose_dayはclosed=false |
| 閉じた日の後発確認 | 2020-01-07を閉じてから期限到来の約束を登録すると、MCPはclosed=true・質問0件、CLIは終了2で閉じられない表示 | MCPはclosed=true・already_closed=true・pending_count=1・質問1件。CLIはクローズ済みと未回答を区別し終了2。元のdays行・closed_at・履歴・日次Markdownは再実行で不変 |
| 未回答の計画確定 | plan_dayはconfirm_planをcommitment_checkより先に表示。CLIは終了2、MCP confirm_planは成功し時刻を書き込み | 確認事項が先、確定質問が最後。双方が未回答の約束・変更提案を拒否。明示回答後、再表示した版で確定成功 |

日次の時刻は `min(実時刻, 対象JST日の最後のナノ秒)`。過去日は23:59:59.999999999まで、翌日00:00は含めない。今日と未来日は実時刻を使い、通常の待ち確認は指定時刻より前には出さない。一致時刻は期限到来。未来日の表示も時計を進めない。

日次終了の共通台帳処理は閉じた事実と現在の未回答を別々に返す。MCPのclosedは歴史的事実であり、回答要否はpending_count/questionsで見る。CLI終了0は未回答なし、終了2は回答待ち（既に閉じていても同じ）。閉じた日の再実行はclosed_atを書き換えず、日次Markdownを再出力しない。後発の明示操作は新しい約束履歴として残る。

計画確定はBEGIN IMMEDIATE内で未回答・日次保護・表示版を検査する。plan_dayが返すplan_revisionをMCP confirm_planへ渡すことが必須。CLIも最終質問の前に表示版を保持する。表示内容と版は同一の読取りスナップショットで作る。約束・提案・タスク・候補・イベント・関連日状態・タスクの表示順が変化した版は拒否し、再表示を要求する。時間が進んで確認期限が到来した場合も、版の一致だけでは確定を許さない。ランダムな質問操作IDや単なる時間経過は版に含めない。

## 変更箇所

- src/commitment.rs: 共通の日時境界と安定した約束状態。
- src/store.rs: 日次終了状態、読取りスナップショット、計画版とトランザクション内の確定前提条件。
- src/engine.rs: 共通終了状態の表示、質問順、表示版。
- src/tools.rs: MCPの終了・確定処理とplan_revision契約。
- src/rituals.rs、src/main.rs: CLIの終了表示・終了コード、表示後の再検査、閉じた日の日誌保護。
- tests/commitment_ledger.rs: 境界・共通確定拒否の回帰6件追加。
- tests/commitment_regressions.rs: CLI/MCP実プロセス回帰9件追加。
- tests/commitments.rs: 期限到来を想定した2ケースの日付を実際の過去日に修正。未来の待ち確認・持ち越しの検証を維持。
- tests/growth.rs: 既存の直接確定呼出しに表示版を渡す。
- PLAN.md、GUIDE.md、acceptance-prompt.md、REPORT.md: 契約更新、今回と過去の検証の区別。

コミットには開始時から存在した約束台帳、合成fixtures、関連ドキュメント、および本文送信同意・取込識別・閉日保護等のtrust-core統合差分も含める。今回の3件修正でDBスキーマ変更は追加していない。既存の移行・既存データ保持テストも今回の全体実行に含まれる。

## 実行・証拠

証拠ルートは `target/verification/commitment-fixes-20261003-71e22276`（Git対象外）。before/に開始時31ファイルのコピー、before.patch、before-hashes.jsonを保存した。baseline-dayloop.exeは修正前バイナリ。

実行コマンド:

```powershell
cargo test --locked --offline --no-fail-fast
cargo clippy --locked --offline --all-targets -- -D warnings
git diff --check
python target/verification/commitment-fixes-20261003-71e22276/reproduce.py baseline
python target/verification/commitment-fixes-20261003-71e22276/reproduce.py after-final
```

再現スクリプトは各ケースに隔離DAYLOOP_HOMEを設定し、子プロセスの認証情報を除去、Jev無効、Outlook無効、PATHを空にして合成入力だけをCLI/MCPへ渡す。baseline/とafter-final/のcalls.jsonにコマンド・入出力、results.jsonに比較用状態、各ケースにSQLite台帳と日誌を保持した。days、commitments、commitment_updates、commitment_decisions、commitment_operationsの実行後状態を採取した。

開始時テストは131件成功。最終実行は146件成功・0件失敗・終了0（final-tests.log）。差分チェックは終了0（final-diff-check.log）。verification-summary.jsonが件数とClippy比較、final-exits.jsonが終了値を記録する。

境界は制御時計で今日の直前・一致・直後、過去日の午後・最後のナノ秒・翌日00:00、未来日を検証した。実プロセステストで終了判定の一致、閉日台帳・日誌不変、約束/変更提案の確定拒否と明示回答後の成功、質問順、表示後の別プロセスによるタスク追加、表示順変更、欠落/古い版、再起動・再実行・操作ID再送を検証した。既存の約束追跡・タスク・移行・trust-coreテストも今回成功した。

## 失敗の区別

Strict Clippyは開始時も最終も終了101。同一の既存7件が残る: src/devops.rs:99のformat_in_format_args、src/intake/mod.rs:324/467/486/505とsrc/jev.rs:315のquestion_mark、src/jev.rs:385のredundant_guards。baseline-clippy.logとfinal-clippy.logのメッセージ・位置が同じであることをverification-summary.jsonで比較した。成功扱いしていない。

途中の全体実行で今回の共通化により既存の超過タスク優先順テスト1件が失敗した。既存の並び替え処理を共通終了状態に使用して修正した。今回追加したsingle_match違反2件も修正した。最終全体実行ではこれらは再発していない。最終レビューで表示順変更の古い版が通る欠落も回帰テストで検出し、表示順を版に含めて修正した。修正前の失敗ログもred-*.log、integration-*.logとして残した。

docs/issues.mdは開始時SHA256 `66C74C67D7510F5D8DD2CAA1B57980F8ACA77EA409724AE00F56E4E94B071ACF` と照合済み（protected-work-check.json）。ユーザー所有の未追跡状態を維持する。

## 未確認・制約

Outlook/Teams実機は安全なサンプル指定がないため未確認。実データ変更、本文送信、新規外部連携、Jira、改善A/Bは実施していない。CI、配布用releaseビルド、デプロイは未実行。Jevは初回に入力形式のローカル検証エラーとなり、成功判断には使用していない。通常のCodex判断と実行証拠で検証した。Gitのpushは後続の明示許可に基づくもので、force pushは使用しない。
