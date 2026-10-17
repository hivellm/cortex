# Daily agent memory recovery

Date: 2026-10-17. Status: implementation, independent reviews, full quality gates, local activation and authorized live validation passed.

## Objective

Make prior decisions and verified lessons change what Claude, Codex, Grok,
and other connected agents do in a later session, without requiring the
operator to repeat context. Integrate Jev for bounded semantic classifications
and assess which ai-memory mechanisms are worth adopting.

## Method

Evidence comes from source inspection, runtime records, current upstream
documentation, and independent model reviews. Historical measurements are
identified separately from fresh checks. No claim of benefit is accepted from
service uptime, document counts, or a structurally valid context bundle alone.

## Coordination

The operator authorized analysis, joint decisions with Claude/Grok/Kimi,
Rulebook task creation, implementation, and review. Existing unrelated
worktree changes are preserved. Implementation follows task checklist order.
Actual model participation and remaining native-client limits are recorded.

The operator explicitly authorized this one recovery task before the old
backlog, then clarified that only work important to current utility should
be implemented. Task: `phase34_daily-agent-memory-recovery`.

## Reading order

- [01 - Current memory loop](01-current-memory-loop.md): F-001 to F-010.
- [02 - Jev classification](02-jev-classification.md): F-011 to F-020.
- [03 - ai-memory comparison](03-ai-memory-comparison.md): F-021 to F-029.
- [04 - Existing Claude session review](04-claude-coordination.md): runtime evidence and coordination.
- [05 - Joint decision](05-joint-decision.md): actual participation, selected architecture and exclusions.
- [05 - Claude proposal](05-execution-plan.md): retained review input; alternate task and OS autostart are not selected.
- [06 - Execution plan](06-execution-plan.md): dependencies, owners and acceptance.
- [07 - Review of Claude proposal](07-astra-review-of-superseded-claude-plan.md): additional review input and limitations.
- [08 - Implementation evidence](08-implementation-evidence.md): actual changes, tests and activation limits.
- [09 - Client contracts](09-client-contracts.md): actual harness capabilities and transport evidence.
- [10 - Resources](10-resource-acceptance.md): complete HTTP latency and idle measurements.
- [11 - Cross-model acceptance](11-cross-model-acceptance.md): observed corrected action after restart.
- [12 - Activation inventory](12-activation-inventory.md): preserved data, stopped services and rollback.
- [13 - Client activation](13-client-activation.md): installed configuration and honest verification limits.
- [14 - Native preservation](14-native-preservation.md): before/after file hashes and scheduler state.
- [15 - Installed workflow](15-installed-workflow.md): actual Claude hooks and public-canary use, pipe correction and disclosure limits.
- [16 - Astra final review](16-astra-final-review.md): archive authorization and projection corrections.
- [17 - Sol final review](17-sol-final-review.md): archive delivery, retries and offline gates.
- [18 - Integration final review](18-integration-final-review.md): explicit repository identity and Windows test coverage.
- [19 - Final validation](19-final-validation.md): quality gates and authorization history.
- [20 - Kimi/Grok review](20-external-review.md): independent findings, triage and feedback correction.
- [21 - Claude final review](21-claude-final-review.md): completed native source review, preserved earlier attempts and local triage.
- [22 - Authorized live validation](22-authorized-live-validation.md): delayed consent and installed-runtime checks.
- [23 - Release runtime validation](23-release-runtime-validation.md): development-build delivery failure, exact release pins, repeated hooks and final resource observation.

## Executive conclusion

Fix the delivery of explicit, sourced decisions and lessons first. Keep Cortex
as the authority, use existing SQLite for reliable compact memory, and provide
a lightweight runtime. Adapt ai-memory's provenance, versioning and bounded
context principles without adding a second permanent service. Jev classifies
low-volume memory candidates; it does not turn model guesses into rules or
replace the basic local memory loop.

The final candidate passed 3,955 tests with zero failures and nine explicitly
ignored tests, plus formatting, type-checking and Clippy. Installed Claude/Fable
recalled two authorized saved decisions through real hooks without tool access.
Jev classified three authorized original candidates and reused cached results
without changing memory content or lifecycle. A follow-up exposed intermittent
delivery failures in the unoptimized development build. The same source compiled
for release passed 40/40 consecutive hooks without changing authentication or
deadlines. The installed release API/adapter ended a 60-second light-activity
observation at 32.031 MiB combined working set; the earlier 97.367 MiB result
belongs to the development build.
All 535 native memory files and six Cortex volumes remain preserved; 12 heavy
containers remain stopped. Analyses22/23 record evidence and practical limits.
