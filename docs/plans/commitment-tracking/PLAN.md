# Dayloop commitment tracking PLAN

Goal: preserve unresolved promises across daily closure; prompt only when due; track changes/cancellations/replies through explicit evidence-backed settlement. Local implementation and isolated verification only. Jira excluded.

Baseline: codex/requirements-gap at 214add6, user-owned untracked docs/issues.md preserved. trust-core 10c76b5 is not an ancestor; inspect its delta and port relevant safeguards without merge/cherry-pick. Baseline offline test suite started before edits.

Design: independent durable commitments, append-only source updates and decisions, stable ULID, revision compare-and-swap. RFC3339 timestamps normalized to UTC. Task links resolve carried descendants; waiting and blocked work distinct. Incoming changes remain proposals until explicit confirmed apply. Source identity comprises source plus external event ID, never title. Old/equal conflicting source timestamps cannot replace newer state. Due confirmations are live revision-bound questions; acknowledgment moves next check or settles, never completes task automatically.

Dependencies/ownership:
1. Root: integration, engine/tools/CLI/ritual wiring, fixture pipeline, docs and acceptance evidence.
2. Ledger worker: src/commitment.rs and model types in that module, migration hook in src/store.rs, isolated ledger tests. No existing task behavior changes.
3. Trust worker: inspected 10c76b5 safeguards in store/markdown/screen/browse and their existing tests. Preserve body consent and source identity; coordinate store hook.
4. Acceptance worker after ledger API ready: tests/commitments.rs CLI/MCP and lifecycle checks.

Risks: closed-day mutation (retain trust guards, decisions get actual new timestamp); duplicate replay (unique identities + payload comparison); stale questions (revision CAS); silent ingestion failure (persist source sync status); task carry identity (root link/descendants); ambiguous requests (no title auto-link, user selects exact ID); migrations (additive transactional schema plus populated legacy DB test). No external sends or real data modifications.

Acceptance: register/wait/reschedule/check/reply/next work/settle; persistence across close/reopen; no questions before deadline and no missing overdue; import/apply idempotence; same-title separation; reply does not finish; stale/conflicting/ambiguous/failed input safe; outdated screen rejected; closed history immutable; CLI/MCP same validators. cargo test --locked --offline --no-fail-fast and diff check; isolated process CLI/MCP smoke evidence. Record baseline vs introduced failures explicitly.

Follow-up only: A capacity analysis consumes linked tasks, estimates, calendar and unknowns; B resume packet attaches next-step/materials/completion criteria to linked task and reply decision. Neither implemented here.

## Completion evidence

Local implementation integrated; final offline suite 131 passed, 0 failed. Ledger tests 13 and process acceptance tests 9 included. Saved synthetic ledger and command JSON are referenced by REPORT.md. diff check passed. Strict Clippy remains failed on 7 existing source statements. Outlook/Teams real clients, A/B follow-ups, real data and external publication remain outside this run. docs/issues.md preserved. No commit or push.

## Review regression fixes — 2026-10-03

Scope: fix the three reproduced gaps in daily commitment deadlines, repeat closure, and plan confirmation in the current dirty checkout. Preserve all pre-existing changes, especially docs/issues.md. The completion evidence above is historical and will not count as this run's verification. Snapshot current sources/diff before editing. No commit, push, publication, real ledger access, external sending, Jira, new integration, or A/B implementation.

Sequence/dependencies: (1) inspect the shared ledger and CLI/MCP paths and run the current offline suite and strict Clippy; (2) reproduce all three problems with fresh DAYLOOP_HOME directories, synthetic data, Jev off and child credentials removed; (3) add failing regressions, then fix the common deadline/closure/confirmation processing; (4) integrate both callers, re-run process acceptance, existing commitment/task/migration/trust-core tests and the complete offline suite; (5) compare changes against the initial snapshot and verify docs/issues.md is byte-identical. Independent test authoring may be delegated after the ledger contract is decided; root owns design, integration and review.

Rules to specify and test: past dates include deadlines reached anywhere in that JST day, with the next day's midnight excluded; today uses a single captured actual instant, with equality due; future dates never advance the real clock to expose future waiting checks. Closed facts (closed_at and original history) remain immutable, while later unresolved confirmations remain visible as separate current state. CLI and MCP consume the same closure decision. Plan confirmation must re-check unresolved commitments and proposals in the shared ledger transaction, and reject a stale displayed plan. Confirmation questions follow prerequisite questions.

Risks/mitigations: retain tracking IDs, next checks and carried task links; preserve revision/op/source/time deduplication, explicit decisions and consent/identity guards. Replies never finish linked work. Use clock-controlled boundary tests rather than sleeps. Do not hide existing lint failures or modify unrelated code to make them disappear. Avoid schema changes unless needed; if needed, retain data with transactional migration tests.

Acceptance: demonstrate before/after results for the three reports; cover past afternoon, today just before/equal/after, future checks, closed_at/history immutability, CLI/MCP closure parity, unresolved promise/proposal confirmation rejection, successful explicit resolution, prerequisite ordering, changed state after display, replay/restart without duplicates or omissions. Record exact commands, outputs, isolated ledger evidence, baseline versus introduced failures, and unverified Outlook/Teams clients.

Resolved contract: cutoff = min(actual UTC instant, last nanosecond of the requested JST day). Thus past days include 23:59:59.999999999, today/future use actual time, and the next midnight belongs to the next day. Shared close status reports closed/already_closed/closed_at plus pending_count and questions; an already closed day may still need answers. CLI exit 0 means no pending answers, exit 2 means answers remain, including after historical closure; it must describe that day as already closed. MCP closed=true is the historical fact, not absence of pending questions. Repeat closure does not re-export the historical Markdown file.

Plan snapshots: return an opaque plan_revision with plan_day and the final confirmation question. Require it for MCP confirmation and capture it before the CLI confirmation prompt. Compare it with the current full ledger snapshot inside BEGIN IMMEDIATE; reject changed state or newly due/pending commitments before writing plan_confirmed_at. Do not include generated question operation IDs or the advancing clock in the revision. Include tasks, source proposals, commitment revisions, candidates, events and relevant day state; use a consistent read transaction for displayed content and revision. No schema change is planned.

Publication authorization update: the user subsequently requested "きりのいいところでpushして". After proportionate verification and scope review, commit the integrated commitment implementation, its existing trust-core guards and these regression fixes, then push the current codex/requirements-gap branch to origin. Keep docs/issues.md untracked and unchanged; do not include ignored synthetic ledgers, backups or logs. No real-data use, other external sends or live-client validation is authorized.

Current verification completed: baseline 131 tests passed; final 146 passed, 0 failed. Strict Clippy exits 101 with exactly the same 7 baseline diagnostics; introduced ordering regression and two lints were repaired. Final diff check exits 0. Isolated before/after process and SQLite evidence is retained under target/verification/commitment-fixes-20261003-71e22276; see REVIEW-FIXES-20261003.md for commands, boundaries and unverified scope. Plan revision also includes displayed task order; an explicit stale-order regression passes.
