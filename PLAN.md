# dayloop implementation plan — daily workflow beta

Approved: 2026-09-05. Baseline: `9ca061199a94cac67369a5ade12ee706353c8df9`.
Branch: `codex/daily-workflow-beta`. The user subsequently authorized push: root will commit/push reviewed task artifacts only after passing checks. No public release, repository visibility change, or automatic user startup registration.

## Goal and boundaries

Deliver the executable-first daily workflow for PdM, PjM and engineers: plan → check → close → weekly retrospective, with seven review categories and AI-chat/CLI/Markdown entry points. Implement stages 1–4 before expanding to Edge ingestion, an executable-free VS Code edition, and M365 integration. Preserve the distinction between implemented code, synthetic tests, real clients, real services, and multi-day acceptance.

- The ledger is local and authoritative. Service connections and cloud-AI disclosure are separate opt-ins; cloud body disclosure defaults off.
- Store application data under `%LOCALAPPDATA%\dayloop`, or explicit `DAYLOOP_HOME`. Only an explicitly requested HKCU startup registration is an exception. No Startup-folder script fallback.
- Runtime requires no PowerShell, Node, administrator elevation or downloaded browser. No external write actions (mail send/read state, attendance punch, alert acknowledgement).
- AI suggests candidates and asks questions. Silence cannot complete, approve or discard work. Preserve CLI/Markdown operation without AI.
- Changes to existing client settings must be additive and reviewable; no secret logging, authentication automation, or settings replacement.
- All runtime verification uses isolated data. Do not modify the real user's ledger or enable startup.

## Ordered work and acceptance

### Task 1: Ledger integrity and safe migration

Implement closed-day write guards, backlog-only scheduling, genuine-due-change carry resets, and previous-day guards on plan confirmation in Store transactions. Include direct Store/CLI/MCP/Markdown/candidate/carry/split paths. Add read-only integrity diagnostics and explicit `reopen_day(date, reason)` with audit history. Reopen clears closed_at and plan_confirmed_at; it never rewrites task outcomes. Add versioned schema migration, consistent pre-migration backup, rejection of future schema versions and failed-migration rollback. Preserve existing task IDs and data.

Acceptance: regression tests fail before the fixes and pass afterward; rejected operations leave all related rows unchanged; concurrent clients cannot create closed-day/open-task contradictions. Existing tests pass.

### Task 2: Shared business review model

Add source observations/provenance, per-source fetch reports, dated review records and weekday routines. Categories: teams, outlook, attendance, tasks, alerts, meeting_prep, meeting_results. Fetch state is independent from human review and task completion. Successful empty results differ from failure/partial/unavailable/stale. Daily review outcomes: pending, confirmed, needs_action, not_checked (nonblank reason required). Required pending checks block close; needs_action must have a persisted task/candidate link. Not-checked outcomes do not disable tomorrow's review.

Default new installations review all seven categories; support explicit selection for subsequent days. Legacy days are not retroactively given new required checks. Routine occurrence keys are routine ID + date and are generated once on first daily preparation; missed dates are disclosed, not silently completed. Preserve source and meeting IDs from observation through candidate adoption. Existing Outlook/fixtures enter the same pipeline and retain partial results independently.

Acceptance: each category is represented; no failed fetch claims success; required checks block close across every entry point; routine retries are idempotent; provenance survives adoption; schema migrations preserve legacy ledgers.

### Task 3: AI chat and manual business intake

Extend the existing MCP/CLI operations with review recording, routine management, note ingestion, retrospective notes and reopen. Use the existing structured questions/options contract consistently. Meeting notes and pasted source excerpts become observations and proposed candidates, never automatic task completions. A single meeting may create multiple distinct candidate actions. Preserve rejected/action IDs across repeat imports. Default extraction is deterministic and works without an LLM; an AI host may propose bounded candidates linked to the stored note.

Provide local LM Studio and GitHub Copilot connection profiles with output disclosure controls. Cloud profile is denied until explicitly enabled; bodies/excerpts/evidence/notes remain excluded unless separately allowed. Explain that information pasted directly into a cloud client has already left the PC. Provide daily conversation instructions and real-client acceptance scripts without claiming a custom host is a real-client test.

Acceptance: CLI and MCP can complete a full daily and weekly workflow with all seven categories; malformed IDs, untrusted notes and missing answers do not cause unrequested transitions. Run real-client checks only where the installed environment permits them; label unavailable evidence.

### Task 4: Daily operation and distribution

Separate phase evaluation, pending questions, notification delivery and business completion. Notify in morning/noon/evening slots; carry pending questions to the next slot; group catch-up notifications after sleep; aggregate Friday retrospective with evening reminders. Persist state atomically and prevent duplicate concurrent serve instances. Use native Windows notifications with file fallback, no PowerShell launch or PowerShell app identity. Startup installation uses HKCU only on explicit command and removes only an owned registration.

Provide first-run setup, capabilities/ledger diagnostics, safe backup, local evaluation scenarios, refreshed README and reproducible beta package/checksum generation. Keep install and client configuration user-initiated. Review release metadata and CI definitions.

Acceptance: deterministic clock tests and restart/failure tests; native notification where testable; clean Windows, signed public distribution, exact-commit CI and five working days of actual usage are separate outstanding gates until actually observed. Prepare the beta artifact without publishing or signing with unapproved credentials.

### Task 5: Edge ingestion (after executable baseline)

Rust CDP + existing Edge + dedicated local profile. Read Outlook mail/calendar, then Teams, then a user-selected attendance and alert product. Login stays with the user. Report observed scope/time and partial/failure states. Do not enable routes whose read-state or business-side effects cannot be avoided. Product names and permitted test accounts are required before product-specific implementations/real-service acceptance; common manual review remains available.

### Task 6: Executable-free VS Code edition

Once executable contracts are stable, implement the same behavior inside the VS Code process in TypeScript, with an in-process MCP endpoint and SQLite compatibility. No external executable; enforce a single writer across editions. Run shared contract tests and extension lifecycle tests. Requires the preceding contract baseline to be accepted.

### Task 7: Graph and M365 (external environment gate)

Begin once a tenant/test account and organization-approved environment are supplied. Read Graph mail/calendar, then Teams. Use an authenticated organization-managed HTTPS MCP relay with outbound PC connection; the authoritative ledger stays local. Offline PCs return unavailable and never queue silent future writes. Test identity/tenant isolation, permissions, outage and end-to-end outcomes before declaring support. No currently available tenant; do not fabricate deployment/authentication evidence.

## Verification and integration

- Every task has focused behavior tests plus root review before dependent integration. Task 2 and client/document preparation may run in parallel after Task 1's contracts pass.
- Final checks: cargo test, clippy with warnings denied, release build, CLI/MCP smoke and migration/backup round-trip. Avoid repeated unchanged test runs.
- Check PdM meeting/decision, PjM request/progress, engineer task/alert scenarios across daily and weekly loops.
- Native client/service/multi-day checks are distinct from synthetic contract tests. Unrun checks remain unverified.
- Source, docs, test artifacts and limitations are delivered for review; user authorized scoped commit/push. Publication remains a separate decision.

## Progress

- Initial checkout clean; feature branch created. Baseline: all 26 tests passed.
- User subsequently requested push; scoped root commits/push are authorized, publication remains gated.
- Task 1: implemented and reviewed. 24 new behavior tests plus 26 existing tests pass (50 total); Clippy and release build pass. Root incorporated review findings for empty unclosed days, strict state/ID decoding, structural read-only diagnosis and backup under the migration write lock. Release CLI/MCP smoke passed 15 assertions with isolated synthetic data. Reviewer accepted the final code. Remote CI will be checked for the pushed commit.
- Tasks 2–4: pending; this first development unit does not claim the seven-category workflow or beta release acceptance.
- Tasks 5–6: follow-on after executable baseline.
- Task 7: environment unavailable as explicitly confirmed by user.
