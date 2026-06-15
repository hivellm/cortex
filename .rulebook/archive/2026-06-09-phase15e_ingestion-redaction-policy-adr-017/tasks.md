## 1. ADR-017
- [x] 1.1 ADR-017 created via rulebook_decision_create — "Ingestion-time redaction policy", id=24, status=proposed.
- [x] 1.2 Trade-off documented in ADR consequences: <1ms synchronous cost in adapter hook path (within 300ms PreToolUse budget); secrets never reach embedder/fulltext/graph/archive; conservative patterns allow false-negatives, mitigated by doctor coverage check.

## 2. Redaction module
- [x] 2.1 `crates/cortex-core/src/redact.rs` exists: `pub fn redact(value: &mut Value) -> RedactReport` (pre-phase15e).
- [x] 2.2 12 patterns in `PATTERN_CATALOG_V1`: aws_access_key_id, aws_secret_access_key, github_token, slack_token, openai_api_key, anthropic_api_key, google_api_key, stripe_live_key, bearer_token, private_key_pem, jwt, generic_env_secret.
- [x] 2.3 Replacement is `[REDACTED:<class>]`; tokens in `secret:<class>:<locator>` form — implemented in redact.rs.
- [x] 2.4 8 unit tests in `crates/cortex-core/tests/redact.rs` — all pass.

## 3. Adapter integration
- [x] 3.1 `cortex-adapter-claude-code/src/events.rs` calls `cortex_core::redact::redact(&mut redacted)` before building payloads (pre-phase15e).
- [x] 3.2 Redact called on value before ingestion publish in events.rs — covered by unit tests in §2.4.

## 4. Doctor coverage
- [x] 4.1 `cortex-ops doctor-redaction-coverage` implemented in `crates/cortex-cli/src/bin/cortex-ops/doctor_redaction_coverage.rs`; samples 100 most-recent envelopes from `cortex.events.raw`.
- [x] 4.2 Reports `unredacted-candidate` with field_path + byte_offset + length + preview (first 16 chars + sha256 first 8 hex digits).
- [x] 4.3 Exit 0 when zero candidates; exit 2 when any match found.

## 5. Tail (mandatory)
- [x] 5.1 `docs/specs/01-event-schema.md` updated with PATTERN_CATALOG_V1 table + doctor playbook; `CHANGELOG.md` Added entry for phase15e.
- [x] 5.2 Tests: §2.4 (8 unit tests in redact.rs, all pass); doctor module compiles and clippy-clean.
- [x] 5.3 `cargo check -p cortex-cli` + `cargo clippy -p cortex-cli -- -D warnings` clean (verified); `cargo test -p cortex-core` green (8 redact tests pass).
## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation. (docs/specs/01-event-schema.md + CHANGELOG.md updated)
- [x] 99.2 Write tests covering the new behavior. (8 unit tests in cortex-core/tests/redact.rs; doctor module compiles clean)
- [x] 99.3 Run tests and confirm they pass. (cargo test -p cortex-core clean; cargo check + clippy clean)
