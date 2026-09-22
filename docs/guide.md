# dayloop でできることと手順

1日のタスクを、朝の計画、昼の確認、夕の確定で回す。忘れの有無は台帳が保証する。判断グラフが同じ件名の2回目以降を自動で進め、Jev は枝の無い節だけを見る。

データの場所は `%LOCALAPPDATA%\dayloop`。変えるときは環境変数 `DAYLOOP_HOME` を置く。

## できること

- 今日のタスクを、期限超過、持ち越し回数、空きに収まるか、見積の短さの順に並べ、次の1件を出す。
- 朝に候補と未計画を見て予定を確定し、昼に未着手を確認し、夕に全件を完了・未完了・持ち越し・取り下げにするまで日を閉じない。
- 前日が開いたままなら、翌朝はそちらを先に閉じる。持ち越しは次の営業日に新しいタスクを作り、3回で止まる。
- Outlook のメールと予定、議事録、チケットと Teams のフィクスチャ、手入力、Markdown から候補やタスクを入れる。
- チケットが閉じている、PR がマージされている、会議の終了時刻を過ぎている、のどれかを読めたときだけ、質問なしで完了にする。
- 同じ件名や状況に過去の答えがあれば、次からは質問しない。
- Jev を有効にすると、未知の候補とタスクを自動で渡し、確度が足りる答えを枝にする。
- MCP から同じ朝昼夕を呼べる。定時に常駐させることもできる。

GitHub、Jira、Teams の実 API は呼ばない。チケットと Teams は JSON ファイルを読む。

## 準備

```powershell
cargo build --release
copy target\release\dayloop.exe $env:LOCALAPPDATA\dayloop\
dayloop config init
dayloop doctor
dayloop where
```

`config init` は `%LOCALAPPDATA%\dayloop\config.toml` を作る。Jev は既定で `mode = "off"` なので、このままでも台帳だけで動く。

ID は表示の末尾8文字で指定できる。終了コードは 0 が完了、1 がエラー、2 が未回答あり。

## 1日の手順

```powershell
dayloop plan      # 朝
dayloop next      # 次の1件
dayloop check     # 昼
dayloop close     # 夕
dayloop retro     # 週末
dayloop today     # いまの台帳を見る
```

朝は、前日の未確定、候補、未計画、今日の予定の順に聞く。最後に「この内容で確定しますか」と出る。前日が残っているあいだは、今日の確定はできない。

昼は、まだ着手していないタスクを並び順に聞く。

夕は、残ったタスクを完了、未完了、持ち越し、取り下げのいずれかにする。1件でも未回答だと日は閉じない。閉じると `days\YYYY-MM-DD.md` が書き出される。

非対話で回すときは `--yes` を付ける。未回答は残ったまま終了コード 2 になる。定時実行は `dayloop serve` が、平日の 08:30 / 13:00 / 18:00 と金曜 18:30 に同じことをする。

```powershell
dayloop serve
dayloop startup install
dayloop startup status
```

## タスクを手で動かす

```powershell
dayloop add "仕様書レビュー" --due 2026-09-25 --estimate 30
dayloop add "いつかやる調査" --backlog
dayloop start 4CFD6BRC
dayloop done 4CFD6BRC --evidence "https://github.com/awano27/dayloop/pull/12"
dayloop notdone 4CFD6BRC --reason "レビュー待ち"
dayloop carry 4CFD6BRC --reason "領収書未着"
dayloop carry 4CFD6BRC --reason "期限見直し" --reschedule 2026-09-30
dayloop split 4CFD6BRC --reason "大きすぎる" --into "領収書を集める" --into "申請入力"
dayloop drop 4CFD6BRC --reason "不要になった"
dayloop revise 4CFD6BRC drop --reason "不要"
```

`revise` の choice は `done`、`not_done`、`carry`、`drop`、`start`、`today`、`backlog`、`shelve`、`reject`。これが判断グラフの枝になる。同じ件名は次の日から質問にしない。

理由は `待ち`、`ブロック`、`時間不足`、`不要`、`大きすぎる`、またはそのコード `waiting`、`blocked`、`no_time`、`not_needed`、`too_big`。

## 並び

今日の未完了は次の順になる。

1. 期限超過、今日、期限なし、未来
2. 持ち越し回数が多いもの
3. 空きに収まるもの、見積が不明なもの、空きに収まらないもの
4. 見積が短いもの

```powershell
dayloop next --date 2026-09-23
dayloop prefer "乙" "甲"
```

`prefer` は同順位の2件について、先にやる方を覚える。期限の違う2件は入れ替わらない。

## 外から入れる

```powershell
dayloop intake outlook
dayloop intake outlook --since 3d --dry-run
dayloop intake fixture fixtures\outlook
dayloop intake note .\minutes.txt
dayloop intake tickets .\tickets.json
dayloop intake teams .\teams.json
dayloop candidates list
dayloop candidates add "田中さんに返信" --source teams --ref teams:msg:123
```

議事録は、次の行だけが候補になる。

```text
決定事項
- 見積を直す
決定: 会場を押さえる
TODO: 請求書を送る
アクション: 本番を確認する
```

`共有:` と `情報:`、見出し `共有事項` の箇条書きは候補にしない。それ以外の行は無視する。

チケット JSON:

```json
[{"source_ref":"ticket:ABC-1","title":"請求書を送る","state":"open"}]
```

Teams JSON:

```json
[{"source_ref":"teams:msg:1","title":"本番の確認をお願いします"}]
```

`state` は `open` か `closed`。取り込み時点では完了にせず、候補にする。完了は下の証跡が読む。

Outlook はクラシック Outlook の COM を使う。本文とメールアドレスは既定で読まない。新しい Outlook だけが入っている PC では、1行のメッセージを出して終了コード 0 で戻る。

## 証跡で完了にする

`config.toml` に観測ファイルを書く。

```toml
[observe]
fixture = "C:/Users/you/dayloop/observations.json"
```

```json
[
  {"source_ref":"ticket:ABC-1","state":"closed"},
  {"source_ref":"github:pr:awano27/dayloop#12","state":"merged"}
]
```

`closed` と `merged` の参照を持つ未完了タスクだけを、朝昼夕の前に質問なしで完了にする。`open` や、参照の無いタスクはそのまま聞く。この完了は判断グラフの枝にしない。同じ件名が翌日に出ても、自動では完了にしない。

会議準備は `outlook:cal:<予定ID>:prep` という参照を持つ。台帳にある予定の終了時刻を過ぎていれば、同じように完了にする。フィクスチャは要らない。

## Jev を使う

既定では呼ばない。使うときは `config.toml` をこうする。

```toml
[jev]
mode = "on"
route = "https://api.example/v1/decide"
timeout_ms = 2000
send_body = false

[minutes]
generator = "rules"
```

環境変数 `DAYLOOP_JEV_API_KEY` に鍵を置く。本文は送らない。送るのは正規化した件名、期限、空き、持ち越し回数と、閉じた選択肢だけである。

`mode = "on"` だと、取り込みの直後と朝昼夕の前に、枝の無い候補とタスクを Jev に渡す。確度が `commit_confidence` 以上の答えは枝にして台帳へ書く。項目が空なら下限は 0.5。`ask`、下限未満、呼び出し失敗は人の質問として残る。

議事録の印の無い行も Jev に見させるときは `generator = "llm"` にする。選択肢は `決定`、`共有`、`無視` だけである。確度が足りる `決定` だけが候補になる。

同順位の2件は「どちらを先か」だけを聞き、確度が足りれば `prefer` と同じ枝を書く。

帯の実測は次のとおり。20件の表はリポジトリの `fixtures/jev-band-20.json` にあり、人の手だけが入っている。

```powershell
dayloop jev-eval
dayloop jev-eval --live
```

`--live` が無いときは件数 0 と「下限はまだ書かない」を出す。設定ファイルは変えない。実測した下限を使うときは、人が `commit_confidence` を書く。

## Markdown

`days\YYYY-MM-DD.md` の `- [ ]` を `- [x]` にして `dayloop import` すると完了になる。`## 今日のタスク` の下に `- [ ] 新しい行` を足すとタスクになる。

## MCP

```powershell
dayloop mcp
```

stdio の JSON-RPC。クライアント設定は [mcp-clients.md](mcp-clients.md)。`plan_day`、`check_in`、`close_day` は、質問を返す前に証跡と既知の枝を適用する。`get_today` は読むだけで、台帳を変えない。
