#!/usr/bin/env bash
# Smoke test for dayloop stage 1. Uses an isolated DAYLOOP_HOME.
set -u
export DAYLOOP_HOME="$(dirname "$0")/dl-home"
rm -rf "$DAYLOOP_HOME"
ROOT="$(dirname "$0")/.."
if [ -n "${DAYLOOP_BIN:-}" ]; then
  BIN="$DAYLOOP_BIN"
elif [ -x "$ROOT/target/release/dayloop" ]; then
  BIN="$ROOT/target/release/dayloop"
else
  BIN="$ROOT/target/release/dayloop.exe"
fi
D1=2026-09-01   # Tue (a past day, so plan on D2 must force-close it)
D2=2026-09-02
D3=2026-09-03
fail=0
step() { echo; echo "### $*"; }
expect_code() { local want=$1; shift; "$@"; local got=$?; if [ "$got" != "$want" ]; then echo "!!! expected exit $want, got $got"; fail=1; fi; }

step "1. add tasks on $D1"
"$BIN" add "仕様書レビュー" --due $D2 --date $D1
"$BIN" add "PR #12 を直す" --estimate 60 --date $D1
"$BIN" add "経費精算" --date $D1
"$BIN" add "いつかやる調査" --backlog
"$BIN" today --date $D1

step "2. close --yes must fail with exit 2 (3 open)"
expect_code 2 "$BIN" close --date $D1 --yes

step "3. reason required for notdone"
expect_code 1 "$BIN" notdone dummy --reason ""

ID1=$("$BIN" today --date $D1 | grep '仕様書レビュー' | awk '{print $1}')
ID2=$("$BIN" today --date $D1 | grep 'PR #12' | awk '{print $1}')
ID3=$("$BIN" today --date $D1 | grep '経費精算' | awk '{print $1}')
echo "ids: $ID1 $ID2 $ID3"

step "4. done / notdone(reason) / carry via CLI"
"$BIN" done "$ID1" --evidence "https://example/pr/12"
expect_code 1 "$BIN" notdone "$ID2" --reason "   "
"$BIN" notdone "$ID2" --reason "レビュー待ちで着手できず"
"$BIN" carry "$ID3" --reason "領収書が届いていない"

step "5. close $D1 now succeeds"
expect_code 0 "$BIN" close --date $D1 --yes
"$BIN" today --date $D1

step "6. candidates: add, dedupe by ref, reject blocks re-add"
"$BIN" candidates add "田中さんに返信" --source teams --ref teams:msg:1
"$BIN" candidates add "田中さんに返信(重複)" --source teams --ref teams:msg:1
"$BIN" candidates add "月次報告を送る" --source outlook --ref mail:99
"$BIN" candidates list
CID=$("$BIN" candidates list | grep '月次報告' | awk '{print $1}')
"$BIN" candidates reject "$CID"
"$BIN" candidates add "月次報告を送る(再)" --source outlook --ref mail:99

step "7. plan $D2 interactively via piped answers"
# carried task from D1 is already planned on D2.
# prompts: candidate 田中 -> 1 (today), backlog いつかやる -> 1 (today), confirm -> 1
printf '1\n1\n1\n' | DAYLOOP_INTERACTIVE=1 "$BIN" plan --date $D2
expect_code 0 true
"$BIN" today --date $D2

step "8. check: start one, carry one"
IDC=$("$BIN" today --date $D2 | grep '経費精算' | awk '{print $1}')
IDT=$("$BIN" today --date $D2 | grep '田中さん' | awk '{print $1}')
"$BIN" start "$IDT"
printf '1\n' | DAYLOOP_INTERACTIVE=1 "$BIN" check --date $D2   # いつかやる -> 今日中にやる
"$BIN" today --date $D2

step "9. carry limit: carry 経費精算 up to 3 then 4th is blocked"
"$BIN" carry "$IDC" --reason "2回目" --to $D3
IDC2=$("$BIN" today --date $D3 | grep '経費精算' | awk '{print $1}')
"$BIN" carry "$IDC2" --reason "3回目" --to 2026-09-04
IDC3=$("$BIN" today --date 2026-09-04 | grep '経費精算' | awk '{print $1}')
expect_code 1 "$BIN" carry "$IDC3" --reason "4回目" --to 2026-09-07
"$BIN" carry "$IDC3" --reason "期限変更" --to 2026-09-07 --reschedule 2026-09-10
"$BIN" today --date 2026-09-07

step "10. split"
IDS=$("$BIN" today --date 2026-09-07 | grep '経費精算' | awk '{print $1}')
"$BIN" split "$IDS" --reason "大きすぎる" --into "領収書を集める" --into "申請フォーム入力"
"$BIN" today --date 2026-09-08

step "11. markdown import: tick [x] on 田中 in $D2 md and add a new line"
MD="$DAYLOOP_HOME/days/$D2.md"
cat "$MD"
sed -i "s/^- \[ \] 着手中 田中さんに返信/- [x] 田中さんに返信/" "$MD"
printf -- '- [ ] Markdownから追加したタスク\n' >> "$MD"
# the appended line lands after 状態 section; move it under 今日のタスク instead:
python - "$MD" <<'EOF'
import sys,io
p=sys.argv[1]; s=io.open(p,encoding='utf-8').read()
s=s.replace('- [ ] Markdownから追加したタスク\n','')
s=s.replace('## 今日のタスク\n','## 今日のタスク\n- [ ] Markdownから追加したタスク\n',1)
io.open(p,'w',encoding='utf-8').write(s)
EOF
"$BIN" import --date $D2
"$BIN" today --date $D2

step "12. close $D2 interactively: remaining open -> answers"
# open on D2 (created order): いつかやる調査, 月次報告, Markdownから追加. 田中 done via import.
printf '1\n2\n理由テスト\n1\n' | DAYLOOP_INTERACTIVE=1 "$BIN" close --date $D2
expect_code 0 "$BIN" close --date $D2 --yes

step "13. retro for week of $D2 (non-interactive)"
"$BIN" retro --date $D2 --yes

step "14. doctor"
"$BIN" doctor

step "15. plan $D3 must not be blocked (D2 closed); 2026-09-04 has open task"
printf '1\n' | DAYLOOP_INTERACTIVE=1 "$BIN" plan --date $D3

echo
echo "=== md for $D2 ==="
cat "$DAYLOOP_HOME/days/$D2.md"
echo
if [ $fail = 0 ]; then echo "SMOKE: ALL EXPECTATIONS MET"; else echo "SMOKE: FAILURES"; fi
