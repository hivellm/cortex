## 1. Worker scaffolding
- [ ] 1.1 Add `cortex-classifier-worker` binary target + runtime deps to `crates/cortex-classifier/Cargo.toml`
- [ ] 1.2 Add `config.rs` parsing `CORTEX_CLASSIFIER_*` env vars
- [ ] 1.3 Add `worker.rs` with Synap consumer/publisher abstractions, kind-mapping, and the run loop
- [ ] 1.4 Add `main.rs` with ctrl-c shutdown and stack composition (static default, cli optional)

## 2. Behavior
- [ ] 2.1 Map bootstrap event kinds onto `cortex_core::events::Kind`
- [ ] 2.2 Build `EnrichmentInput` from both bootstrap and canonical envelope shapes
- [ ] 2.3 Publish `EnrichedEvent` matching `cortex_embedder::EnrichedEvent` shape on `cortex.events.enriched`
- [ ] 2.4 In-memory replay dedup keyed on `event_id`

## 3. Repo bootstrap config
- [ ] 3.1 Create `cortex.toml` at the Cortex repo root with exclude / decisions / memories rules
- [ ] 3.2 Create `.env` from `.env.example` so workers pick up the right service URLs

## 4. Tests
- [ ] 4.1 Unit test: kind-mapping for every bootstrap kind
- [ ] 4.2 Unit test: bootstrap envelope to EnrichedEvent fixture
- [ ] 4.3 Unit test: canonical envelope to EnrichedEvent fixture
- [ ] 4.4 Unit test: replay dedup drops the duplicate
- [ ] 4.5 Unit test: budget halt swaps to static fallback

## 5. End-to-end run
- [ ] 5.1 Build the new worker
- [ ] 5.2 Bring up classifier + embedder + graph + fulltext workers
- [ ] 5.3 Run cortex-bootstrap on the Cortex repo
- [ ] 5.4 Verify content landed in Vectorizer / Nexus / Meilisearch

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation
- [ ] 6.2 Write tests covering the new behavior
- [ ] 6.3 Run tests and confirm they pass
