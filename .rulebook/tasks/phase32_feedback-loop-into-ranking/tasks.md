## 1. Prerequisites and attribution
- [ ] 1.1 Verify phase36_local-first-core, phase28_retrieval-eval-gate-live and phase31_pre-thinking-audit-trail are complete; record executable interfaces, locked evaluation baseline and current legacy feedback schema before implementation.
- [ ] 1.2 Define canonical feedback identity, exact repository/query/record-revision attribution, explicit versus inferred evidence, duplicate handling and import eligibility; preserve unattributed legacy feedback without allowing it to tune ranking.
- [ ] 1.3 Specify deterministic decay, minimum evidence threshold, adjustment bounds, explanation fields and rollback; a bundle verdict or missing citation cannot imply per-record or per-lane blame.

## 2. Local implementation and verification
- [ ] 2.1 Persist and inspect feedback through local CLI/MCP/embedded GUI; test scope/identity validation, duplicate retries, immutable provenance, index rebuild and legacy unknown attribution.
- [ ] 2.2 Add disabled-by-default bounded ranking adjustment and explanation; test baseline equality when disabled, decay/bounds, insufficient evidence, duplicate replay and repository isolation.
- [ ] 2.3 Run enabled-versus-disabled evaluation against the locked local corpus; report per-query quality and latency changes, regressions and unknowns; retain disabled default if evidence is inconclusive or harmful and verify rollback preserves feedback/source records.

## 3. Tail (docs + tests)
- [ ] 3.1 Update or create documentation covering the implementation: attribution rules, opt-in behavior, bounds, evaluation evidence and rollback in operator docs and CHANGELOG.
- [ ] 3.2 Write tests covering the new behavior; review persistence/reindex, attribution, isolation and ranking regressions and add missing behavior tests.
- [ ] 3.3 Run tests and confirm they pass, plus type-check, format/lint and full required suite; record independent review and actual results before completion/archive.
