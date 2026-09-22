# Windows 実機受入キット

## 目的と現在地

このキットは、release exe を用いた通知の目視、スリープ復帰、OS 再起動後の未回答保持、本人が明示した HKCU 自動起動を確認するための手順である。実行前に、対象 exe、専用 `DAYLOOP_HOME`、登録内容を本人が確認する。

2026-09-05 の隔離プロセス試験では、同一 `DAYLOOP_HOME` の二重 `serve` 起動拒否と、プロセス再起動後の 4 phase の保留・論理 ledger 保持を確認した。通知の目視、スリープ復帰、OS 再起動、HKCU 登録は **未実施** であり、この手順を実行するまでは **Unknown** である。

## 実施前の記録と確認

| 項目 | 記録 |
|---|---|
| 実施日・担当者 |  |
| Git / package の対象 |  |
| exe の絶対パス |  |
| exe SHA-256 |  |
| 専用 `DAYLOOP_HOME` の絶対パス |  |
| 専用 home が実業務データと分離されている確認 |  |
| `config.toml` の notify / schedule / intake 設定 |  |
| Windows バージョン・通知設定 |  |
| 判定 | Pass / Fail / Unknown |

専用 home は絶対パスにする。`serve --data-dir` と、必要なら環境変数 `DAYLOOP_HOME` は同じ専用パスを指す。実業務の `%LOCALAPPDATA%\dayloop`、既存の startup 登録、外部サービス接続、認証情報は使わない。

新しい専用 home を初期化する場合は、まず実行するプロセスの `DAYLOOP_HOME` をその絶対パスに設定してから、次の製品コマンドを使う。`config init` 自体には `--data-dir` オプションはない。

```text
<exe> config init
```

作成後に通知・時刻を試験目的に合わせるときは、専用 home の `config.toml` だけを本人が確認して編集する。通常利用の runtime が PowerShell、管理者権限、外部ツールを要求するものではない。`[intake] outlook = false` を保ち、外部取得は有効にしない。

## 1. 通知の目視

### 手順

1. 上表を埋め、専用 home と対象 exe を本人が確認する。
2. native notification 経路の明示試験として、対象 exe をその専用環境で次のように実行する。

   ```text
   <exe> doctor --notify-test
   ```

3. Windows の通知領域に dayloop の通知が実際に表示されたかを、本人が目視で記録する。表示時刻、文言、表示時間、Focus Assist 等の状態も残す。
4. 表示されなかった場合、専用 home の `notify.txt` と stderr を確認し、native notification 失敗後の file fallback か、別の失敗かを分けて記録する。

### 期待値と判定

| 観測 | 判定 |
|---|---|
| Windows 上で dayloop 通知を本人が視認 | Pass（目視） |
| `notify.txt` のみ生成 | file fallback は確認できるが、目視通知は Fail または Unknown |
| API が成功しても目視できない | 目視通知は Unknown。既読は証明しない |

`doctor --notify-test` は業務状態を変更しない通知試験である。通常の `serve` 通知では、朝は plan、昼は check と未解決項目、夕方は close、金曜の retro は夕方にまとめて通知候補になる。通知が表示されても、本人の回答なしに plan、close、retro、候補採用、タスク完了が確定することは期待しない。

## 2. スリープ復帰

### 手順

1. 専用 home の設定を確認し、notification を `toast`、Outlook を無効にする。
2. 同じ専用 home を明示して常駐を起動する。

   ```text
   <exe> serve --quiet --data-dir <専用DAYLOOP_HOMEの絶対パス>
   ```

3. morning / noon / evening のいずれかで、未回答の `questions[]` または `serve-state.json` の pending を観測してから、本人が Windows をスリープさせる。
4. 予定時刻をまたいで復帰し、`serve.log`、`serve-state.json`、通知の目視、必要なら `notify.txt` を記録する。
5. 同一日で未回答の項目が、復帰後に一つの catch-up 通知へ集約されるかを確認する。回答しないままなら、台帳の業務状態が完了にならないことを確認する。

### 期待値

- 設定時刻を過ぎた phase は評価される。保留は後続 slot に持ち越される。native と file fallback の両方に失敗した場合は配信済みにしない。fallback 成功はファイル配信成功として記録するが、通知の目視成功は証明しない。
- 同じ slot 内の重複通知は発生しない。
- これは Windows の実際のスリープ・復帰で観測して初めて Pass とする。プロセス再起動試験、単体テスト、時刻を手で変える試験は代用にならない。

## 3. OS 再起動後の未回答保持

1. 専用 home で `serve` を起動し、少なくとも一つの phase が未回答 pending であることを、`serve-state.json` と時刻で記録する。
2. 本人が通常の Windows 再起動を実施する。OS の強制終了や時刻変更は行わない。
3. 再ログオン後、本人が同じ exe と同じ専用 home で `serve` を起動するか、次節で登録済みの HKCU startup を使う。
4. `serve.log` の新しい起動記録、`serve-state.json` の origin date / pending、ledger の read-only 診断を記録する。

   ```text
   <exe> ledger check --json
   ```

5. 再起動前の未回答が消えず、回答したものだけが消えることを確認する。過去日の未回答と当日の新規 pending が同時にある場合は、通知に元の日付が含まれることも記録する。

OS 再起動を完了していなければ、結果は **Unknown** のままである。

## 4. HKCU startup の明示承認チェックポイント

`startup install` は HKCU Run に dayloop の値を作成するため、実施前に本人の具体的な承認が必要である。今回のキット作成自体は承認ではない。

本人に次を提示し、全てを確認してからだけ続ける。

| 確認対象 | 本人の確認 |
|---|---|
| 対象 exe の絶対パスと SHA-256 |  |
| 専用 `DAYLOOP_HOME` の絶対パス |  |
| 登録される内容: `"<exe>" serve --quiet --data-dir "<専用home>"` |  |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` の値名 `dayloop` |  |
| 既存の `dayloop` 値を上書きしないこと |  |
| OS 再起動後にこの専用 home で常駐が起動することへの明示承認 |  |

確認後の製品コマンドは次の順である。

```text
<exe> startup status
<exe> startup install
<exe> startup status
```

`startup install` は別の登録を上書きせず終了する。管理者権限は不要で、HKCU 以外や Startup folder は使わない。再起動後の確認が終わったら、本人が解除を指示した場合にだけ同じ exe と同じ専用 home で次を実行する。

```text
<exe> startup remove
<exe> startup status
```

`startup remove` は期待した exe / home と一致する自身の登録だけを削除し、別の登録を削除しない。

## 記録テンプレート

| 試験 | 実施 | 判定 | 証拠ファイル・画面 | 失敗・復旧 | 備考 |
|---|---|---|---|---|---|
| native notification 目視 |  |  |  |  |  |
| morning plan pending |  |  |  |  |  |
| noon check pending |  |  |  |  |  |
| evening close / Friday retro |  |  |  |  |  |
| スリープ復帰 catch-up |  |  |  |  |  |
| OS 再起動後 pending |  |  |  |  |  |
| HKCU status → install → status |  |  |  |  |  |
| HKCU remove → status |  |  |  |  |  |

実施していない項目は空欄にせず **Unknown** と書く。合成台帳の結果と実業務データの結果を同じ行に混ぜない。
