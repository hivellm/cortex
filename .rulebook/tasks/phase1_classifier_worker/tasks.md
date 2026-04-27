## 1. Worker scaffolding
- [x] 1.1 Add `cortex-classifier-worker` crate (separate from classifier lib to avoid embedder cycle)
- [x] 1.2 Add `config.rs` parsing `CORTEX_CLASSIFIER_*` env vars
- [x] 1.3 Add `worker.rs` with Synap consumer/publisher abstractions, kind-mapping, and the run loop
- [x] 1.4 Add `main.rs` with ctrl-c shutdown and stack composition (static default, cli optional)

## 2. Behavior
- [x] 2.1 Map bootstrap event kinds onto `cortex_core::events::Kind`
- [x] 2.2 Build `EnrichmentInput` from both bootstrap and canonical envelope shapes
- [x] 2.3 Publish `EnrichedEvent` matching `cortex_embedder::EnrichedEvent` shape on `cortex.events.enriched`
- [x] 2.4 In-memory replay dedup keyed on `event_id`

## 3. Repo bootstrap config
- [x] 3.1 Create `cortex.toml` at the Cortex repo root with exclude / decisions / memories rules
- [x] 3.2 Create `.env` from `.env.example` with classifier defaults pointing at static mode

## 4. Tests
- [x] 4.1 Unit test: kind-mapping for every bootstrap kind
- [x] 4.2 Unit test: bootstrap envelope to EnrichedEvent fixture
- [x] 4.3 Unit test: canonical envelope to EnrichedEvent fixture
- [x] 4.4 Unit test: replay dedup drops the duplicate
- [x] 4.5 Unit test: budget halt swaps to static fallback

## 5. End-to-end run
- [x] 5.1 Build the new worker (release profile)
- [x] 5.2 Bring up classifier + embedder + graph + fulltext workers
- [x] 5.3 Run cortex-bootstrap on the Cortex repo (519 events published in 0.6s)
- [x] 5.4 Verify content landed in Meilisearch (519 docs across 4 indexes); flag pre-existing Vectorizer upsert + Nexus persistence bugs as separate follow-ups

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation
- [x] 6.2 Write tests covering the new behavior
- [x] 6.3 Run tests and confirm they pass
