# dayloop

1日のタスクを、朝に計画し、昼に確認し、夕に確定する。できたのか、忘れているのかは LLM ではなく台帳が決める。未確定が残る日は閉じない。

A local Windows CLI for that loop. The ledger decides whether the day is done. An LLM is optional.

https://github.com/awano27/dayloop

管理者権限は要らない。書き込みは `%LOCALAPPDATA%\dayloop` だけ。Copilot や LM Studio を繋いでも、無くても回る。

## 1日

```bash
dayloop plan     # 朝
dayloop next     # 次の1件
dayloop check    # 昼
dayloop close    # 夕。未確定が残ると閉じない
dayloop retro    # 週末
```

前日が開いていれば、翌朝はそこから始まる。3回持ち越したタスクは、分割・取り下げ・期限変更まで止まって残る。

コマンドの一覧、終了コード、設定は [docs/guide.md](docs/guide.md)。

## 試す

```bash
cargo build --release
```

`target/release/dayloop.exe` を好きなフォルダに置く。SmartScreen が出たら「詳細情報 → 実行」。昇格は不要。配布用の zip はまだ無い。

## 動く範囲

- Windows 10 / 11
- クラシック Outlook があるときだけ、メールと予定を読む。無くても計画・確認・クローズ・MCP・常駐は動く
- ネットワークには送らない。パスワードもトークンも持たない
- Outlook へは書かない。本文とメールアドレスは既定で読まない

判断の記録は [docs/decisions.md](docs/decisions.md)。MCP のつなぎ方は [docs/mcp-clients.md](docs/mcp-clients.md)。

## この先

今あるのは CLI、MCP、常駐、Outlook の読み取りまで。人が答えたことは次から質問しない。同じ状況の枝が無いときだけ Jev が初回の判断をする。開いたタスクが残る日を閉じる権限は台帳のまま。

予定は [docs/plans/2026-09-22-roadmap.md](docs/plans/2026-09-22-roadmap.md)。判断の残し方は [docs/analysis/05-decision-graph.md](docs/analysis/05-decision-graph.md)。
