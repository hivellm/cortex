## 1. Schema authoring
- [x] 1.1 Write envelope JSON Schema (event_id, ts, kind, adapter, model, session_id, turn_id, source, context_repo, content_hash, schema_version)
- [x] 1.2 Write per-kind payload schemas (turn.*, tool_call.*, artifact.*, decision.*, memory.*, law.*, analysis.*, event.notification)
- [x] 1.3 Publish schemas under `cortex-core/schemas/` with `schema_version` constant

## 2. Rust crate scaffold
- [x] 2.1 Create `cortex-core` crate (workspace root `Cargo.toml`)
- [x] 2.2 Wire single-source-of-truth contract: schemas are authoritative, Rust types under `src/events.rs` are locked to them via the `fixtures_roundtrip.rs` integration tests (each fixture validates against its schema AND round-trips through the struct byte-for-byte after canonicalization)
- [x] 2.3 Re-export types through `lib.rs`

## 3. Helpers
- [x] 3.1 Canonical-JSON serializer (sorted keys, UTF-8, shortest-roundtrip numbers)
- [x] 3.2 ULID generator + typed `EventId` / `SessionId` wrappers
- [x] 3.3 Content-hash helper: `sha256(canonical_json(payload))` with `ContentHash` type and known-vector test

## 4. Validator
- [x] 4.1 Envelope + per-kind validator API backed by `jsonschema` 0.30 draft-2020-12
- [x] 4.2 `cortex-core validate <file>` CLI binary (`cortex-core new-id` and `cortex-core hash` included)
- [x] 4.3 Fixture set: one valid fixture per kind + edge cases (redacted / CAS-offloaded / blocked) + invalid fixtures under `tests/fixtures/`

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation (spec 01 flipped to 🟢 in [docs/specs/00-index.md](../../../docs/specs/00-index.md) and [01-event-schema.md](../../../docs/specs/01-event-schema.md))
- [x] 5.2 Write tests covering the new behavior (24 unit tests + 5 integration tests under [crates/cortex-core/tests/](../../../crates/cortex-core/tests/))
- [x] 5.3 Run tests and confirm they pass (`cargo check && cargo clippy --all-targets -- -D warnings && cargo test` all green)
