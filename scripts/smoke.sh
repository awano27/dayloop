#!/usr/bin/env bash
# Isolated executable smoke. Temporary evidence is retained; existing folders are never removed.
set -euo pipefail
BIN="${DAYLOOP_BIN:-$(dirname "$0")/../target/release/dayloop.exe}"
SMOKE_DIR=$(mktemp -d -t dayloop-smoke-XXXXXXXX)
if command -v cygpath >/dev/null 2>&1; then
  export DAYLOOP_HOME="$(cygpath -w "$SMOKE_DIR")"
else
  export DAYLOOP_HOME="$SMOKE_DIR"
fi
expect_code() {
  local want=$1; shift
  local got=0
  "$@" || got=$?
  if [ "$got" -ne "$want" ]; then
    echo "Expected exit $want, got $got" >&2
    exit 1
  fi
}
"$BIN" setup
expect_code 2 "$BIN" plan --date 2026-09-07 --yes
expect_code 2 "$BIN" close --date 2026-09-07 --yes
for category in teams outlook attendance tasks alerts meeting_prep meeting_results; do
  "$BIN" reviews record "$category" not_checked --reason "synthetic CI; service not connected" --date 2026-09-07
done
"$BIN" note --category meeting_results --title "CI meeting" --body "ACTION: review the meeting notes" --meeting-id ci-meeting
"$BIN" candidates list
"$BIN" close --date 2026-09-07 --yes
expect_code 1 "$BIN" add "closed-day write must fail" --date 2026-09-07
"$BIN" retro --date 2026-09-07 --note "synthetic CI retrospective"
"$BIN" backup
"$BIN" ledger check --json
"$BIN" doctor
echo "SMOKE: ALL EXPECTATIONS MET (evidence: $SMOKE_DIR)"
