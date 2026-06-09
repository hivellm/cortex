## §1. Bug #8 live verification (classifier summaries → re-embed)

- [ ] §1.1 Re-classification pass: trigger re-classification on events where `summary IS NULL`; requires live Synap enriched stream + classifier container.
- [ ] §1.2 Re-embed: after §1.1, trigger re-embed of re-classified events; confirm embedder processes them without errors.
- [ ] §1.3 Vector score validation: run audit queries (`"event classification system"`, `"deploy process and release workflow"`); confirm vector rank-1 score > 0.50 (was <0.15 before phase26c).

## §2. Bug #9 live verification (bundle cache)

- [ ] §2.1 Cache hit: issue two identical pre-thinking queries within 60s; confirm second returns `cache_hit: true` in the health endpoint and response time < 10ms.
- [ ] §2.2 p95 latency: confirm `pre_thinking_p95_ms` series in dashboard drops back below 200ms under normal session load.

## §3. Bug #10 live verification (ADR status)

- [ ] §3.1 Run `cortex bootstrap run` against the Cortex repo; confirm ADR-001 shows `status: "superseded"` in Meilisearch after the incremental run (was always "proposed" before phase26c).

## §4. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] §4.1 Update or create documentation covering the implementation. Mark the six verification items as confirmed in `docs/analysis/cortex/12-live-audit-2026-06-09.md`.
- [ ] §4.2 Write tests covering the new behavior. Operational verification only; code tested in phase26c.
- [ ] §4.3 Run tests and confirm they pass. Confirm live stack state per §1–§3 above.
