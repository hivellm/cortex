## §1. Bug #8 live verification (classifier summaries → re-embed)

- [x] §1.1 Re-classification pass: summaries CONFIRMED populated via Meili sampling — turns/code/docs 100% (1000/1000 each), knowledge 29/30; analyses lags at 22% (190/868) → tracked in phase26e §1.1.
- [x] §1.2 Re-embed: CONFIRMED — corpus carries NL summaries (phase0 nl-embedding re-embed). Caveat: re-embed was additive, stale raw-JSON vectors not purged → phase26e §1.
- [x] §1.3 Vector score validation: top vector hit recovered 0.130→0.238 (audit method, /v1/query repo=cortex "event classification system") — real recovery but below the 0.50 bar; root cause = stale additive vectors dilute. Remediation tracked in phase26e §1 (purge + clean re-embed).

## §2. Bug #9 live verification (bundle cache)

- [x] §2.1 Cache hit: NOT OBSERVABLE in split deployment — BundleCache + counters live in cortex-adapter process; /v1/health/pre-thinking (cortex-api) uses UnwiredPreThinkingHealthSource → 0/0; adapter does not export counters. Behavior covered by 5 unit tests; backend warm path 2.5ms. Remediation tracked in phase26e §2.
- [x] §2.2 p95 latency: pre_thinking_p95_ms series = p95 of envelope duration_ms (all envelopes), NOT bundle latency; live 2947–43426ms reflects long tool_calls. Cannot validate cache from this series. Remediation tracked in phase26e §3 (dedicated metric).

## §3. Bug #10 live verification (ADR status)

- [x] §3.1 ADR status: CONFIRMED — Meili cortex-cortex-decisions shows ADR-001 ext.decision.status="superseded"; ADR-002 remains "proposed" (correct). Incremental re-emit propagates the **Status**: line as designed.

## §4. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] §4.1 Update or create documentation covering the implementation. Added "Live Verification (phase26d — 2026-06-24)" section to docs/analysis/cortex/12-live-audit-2026-06-09.md with all six item results.
- [x] §4.2 Write tests covering the new behavior. Operational verification only; no new code. Behavior covered by phase26c tests (5 bundle-cache, static-classifier, bootstrap-runner). Three residual gaps materialized as phase26e_retrieval-quality-remediation.
- [x] §4.3 Run tests and confirm they pass. cargo check (adapter+workers+cli) exit 0; cortex-adapter-claude-code --lib tests pass (bundle-cache regression).
