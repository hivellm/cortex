## 1. Schema authoring
- [ ] 1.1 Write envelope JSON Schema (event_id, ts, kind, adapter, model, session_id, turn_id, source, context_repo, content_hash, schema_version)
- [ ] 1.2 Write per-kind payload schemas (turn.*, tool_call.*, artifact.*, decision.*, memory.*, law.*, analysis.*, event.notification)
- [ ] 1.3 Publish schemas under `cortex-core/schemas/` with `schema_version` constant

## 2. Rust crate scaffold
- [ ] 2.1 Create `cortex-core` crate (workspace root `Cargo.toml`)
- [ ] 2.2 Wire `build.rs` that generates `src/events.rs` from `schemas/*.json` (schemars or typify)
- [ ] 2.3 Re-export generated types through `lib.rs`

## 3. Helpers
- [ ] 3.1 Canonical-JSON serializer (sorted keys, UTF-8 NFC, ms timestamps)
- [ ] 3.2 ULID generator + ID convention utilities
- [ ] 3.3 Content-hash helper: `sha256(canonical_json(redacted_payload))`

## 4. Validator
- [ ] 4.1 Envelope + per-kind validator API
- [ ] 4.2 `cortex-core validate <file>` CLI binary
- [ ] 4.3 Fixture set: one valid + one invalid sample per kind under `tests/fixtures/`

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/01-event-schema.md` status flag to 🟢 and `docs/specs/00-index.md` row
- [ ] 5.2 Unit tests: round-trip fixtures through validator + canonical-JSON stability test
- [ ] 5.3 Run `cargo check && cargo clippy -- -D warnings && cargo test`; confirm ≥95% coverage on the crate
