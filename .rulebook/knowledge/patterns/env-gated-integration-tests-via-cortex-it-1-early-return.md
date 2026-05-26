# Env-gated integration tests via CORTEX_*_IT=1 early return

**Category**: integration
**Tags**: testing, integration, cargo-test, ci, cortex-embedder

## Description

Gate live-stack integration tests on an environment variable (`CORTEX_*_IT=1`) and early-return inside the `#[tokio::test]` when unset, rather than tagging with `#[ignore]`.

```rust
fn it_enabled() -> bool {
    std::env::var("CORTEX_EMBEDDER_IT").as_deref() == Ok("1")
}

#[tokio::test]
async fn my_live_test() {
    if !it_enabled() {
        eprintln!("skipping: CORTEX_EMBEDDER_IT != 1");
        return;
    }
    // ... real live-stack assertions ...
}
```

**Why this beats `#[ignore]`**:
- Default `cargo test` is green in CI without any extra flag; no `-- --ignored` required.
- Skipped tests report `ok` in the cargo output, so a ten-test suite that is mostly live still shows `test result: ok. 10 passed` on the default run — CI dashboards stay clean.
- The early-return path can print a human-readable "skipping" line to stderr so local developers see why the test didn't exercise the live path.
- No risk that `-- --ignored` masks real regressions by sweeping in tests that were never meant to run together.

Used in `crates/cortex-embedder/tests/common/mod.rs::it_enabled()` and every `#[tokio::test]` in `tests/it_vectorizer.rs`, `tests/it_end_to_end.rs`.

## Example

// tests/common/mod.rs
pub fn skip_if_not_it() -> bool {
    if !it_enabled() {
        eprintln!("skipping: CORTEX_EMBEDDER_IT != 1");
        return true;
    }
    false
}

// tests/it_vectorizer.rs
#[tokio::test]
async fn ensure_collection_is_idempotent() {
    if skip_if_not_it() { return; }
    // live assertions
}

## When to Use

For integration tests that require a live external service (Vectorizer, Synap, Nexus, etc.) and should be opt-in in CI.

## When NOT to Use

For pure-Rust tests that never hit the network — those should run unconditionally. Also not suitable when you need the test binary itself to exit non-zero on skip (use `#[ignore]` with a wrapper script instead).
