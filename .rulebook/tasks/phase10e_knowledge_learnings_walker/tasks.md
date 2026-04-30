## 1. Envelope schema
- [ ] 1.1 Register `kind="knowledge"` and `kind="learning"` in `crates/cortex-core/src/envelope.rs` (or wherever the kind enum lives)
- [ ] 1.2 `payload.category ∈ {pattern, anti_pattern}` for `knowledge`; `payload.source ∈ {rulebook.knowledge, rulebook.learning}`
- [ ] 1.3 Update the spec-04 JSON schema fixtures + redaction allow-list

## 2. Storage declarations
- [ ] 2.1 Add `cortex.knowledge.fp32` + `cortex.learning.fp32` to `crates/cortex-storage/src/collections.rs`
- [ ] 2.2 Add `cortex_knowledge` + `cortex_learnings` to `crates/cortex-storage/src/fulltext.rs`
- [ ] 2.3 Add `:Knowledge` + `:Learning` Nexus labels and edges (`(:Session)-[:LEARNED]->(:Learning)`, `(:Knowledge)-[:RELATES_TO]->(:Decision)`)

## 3. Bootstrap walker
- [ ] 3.1 In `crates/cortex-cli/src/bootstrap/walker.rs`, add the two extra root-relative globs: `.rulebook/knowledge/*.md` and `.rulebook/learnings/*.md`
- [ ] 3.2 One envelope per file, `kind=knowledge|learning`, body inline + CAS, `repo` from the bootstrap context
- [ ] 3.3 Walker honours phase10c dedup (no double-emit on re-run)

## 4. Worker indexing
- [ ] 4.1 Embedder picks up `kind=knowledge|learning` and writes to the new collections
- [ ] 4.2 Fulltext worker projects to the new indexes
- [ ] 4.3 Graph worker writes the labels + edges

## 5. Surfaces
- [ ] 5.1 `/v1/dashboard/memory?facets=knowledge,learning` filters returns to the new kinds
- [ ] 5.2 Pre-thinking pipeline pulls knowledge + learnings for `pre_change_context` and `decision_lookup` intents
- [ ] 5.3 The query orchestrator's `decisions` lane fuses `:Knowledge` neighbors via the existing graph lane

## 6. Tests
- [ ] 6.1 Walker smoke: bootstrap a fixture repo with one knowledge + one learning file, assert two extra envelopes emitted
- [ ] 6.2 Embedder/fulltext/graph round-trip tests for the new kinds
- [ ] 6.3 Pre-thinking bundle includes a known-knowledge phrase when prompted to make a related change

## 7. Spec / docs
- [ ] 7.1 Update `docs/specs/02-storage-layout.md` §collections / indexes / labels
- [ ] 7.2 Update `docs/specs/06-embedder.md` §single-tier kinds
- [ ] 7.3 Update `docs/specs/12-pre-thinking-injection.md` §sources

## 8. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 8.1 Update or create documentation covering the implementation
- [ ] 8.2 Write tests covering the new behavior
- [ ] 8.3 Run tests and confirm they pass
