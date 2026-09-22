# このPCでのWindows受入試験・実施前確認

作成: 2026-09-06。以下は実施する内容の提示であり、未実施の操作を完了扱いにしない。

## 対象

- 実行ファイル: `C:\Users\awano\新規プロダクト\dayloop\target\release\dayloop.exe`
- SHA-256: `C1160AE5C480A05386EE42C3B3E10FCB73990695E55C0BEDA94EA3077270B6DE`
- 業務操作に使う専用データ領域: `C:\Users\awano\新規プロダクト\dayloop\target\verification\client-acceptance-20260906\lmstudio-204633\data`
- LM Studioの接続名: `dayloop-acceptance-20260906`。同じデータ領域を明示済み。
- 通知の単発試験用データ領域: `C:\Users\awano\新規プロダクト\dayloop\target\verification\windows-acceptance-20260906\data`

外部サービス・クラウドへの開示は無効のまま。検証には合成データを使用する。

## 自動起動を有効化する場合

承認済み計画の「自動起動を本人が有効化した場合だけHKCU登録」という条件に従い、本人がこの内容での有効化を明示した後に実施する。

- キー: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
- 値名: `dayloop`
- 2026-09-06 21:02の読み取り確認: 登録なし。
- 登録する文字列:

```text
"C:\Users\awano\新規プロダクト\dayloop\target\release\dayloop.exe" serve --quiet --data-dir "C:\Users\awano\新規プロダクト\dayloop\target\verification\client-acceptance-20260906\lmstudio-204633\data"
```

登録直前に再度 `startup status` を確認する。`DAYLOOP_HOME` を上の業務操作用の専用領域として、製品の `startup install` と `startup status` を実行する。別の既存登録があれば上書きしない。PowerShellスクリプトやStartupフォルダへの登録は行わない。

登録だけではPCをスリープ・再起動しない。実際のスリープ・再起動は、作業中のアプリを保存した上で本人と実施時刻を合わせる。

## 実機で観測すること

1. `doctor --notify-test` の通知を本人が目視できること。API受付と目視を分けて記録する。
2. 未回答を残してスリープし、通知予定時刻をまたいだ復帰時に未回答が保持・集約されること。
3. 未回答を残してOSを再起動し、再ログオン後に同じ台帳を読み込むこと。
4. 自動起動を有効化した場合、登録したexeと専用領域で起動すること。

手順全体は [Windows受入キット](WINDOWS_ACCEPTANCE.md)、継続利用の記録は [5営業日トライアル](FIVE_DAY_TRIAL.md) を使う。

現時点ではHKCU登録、OSスリープ・再起動、5営業日の実利用を実施していない。`target` 以下のexe・台帳を使う間は、試験終了まで `cargo clean` や同フォルダの削除を行わない。
