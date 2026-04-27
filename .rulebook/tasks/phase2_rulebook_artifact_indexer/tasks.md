## 1. Envelope kinds + payloads
- [ ] 1.1 Add `Kind::Decision`, `Kind::Learning`, `Kind::Pattern`, `Kind::Law` variants to `cortex-core/src/events.rs` with serde rename rules
- [ ] 1.2 Add canonical payload structs (`DecisionPayload`, `LearningPayload`, `PatternPayload`, `LawPayload`) carrying id / title / status / ts / body / links / metadata
- [ ] 1.3 Round-trip tests for each new kind through the canonical envelope validator

## 2. Indexer crate
- [ ] 2.1 Scaffold `crates/cortex-rulebook-indexer/` with binary + lib + Cargo.toml workspace entry
- [ ] 2.2 Walker that visits `.rulebook/{decisions,learnings,knowledge/patterns,knowledge/anti-patterns,specs}/`
- [ ] 2.3 Parser that reads each `.md` plus sibling `.metadata.json` and emits the right canonical envelope
- [ ] 2.4 Spec-doc parser that splits `## Requirement` / `LAW-NNN` blocks into separate `Kind::Law` envelopes (one per requirement)
- [ ] 2.5 Publisher integration via `cortex-adapter-claude-code::Publisher` trait (or the canonical core publisher) so envelopes hit `~/.cortex/archive/`

## 3. Bootstrap + watch wiring
- [ ] 3.1 `cortex-bootstrap` invokes the indexer at startup, after repo discovery
- [ ] 3.2 Hourly re-scan tokio task (configurable) so edits to `.rulebook/**` land without a stack restart
- [ ] 3.3 Idempotency: re-emitting the same artifact must not double-count — dedupe by `(kind, payload.id)` in the lane seeder

## 4. Query API consumption
- [ ] 4.1 `cortex-api/src/strategies.rs` decisions strategy reads `Kind::Decision` envelopes and populates `results.decisions`
- [ ] 4.2 Laws overlay populates `laws_active` from `Kind::Law` envelopes scoped to the active repo
- [ ] 4.3 Patterns + learnings populate `results.snippets` with a `kind_label` distinguishable from raw turn snippets
- [ ] 4.4 Integration test against a live daemon: seed `.rulebook/` with a known fixture, run `/v1/query`, assert decisions ≥ 1 / laws_active ≥ 1

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update or create documentation covering the implementation (extend spec-09 with a `## Rulebook artifact indexing` section, or a new spec doc)
- [ ] 5.2 Write tests covering the new behavior (per crate as listed above)
- [ ] 5.3 Run tests and confirm they pass
