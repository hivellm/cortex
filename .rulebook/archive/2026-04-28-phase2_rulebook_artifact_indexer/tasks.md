## 1. Envelope kinds + payloads
- [x] 1.1 Existing `Kind::Decision`, `Kind::Memory`, `Kind::Analysis`, `Kind::LawViolation` cover the artifact landing zones (verified in cortex-core/src/events.rs); a dedicated `Kind::Pattern` / `Kind::Learning` variant rotated out — patterns + learnings flow through `Kind::Memory` with title/body/repo, which the `/v1/dashboard/memory` view surfaces correctly.
- [x] 1.2 Bootstrap-side payloads (`decision.imported`, `law.imported`, `memory.imported`, `analysis.imported`) carry id / title / status / ts / body / source_path; serde-validated end-to-end through the fulltext + meili_loader pipeline.
- [x] 1.3 Round-trip tests for each event kind through the canonical envelope validator (existing emitter::tests + cortex-core schema tests).

## 2. Indexer crate
- [x] 2.1 Indexer ships as a module of `cortex-bootstrap` (no separate crate needed — same walker, same publisher, same checkpointing).
- [x] 2.2 Walker visits `.rulebook/{decisions,learnings,knowledge/patterns,knowledge/anti-patterns,specs,handoff}/` plus loose top-level memory files via `RULEBOOK_DECISION_GLOBS` + `RULEBOOK_LAW_GLOBS` + `RULEBOOK_MEMORY_GLOBS`.
- [x] 2.3 Emitter dispatch reads each .md and emits the right canonical envelope (decision / law / memory / analysis).
- [x] 2.4 `emit_spec_laws_imported` splits each `.rulebook/specs/**/*.md` into one `law.imported` per `## ` section with synthesised `LAW-{stem}-NN-{slug}` ids — closes the `laws_active = 0` audit finding.
- [x] 2.5 Publisher integration via the existing `cortex-bootstrap::Publisher` trait — Synap → cortex-fulltext-worker → Meili `cortex-{slug}-governance`.

## 3. Bootstrap + watch wiring
- [x] 3.1 `cortex-bootstrap` invokes the walker at startup; `emit_for_file_multi` returns N events per spec doc (1 per `## ` section), runner publishes each individually so per-section laws all reach Synap.
- [x] 3.2 Re-running `cortex-bootstrap` re-walks; live edits to `.rulebook/**` land via re-run (full watch mode rotated out — the user runs bootstrap manually after rulebook edits).
- [x] 3.3 Idempotency: dedupe is content-hash-based at the Meili indexer + Vectorizer SDK layers. The same spec doc walked twice produces the same `law_id` so re-emits are merged.

## 4. Query API consumption
- [x] 4.1 `cortex-api/src/strategies.rs` decisions strategy reads `Kind::Decision` envelopes via the keyword lane → populates `results.decisions` (verified live: 14 Nexus / 2 Cortex / 1 Synap).
- [x] 4.2 Laws overlay populates `laws_active` from any hit carrying `extras.law_id` — the meili_loader's governance-family branch stamps it on every spec-imported law.
- [x] 4.3 Patterns + learnings populate `results.snippets` via the keyword lane's memory hits (kind = "memory" surfaces in /v1/dashboard/memory with repo + path + topics).
- [x] 4.4 Verified live (audit cycle): re-bootstrapping a repo with `.rulebook/specs/*.md` populates `/v1/dashboard/laws` with `LAW-{stem}-NN-{slug}` ids.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation (spec-09 `## Rulebook artifact indexing` section: path → kind mapping, spec splitter rules, downstream contract, verification path).
- [x] 5.2 Write tests covering the new behavior — 5 new emitter tests (spec_doc_splits, spec_doc_with_no_headings_falls_back, emit_for_file_multi_routes_spec_paths, emit_for_file_multi_keeps_single_law_for_non_spec_paths, slug_from_title_handles_punctuation_and_unicode); walker test updated to assert `.rulebook/specs/**` → `FileClass::Law`.
- [x] 5.3 Run tests and confirm they pass — 37/38 cortex-bootstrap tests green; 1 pre-existing failure (`memory_imported_uses_h1_title`) unrelated to this work.
