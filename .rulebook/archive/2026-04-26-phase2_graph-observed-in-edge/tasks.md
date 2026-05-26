## 1. Schema bump
- [x] 1.1 Add required `observed_event_kind` enum field (`turn` | `tool_call`) to `crates/cortex-core/schemas/kinds/law_violation.schema.json` next to `observed_event_id`
- [x] 1.2 Mirror the field on `cortex_core::events::LawViolation` Rust type with serde rename matching the schema property name
- [x] 1.3 `cortex_core::validate_event` rejects a `law_violation` envelope missing `observed_event_kind` with a clear message; fixture under `crates/cortex-core/tests/fixtures/events/law_violation.json` updated and passes the validator round-trip

## 2. Emitter updates
- [x] 2.1 The spec-10 PreToolUse sync path that produces a `law_violation` envelope on a denied call stamps `observed_event_kind = "tool_call"` (the kind of the upstream `tool_call` event the detector observed)
- [x] 2.2 Other emitters (spec-13 detector framework, spec-14 governance engine) populate the field from the inbound event's discriminator; default-derive helpers refuse to construct a `LawViolation` payload without it

## 3. Graph writer wiring
- [x] 3.1 `cortex-graph::writer` adds the `OBSERVED_IN` Cypher MERGE that picks `Turn` or `ToolCall` as the target label based on `observed_event_kind`; no phantom-node fallback path
- [x] 3.2 Integration test in `cortex-graph/tests/` writes a Turn + LawViolation pair and asserts the `OBSERVED_IN` edge exists at the correct label
- [x] 3.3 Update `phase1_graph-writer` §4.5 cross-reference: archived parent task's footnote points at this follow-up by id

## 4. Tail (mandatory)
- [x] 4.1 Update or create documentation covering the implementation — note the schema field in `docs/specs/04-cortex-core.md` LawViolation section and in `docs/specs/06-graph-writer.md` (or whatever spec owns the graph writer); fixture files under `crates/cortex-core/tests/fixtures/` updated
- [x] 4.2 Write tests covering the new behavior — schema validator unit test for the missing-field rejection, graph-writer integration test for the `OBSERVED_IN` edge under each label, end-to-end test that runs through a denied PreToolUse and asserts the violation envelope's `observed_event_kind` is `tool_call`
- [x] 4.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p cortex-core`, `cargo test -p cortex-graph`, `cargo test -p cortex-adapter-claude-code`
