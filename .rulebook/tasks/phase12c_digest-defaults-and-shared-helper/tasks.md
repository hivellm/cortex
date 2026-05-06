## 1. tool-call-digest default flip
- [ ] 1.1 Update `seed_defaults` in `cortex-workers/src/retention/scheduler.rs` so the `tool-call-digest` row's `command` ends with `--purge-originals`.
- [ ] 1.2 Add an idempotent migration: existing rows whose `command` matches the prior literal get rewritten on boot. Operator-edited commands stay untouched (compare against the pre-flip default exactly).
- [ ] 1.3 Regression test: seed pre-flip default → boot → row carries the new flag.

## 2. Shared partial-frame helper
- [ ] 2.1 Move `is_live_partial_frame(path: &Path) -> bool` to `crates/cortex-storage/src/archive_purge.rs`. Re-export from `cortex_storage` root.
- [ ] 2.2 Replace the duplicate in `crates/cortex-cli/src/bin/cortex-ops.rs::live_file_purge` with the shared call.
- [ ] 2.3 Replace the duplicate in `crates/cortex-workers/src/retention/digest_purge.rs` with the shared call.
- [ ] 2.4 `grep -r "fn is_live_partial_frame" crates/` MUST return exactly one definition after the change.
- [ ] 2.5 Unit tests: 4 cases pinned in `archive_purge.rs` (complete frame, partial frame, missing footer, empty file).

## 3. Tail (mandatory)
- [ ] 3.1 Update `docs/specs/19-retention.md` and `CHANGELOG.md` `[Unreleased]` Changed.
- [ ] 3.2 Tests: §1.3 + §2.5 + grep assertion in CI.
- [ ] 3.3 `cargo check --workspace && cargo clippy -- -D warnings && cargo test --workspace` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
