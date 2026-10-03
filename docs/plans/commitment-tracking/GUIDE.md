# 相手待ち・依頼変更の使い方

約束は日別タスクから独立した台帳です。`waiting` → `replied` → `settled` または `cancelled` を区別し、返答で自分の作業を自動完了にしません。関連タスクは持ち越し元のIDを保持し、持ち越し先も参照します。待ちが解けるまで進められない作業は `blocked_task_ids` に登録すると `next` の対象から外れます。日別の予定に入れた作業は既存の日次確定が必要です。毎日の持ち越しを避ける場合は、依存作業を未計画に置き、約束の確認予定を使います。

## 安全なローカルサンプル

PowerShellで専用の空フォルダを指定します。`DAYLOOP_HOME` は実データのフォルダに向けないでください。

```powershell
$env:DAYLOOP_HOME = Join-Path $env:TEMP ('dayloop-sample-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $env:DAYLOOP_HOME | Out-Null
'[jev]`nmode = "off"' -replace '`n', "`n" | Set-Content -Encoding utf8 (Join-Path $env:DAYLOOP_HOME 'config.toml')
.\target\debug\dayloop.exe intake fixture .\fixtures\commitments
.\target\debug\dayloop.exe commitment list
```

`fixtures/commitments/commitments.json` は新規依頼の提案です。台帳の質問で、本人が適用または却下を決めます。既存のfixture入口を利用し、メール・予定用JSONは空配列です。更新通知はこのJSON配列に追加できます。`source` と `external_id` は元情報のイベントごとに固定します。再取込は重複せず、同じ識別子で内容が変わると競合として保留します。

## CLIとMCPの共通入力

CLIは `dayloop commitment <register|list|ingest|apply|check|history|sync> --file <UTF-8 JSON>` を使います。MCPは同じ名前に `commitment_` を付けます。`--json` も使えます。Windowsで日本語や引用符を含むときは `--file` が扱いやすいです。

- register: `op_id,request,counterparty,next_check,source,evidence` が必須。`reply_due,source_ref,related_task_ids,blocked_task_ids` は任意。
- ingest: `source,external_id,kind,occurred_at,evidence`。kindは `new_request,additional_info,deadline_change,cancellation,reply`。既存IDが不明なら候補を返し、件名だけで結び付けません。
- apply: `op_id,update_id,expected_revision,confirmed:true`。未照合なら本人が選んだ `commitment_id` も必要。新規は版0。却下は `action:reject,evidence` を加えます。
- check: `op_id,commitment_id,expected_revision,confirmed:true,action,evidence`。`defer` または `acknowledge` では未来の `next_check` が必須。返答後に別途追加した作業を `next_task_id` で結び付けられます。`settle` は根拠を残して決着。
- history: `commitment_id`。変更前後、元情報、判断日時を確認します。
- sync: `op_id,source,status,at,evidence`。statusは `success,partial,failed,unavailable`。

日時はタイムゾーン付きRFC3339、IDは完全ID、版番号は整数です。`op_id` は別の操作で新しくし、同じ内容の再試行だけ再利用します。重要な変更は無回答やAIの分類だけで確定しません。現状態が変わった場合は版番号を再取得し本人に再提示します。

`plan/check/close` とMCPの日次ツールが期限到来・未確定の変更提案を提示します。過去日はその日の23:59:59.999999999 JSTまで、今日と未来日の表示は実時刻で評価します。翌日00:00は前日の範囲に含めず、確認日時と一致した時点で質問を出します。未来日を指定しても時計を進めません。正確な時刻を検証する場合は `commitment_list {at: RFC3339}` を使います。次回確認前は通常の待ち確認を出しません。未確定の通知提案は先に確認できます。

日次終了の `closed` は閉じた事実です。現在の未回答は `pending_count` と `questions` で確認します。閉じた後に確認事項が生じると `closed:true,already_closed:true,pending_count:1` のように返します。CLIも「クローズ済み」と「閉じた後の未回答」を分け、未回答があれば終了コード2、なければ0です。元の `closed_at` と日別Markdownは再終了で更新せず、以後の約束変更は新しい判断履歴として残します。

`plan_day` は必要な確認事項の後に確定質問を返します。回答後に計画を再表示し、本人が最後の確定質問に明示回答したら、その `options[].args` に含まれる `date` と `plan_revision` を `confirm_plan` に渡してください。約束・変更提案が未回答、表示後に内容が変わった、または版が未指定なら確定を拒否します。CLIも表示時の版を取得し、本人が答える間の変更を検査します。計画確定日時は検査と同じ書込トランザクション内で記録するため、質問順だけで安全性を保証しません。

既存アダプタの取込成功は取得範囲の完全性を保証しないため `partial` として同期記録を残します。fixtureの全範囲が成功した場合だけ `success` とします。失敗や部分取得は返答なし・変更なしの証明にはなりません。今回、Outlook/Teamsの実機から変更分類や照合する新しい連携は追加していません。既存の候補の元参照を手入力の約束へ引き継げます。

## 実使用の測定と後続計画

追いかけ漏れは、登録した約束のうち次回確認時刻を過ぎて未確認の件数と最長経過時間で測ります。historyの確認・延期・決着日時、次回確認と版番号を根拠にします。週1回、実際の依頼一覧と約束IDを照合し、未登録の漏れも別に数えます。同期失敗・部分取得は母数不明として別に記録します。

確認にかかる時間は、質問を表示した時刻から本人の判断時刻までの時間と、質問件数・延期件数・同一約束への再確認件数を記録します。現在の履歴は判断日時を保存しますが、質問表示時刻は保存しないため、測定時はクライアント側で開始時刻を記録します。

A（今日引き受けられる量）は関連タスクと既存予定・見積を入力に、残り勤務時間や休憩を追加して検討します。不明な見積・予定は不明として残します。B（中断からの再開）は返答の根拠と `next_task_id` の接続を利用し、到達点・次の一手・資料・完了条件を作業へ添える設計につなげます。今回はA/Bを実装していません。
