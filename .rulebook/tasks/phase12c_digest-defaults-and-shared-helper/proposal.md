# Proposal: phase12c_digest-defaults-and-shared-helper

Source: `docs/analysis/rework/02-memory-cleanup.md` Achado 2 + Achado 3; `docs/analysis/rework/opus5.7/03-recommendation.md` patches #2 + #3.

## Why

Two tightly coupled gaps in the digest/purge pipeline:

1. The `tool-call-digest` cron runs without `--purge-originals` by default, so digested tool-call envelopes accumulate alongside their digest twins. Storage doubles for no gain.
2. `is_live_partial_frame()` is duplicated across multiple purgers (live-file, archive, digest). Each copy drifts slightly. The 4-doc analysis notes that this is the canonical signal of "missing shared module".

Both fixes are cheap (<1 day each) and unblock cleaner sweep logic in Phase B.

## What Changes

- Flip the default of `tool-call-digest` cron command to include `--purge-originals`.
- Add a startup migration that updates the row's `command` column when it matches the prior default literal.
- Extract `is_live_partial_frame()` to `crates/cortex-storage/src/archive_purge.rs` (the new module from phase12b) and re-export from `crates/cortex-storage/src/lib.rs`.
- Replace every duplicate definition with the shared call. Confirmed locations: `cortex-cli` (live-file purge), `cortex-workers/src/retention/digest_purge.rs`, `cortex-storage/src/archive_purge.rs`.

## Impact

- Affected specs: `docs/specs/19-retention.md` § "Tool-call digest" + § "Live-file partial-frame guard".
- Affected code: `crates/cortex-storage/src/archive_purge.rs`, `crates/cortex-storage/src/lib.rs`, `crates/cortex-workers/src/retention/{scheduler.rs,digest_purge.rs}`, `crates/cortex-cli/src/bin/cortex-ops.rs`.
- Breaking change: NO. The migration is idempotent.
- User benefit: digest cron stops doubling storage; partial-frame guard is single-source-of-truth.
