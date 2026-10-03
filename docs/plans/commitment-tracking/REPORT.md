# Local implementation and verification report (2026-10-03 JST)

Historical initial-implementation evidence only. The three review regressions were reproduced after this report; its 131-test result and midnight-JST limitation are not current acceptance evidence. See [review fixes and current verification](REVIEW-FIXES-20261003.md).

## Outcome

Stable promise IDs survive daily close, task carry and process restarts. Source updates remain proposals until the person confirms. The due flow includes evidence and revisions. Reply receipt is separate from task completion; next work can be linked and final settlement has evidence. Unlinked/no-match proposals can be explicitly rejected without creating or changing a promise.

## Baseline and protected work

Initial branch: codex/requirements-gap, HEAD 214add69. Initial untracked user file docs/issues.md was not edited. Related worktree: C:/Users/awano/.codex/worktrees/dayloop-trust-core/dayloop at 10c76b5. git merge-base --is-ancestor 10c76b5 HEAD returned non-ancestor. Inspected git diff HEAD 10c76b5 and ported the task/date transaction, Markdown, body-consent and intake-identity safeguards locally. No merge, cherry-pick, commit or push.

Initial cargo test --locked --offline --no-fail-fast returned failure in intake_pipeline; its detailed failure text was not retained in the truncated output. Immediate targeted cargo test --locked --offline --test intake_pipeline -- --nocapture passed both tests. The baseline failure cause remains unconfirmed. Do not present it as fixed by this feature.

## Changed areas

- src/commitment.rs: additive transactional schema, stable promise IDs, UTC timestamps, source identity, payload conflict checking, replay IDs, human confirmation/revision checks, source and local schedule freshness, immutable history and sync, task-root/descendant links.
- src/store.rs: migration hook, trust-core safeguards, due-confirmation close guard under BEGIN IMMEDIATE, repeat close keeps historical timestamp.
- src/engine.rs, tools.rs, main.rs, rituals.rs: shared CLI/MCP ledger tools, daily questions, explicit confirmation, quiet future waiting.
- src/order.rs, graph.rs, jev.rs, observe.rs: waiting blocked tasks excluded from next; unresolved related tasks protected from automatic title decisions and observations.
- src/intake/mod.rs and fixtures/commitments: existing fixture pipeline, manual proposals, failed/partial source-sync records. No new live integration.
- src/browse.rs, screen.rs, markdown.rs and trust regression tests: inspected trust-core protection port.
- tests/commitment_ledger.rs and commitments.rs: migration/rollback, state rules and real CLI/MCP processes with isolated child environments.
- docs/design.md, guide.md, requirements.md plus this task directory: plan, prompt, operating constraints and follow-up links.

## Final commands and results

| Command | Result |
|---|---|
| cargo test --locked --offline --no-fail-fast | PASS, 131 passed, 0 failed, exit 0. target/commitment-final-tests.log |
| cargo test --locked --offline --test commitments --test commitment_ledger | PASS, 9 process acceptance + 13 ledger tests. target/commitment-focused-tests.log |
| cargo test --offline --test invariants --test intake_pipeline --test work_steps | PASS, 16 trust regression tests (worker evidence) |
| python target/commitment-smoke.py | PASS; isolated saved ledger plus command/result JSON |
| git diff --check | PASS; only Git line-ending normalization notices |
| cargo clippy --locked --offline --all-targets -- -D warnings | FAIL, exit 101: 7 diagnostics in existing devops/intake/jev statements; one newly introduced commitment cmp_owned finding was corrected. target/commitment-clippy-final.log |

Clippy remaining locations: devops.rs nested format; intake/mod.rs four preexisting Option guards; jev.rs Option guard and redundant status guard. These source statements predate this change; baseline Clippy command was not rerun against HEAD separately. No strict-Clippy success claim.

## Saved isolated ledger evidence

Directory: C:\Users\awano\新規プロダクト\dayloop\target\commitment-verification-4913dcad-9fec-4d29-b694-6937121b5a24

See evidence.json for executed process commands, inputs, responses and final state; dayloop.db is retained for inspection. Synthetic scenario timestamps intentionally use 2099 so future checks are deterministic; audit timestamps use the actual local clock.

- Promise ID 01M3YK7W9JAGSE77AJS5GD0Z7A: settled, revision 6.
- Decisions 6, source updates 2.
- Next task 01M3YK7WSCRR6PNVS0RQ4VGXQA remains backlog (reply did not complete it).
- Day 2026-10-03 closed before the changes; subsequent changes were separate append-only decisions.
- Each CLI call launched a new process; an MCP stdio call created the next task in the same isolated ledger.

## Acceptance coverage

Registration/wait/deadline change/due check/reply/next work/settlement; close/restart persistence; before/exact/after due boundaries; import and operation replay; same-title isolation; reply does not finish; old source/local schedule guard; ambiguous/no-match rejection; failed/partial reads; stale screen revision; immutable historical records; CLI/MCP validation parity; populated migration and rollback failure all passed in isolated tests.

Two failures during development were distinguished: the initial baseline intake_pipeline failure above, and the new acceptance test revealing automatic learned carry of a waiting task. The latter was corrected with active-promise task protection and its acceptance regression now passes. The initial wrapper mismatch in a draft process test was corrected to use the documented commitment result field.

## Limits and follow-ups

Outlook/Teams live UI, real corporate PC permissions, new live semantic classification, Jev classification calibration, and release packaging were not tested. Jira development, capacity analysis A and resume support B were not implemented. No production ledger or external service was changed.

A promise itself never requires daily carry. A blocked task already placed in a daily plan still needs explicit daily disposition to preserve the original task closure invariant. Keep dependency work in backlog while waiting to avoid repeated scheduling questions. Daily view for today uses real time; a different explicit date is a midnight-JST snapshot. Use commitment_list.at for exact timestamp checks.

GUIDE.md describes measurement: overdue unconfirmed count/oldest age, separate unregistered promises and sync coverage unknowns; client timestamp from question display to immutable decision time for confirmation effort. No automatic display-time telemetry was added.

Jev: existing TypeSafe skill used for initial effort judgment only, recommended medium / stuck 0.05 / confidence 0.66 / shadow; model and effort were not changed. Manual core functionality and all fixture acceptance are independent of Jev.
