## 1. Envelope schema
- [x] 1.1 Register `kind="knowledge"` and `kind="learning"` in `crates/cortex-core/src/envelope.rs` (or wherever the kind enum lives)
- [x] 1.2 `payload.category ∈ {pattern, anti_pattern}` for `knowledge`; `payload.source ∈ {rulebook.knowledge, rulebook.learning}`
- [x] 1.3 Update the spec-04 JSON schema fixtures + redaction allow-list

## 2. Storage declarations
- [x] 2.1 Add `cortex.knowledge.fp32` + `cortex.learning.fp32` to `crates/cortex-storage/src/collections.rs`
- [x] 2.2 Add `cortex_knowledge` + `cortex_learnings` to `crates/cortex-storage/src/fulltext.rs`
- [x] 2.3 Add `:Knowledge` + `:Learning` Nexus labels and edges (`(:Session)-[:LEARNED]->(:Learning)`, `(:Knowledge)-[:RELATES_TO]->(:Decision)`)

## 3. Bootstrap walker
- [x] 3.1 In `crates/cortex-cli/src/bootstrap/walker.rs`, add the two extra root-relative globs: `.rulebook/knowledge/*.md` and `.rulebook/learnings/*.md`
- [x] 3.2 One envelope per file, `kind=knowledge|learning`, body inline + CAS, `repo` from the bootstrap context
- [x] 3.3 Walker honours phase10c dedup (no double-emit on re-run)

## 4. Worker indexing
- [x] 4.1 Embedder picks up `kind=knowledge|learning` and writes to the new collections
- [x] 4.2 Fulltext worker projects to the new indexes
- [x] 4.3 Graph worker writes the labels + edges

## 5. Surfaces
- [x] 5.1 `/v1/dashboard/memory?facets=knowledge,learning` filters returns to the new kinds
- [x] 5.2 Pre-thinking pipeline pulls knowledge + learnings for `pre_change_context` and `decision_lookup` intents
- [x] 5.3 The query orchestrator's `decisions` lane fuses `:Knowledge` neighbors via the existing graph lane

## 6. Tests
- [x] 6.1 Walker smoke: bootstrap a fixture repo with one knowledge + one learning file, assert two extra envelopes emitted
- [x] 6.2 Embedder/fulltext/graph round-trip tests for the new kinds
- [x] 6.3 Pre-thinking bundle includes a known-knowledge phrase when prompted to make a related change

## 7. Spec / docs
- [x] 7.1 Update `docs/specs/02-storage-layout.md` §collections / indexes / labels
- [x] 7.2 Update `docs/specs/06-embedder.md` §single-tier kinds
- [x] 7.3 Update `docs/specs/12-pre-thinking-injection.md` §sources

## 8. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 8.1 Update or create documentation covering the implementation
- [x] 8.2 Write tests covering the new behavior
- [x] 8.3 Run tests and confirm they pass
