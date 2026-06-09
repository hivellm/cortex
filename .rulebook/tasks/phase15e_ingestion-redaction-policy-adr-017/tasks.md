## 1. ADR-017
- [x] 1.1 ADR-017 created via rulebook_decision_create — "Ingestion-time redaction policy", id=24, status=proposed.
- [x] 1.2 Trade-off documented in ADR consequences: <1ms synchronous cost in adapter hook path (within 300ms PreToolUse budget); secrets never reach embedder/fulltext/graph/archive; conservative patterns allow false-negatives, mitigated by doctor coverage check.

## 2. Redaction module
- [ ] 2.1 New `crates/cortex-core/src/redaction.rs` exposing `pub fn redact(env: Envelope) -> Envelope`.
- [ ] 2.2 Patterns: AWS access keys (`AKIA[0-9A-Z]{16}`), GitHub PATs (`ghp_[A-Za-z0-9]{36}`), Anthropic keys (`sk-ant-[A-Za-z0-9_-]+`), `Bearer <token>` / `Authorization: ...`, `password=...`, generic ≥40-char base64-ish strings (`[A-Za-z0-9+/=]{40,}` with high entropy).
- [ ] 2.3 Replace each match with `<REDACTED:<kind>:<hash8>>` where `hash8` is the first 8 chars of SHA256(matched_text). Duplicate detection works on the placeholder.
- [ ] 2.4 8 unit tests: each pattern matches and redacts a fixture; non-secret strings of similar shape are not redacted.

## 3. Adapter integration
- [ ] 3.1 `cortex-adapter-claude-code/src/dispatcher.rs::dispatch` calls `redaction::redact(env)` before sending to the ingestion endpoint.
- [ ] 3.2 Round-trip IT: synthetic envelope with embedded AWS key → redacted before reaching ingestion.

## 4. Doctor coverage
- [ ] 4.1 `cortex-ops doctor redaction-coverage` samples 100 random envelopes from Synap and runs the pattern detectors against them.
- [ ] 4.2 Reports any matches as `unredacted-candidate` with line + offset (truncated to first 16 chars + hash).
- [ ] 4.3 Exit 0 when zero candidates; exit 2 when any.

## 5. Tail (mandatory)
- [ ] 5.1 Update `docs/specs/04-event-schema.md` + `CHANGELOG.md` Added.
- [ ] 5.2 Tests: §2.4 + §3.2 IT.
- [ ] 5.3 `cargo check --workspace && cargo clippy -p cortex-core -- -D warnings && cargo test -p cortex-core redaction` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
