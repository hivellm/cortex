## 1. Move scheduler module
- [x] 1.1 Copy `crates/cortex-cli/src/ops/scheduler.rs` to `crates/cortex-retention/src/scheduler.rs` verbatim
- [x] 1.2 Add `pub mod scheduler;` to `cortex-retention/src/lib.rs`
- [x] 1.3 Add `cron` to `cortex-retention/Cargo.toml`
- [x] 1.4 Delete the original file from `cortex-cli`
- [x] 1.5 Update `cortex-cli/src/ops/mod.rs` to `pub use cortex_retention::scheduler;`
- [x] 1.6 Update `cortex-cli/src/bin/cortex-ops.rs` import path

## 2. Wire the always-on tick loop
- [x] 2.1 Add `cortex-retention` to `crates/cortex-api/Cargo.toml`
- [x] 2.2 New `crates/cortex-api/src/retention_daemon.rs` exposing `spawn(metadata, opts)`
- [x] 2.3 Call `seed_defaults` once at boot; spawn the 30 s tick loop
- [x] 2.4 Honour `CORTEX_RETENTION_DAEMON=disabled` opt-out
- [x] 2.5 Wire from `cortex-api/src/main.rs` next to `silent-drop`

## 3. Tests
- [x] 3.1 Unit test: `seed_defaults` is idempotent across two daemon boots
- [x] 3.2 Unit test: tick loop calls runner exactly once per due job (use `MemoryRunner`)
- [x] 3.3 Unit test: opt-out env var skips the spawn

## 4. Spec / docs
- [x] 4.1 Update `docs/specs/19-retention.md` §scheduler to point at `cortex-api`
- [x] 4.2 Document `CORTEX_RETENTION_DAEMON` opt-out in the README

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation
- [x] 5.2 Write tests covering the new behavior
- [x] 5.3 Run tests and confirm they pass
