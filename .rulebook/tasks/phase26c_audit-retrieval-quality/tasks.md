## §1. Bug #8 — Vector retrieval quality: classifier summaries + re-embed

- [ ] §1.1 Audit the Static classifier: confirm whether it populates `summary` on output; read `crates/cortex-workers/src/classifier/static_backend.rs`
- [ ] §1.2 If Static mode does not produce summaries, implement a deterministic template: `"{kind} in {path}: {first 120 chars of payload}"` — no LLM required
- [ ] §1.3 Align `.env` and docker-compose so the classifier mode is consistent and the Static backend with summaries is the running config
- [ ] §1.4 Run a targeted re-classification pass on events where `summary IS NULL` in the enriched stream (or flag them for re-processing via backfill CLI)
- [ ] §1.5 After re-classifying, trigger re-embed for the affected events
- [ ] §1.6 Validate: run query `"event classification system"` against cortex repo; confirm at least one vector result scores above 0.30

## §2. Bug #9 — Pre-thinking bundle cache

- [ ] §2.1 Read `crates/cortex-pre-thinking/src/` — locate the bundle assembler entry point and measure where latency accumulates
- [ ] §2.2 Add an in-process LRU cache: key = `sha256(query_text + scope_repo + intent)`, TTL = 60s, max entries = 256
- [ ] §2.3 Expose `cache_hit_total` and `cache_miss_total` in the pre-thinking health endpoint response
- [ ] §2.4 Verify: two identical pre-thinking queries within 60s — second must return in < 10ms and report `"cache": "hit"` in the response
- [ ] §2.5 Verify: p95 latency in the dashboard overview series drops below 200ms under normal session load

## §3. Bug #10 — Decision status re-parsed on bootstrap

- [ ] §3.1 Read the bootstrap decision promoter — identify where `status: "proposed"` is hardcoded
- [ ] §3.2 Parse the `**Status**: <value>` line from each ADR markdown file during promotion; map to the `status` field
- [ ] §3.3 On incremental bootstrap, compare parsed status against the stored document; update Meilisearch if changed
- [ ] §3.4 Run bootstrap against the Cortex repo; confirm ADR-001 shows `status: "superseded"` in Meilisearch

## §4. Tail (mandatory)

- [ ] §4.1 Update `docs/analysis/cortex/12-live-audit-2026-06-09.md` — mark bugs #8, #9, #10 as fixed with commit reference
- [ ] §4.2 Write tests: Static classifier produces non-empty summary; bundle cache hit/miss behavior; ADR status parser with proposed/accepted/superseded inputs
- [ ] §4.3 Run `cargo check && cargo test -p cortex-workers -p cortex-pre-thinking -p cortex-cli` and confirm all pass
