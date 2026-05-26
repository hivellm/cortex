## 1. Walker
- [x] 1.1 Add `"vue"` to the Code allowlist arm in `crates/cortex-cli/src/bootstrap/walker.rs:417-419`
- [x] 1.2 Extend `classify_path_via_public_api` in `crates/cortex-cli/tests/bootstrap_runner.rs` with a `.vue` → `FileClass::Code` assertion

## 2. Fulltext routing
- [x] 2.1 Add `"vue"` to `CODE_EXTENSIONS` in `crates/cortex-workers/src/fulltext/routing.rs:16-21`
- [x] 2.2 Extend `family_for_event_uses_path_extension_for_artifacts` in `crates/cortex-workers/src/fulltext/routing.rs:264` with a `.vue` → `code` assertion

## 3. Quality gates
- [x] 3.1 `cargo check -p cortex-cli -p cortex-workers` — green (only pre-existing dead-code warnings in `cortex-api`, unrelated)
- [x] 3.2 `cargo clippy -p cortex-cli -p cortex-workers -- -D warnings` — pre-existing `field-reassign-with-default` debt in `cortex-retention` (transitive dep) blocks the lint; not introduced by this change. Documented for separate cleanup task.
- [x] 3.3 `cargo test -p cortex-cli --test bootstrap_runner` — 18 / 18 passed
- [x] 3.4 `cargo test -p cortex-workers --lib fulltext::routing` — 12 / 12 passed

## 4. Operational backfill (synap docs index)
- [x] 4.1 Documented in proposal — `cortex-bootstrap --repo synap --kinds docs,code` is required to backfill the missing `cortex-synap-docs` index and pull in the new `.vue` artifacts.
- [x] 4.2 Backfill is user-side (synap repo is not local under the Cortex working tree); marking complete with handoff note in proposal.

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation — `docs/specs/09-bootstrap-cli.md` walker note + `docs/specs/08-fulltext-indexer.md` `CODE_EXTENSIONS` list updated
- [x] 5.2 Write tests covering the new behavior — `.vue` cases added in `crates/cortex-cli/tests/bootstrap_runner.rs` (§1.2) and `crates/cortex-workers/src/fulltext/routing.rs` (§2.2)
- [x] 5.3 Run tests and confirm they pass — bootstrap_runner 18/18 green, fulltext::routing 12/12 green
