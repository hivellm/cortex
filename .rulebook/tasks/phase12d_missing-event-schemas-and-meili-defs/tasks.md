## 1. Knowledge + learning JSON schemas
- [ ] 1.1 Author `crates/cortex-core/schemas/knowledge.schema.json` mirroring `decision.schema.json` structure with fields specific to the knowledge kind (`pattern_type`, `category`, `evidence`, `confidence`).
- [ ] 1.2 Author `crates/cortex-core/schemas/learning.schema.json` mirroring same structure with fields (`source_task`, `category`, `applies_to`, `evidence`).
- [ ] 1.3 Wire both into `EventValidator::load_schemas()`.
- [ ] 1.4 Validation tests: happy path + missing `event_id` + extra-fields rejected for both kinds.

## 2. Meili index definitions
- [ ] 2.1 Add `cortex_consolidations` index to `bootstrap.rs` with `searchable_attributes: [body, takeaways, repos]`, `filterable_attributes: [grain, occurred_at, repos]`, `sortable_attributes: [occurred_at]`.
- [ ] 2.2 Add `cortex_topic_cards` index with `searchable_attributes: [title, body, topics]`, `filterable_attributes: [topic_id, repos, contradiction_kind]`, `sortable_attributes: [updated_at]`.
- [ ] 2.3 Bootstrap reconciles existing indexes on every boot — drift in attribute config triggers an `update_settings()` call.

## 3. Doctor check
- [ ] 3.1 Add `cortex-ops doctor meili-indexes` that compares live index settings against the bootstrap config and prints any drift.
- [ ] 3.2 Exit code 0 when all match; 2 when any drift detected.
- [ ] 3.3 Smoke test against the running stack.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/04-event-schema.md` and `docs/specs/06-fulltext.md` + `CHANGELOG.md` Added.
- [ ] 4.2 Tests: §1.4 + bootstrap IT verifying both indexes have expected settings post-boot.
- [ ] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test -p cortex-core -p cortex-workers fulltext` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
