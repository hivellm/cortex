# Proposal: phase11f_indexer_vue_md_visibility

## Why

Tracking issue: https://github.com/hivellm/cortex/issues/3

`.vue` files are completely invisible to `cortex_query` — the bootstrap
walker classifies them as `FileClass::Other`
(`crates/cortex-cli/src/bootstrap/walker.rs:415-423`), and the emitter
returns `Vec::new()` for `Other`
(`crates/cortex-cli/src/bootstrap/emitter.rs:967`), so no envelope is
ever published. On `synap` this drops the entire 20-file Vue dashboard
layer (`gui/src/**/*.vue`) from retrieval. The same pattern silently
hides any other extension that falls through the Code/Doc/Decision/Law/
Memory/Knowledge/Learning arms.

`.md` files are correctly classified as `FileClass::Doc` and emitted as
`artifact.doc` envelopes, then routed to `cortex-{repo}-docs` by
`crates/cortex-workers/src/fulltext/routing.rs:93`. The pipeline is
wired end-to-end, so the issue's "0 .md hits" on `synap` is a stale
index — `synap` was bootstrapped before docs routing landed (or the
last fan-out used `--kinds` excluding `docs`). The fix is operational
(re-bootstrap with `--kinds docs`), not code.

## What Changes

1. Walker (`crates/cortex-cli/src/bootstrap/walker.rs:417-419`) — add
   `"vue"` to the Code allowlist arm so the SFC reaches the emitter as
   `FileClass::Code`.
2. Fulltext routing (`crates/cortex-workers/src/fulltext/routing.rs:16-21`) —
   add `"vue"` to `CODE_EXTENSIONS` so the artifact lands in
   `cortex-{repo}-code` instead of `misc`.
3. Tests — extend `classify_path_via_public_api`
   (`crates/cortex-cli/tests/bootstrap_runner.rs:739`) and
   `family_for_event_uses_path_extension_for_artifacts`
   (`crates/cortex-workers/src/fulltext/routing.rs:264`) with `.vue`
   coverage.
4. Operational — re-run `cortex-bootstrap --repo synap --kinds
   docs,code` to backfill the missing `cortex-synap-docs` index and
   pick up the new `.vue` artifacts.

SFC-aware splitting (`<template>` / `<script>` / `<style>` blocks) is
**out of scope** for this fix — getting the file into the index closes
the visibility gap; better chunking is a later optimization.

## Impact

- Affected specs: `docs/specs/09-bootstrap-cli.md` (extension list note),
  `docs/specs/08-fulltext-indexer.md` (CODE_EXTENSIONS note)
- Affected code: `crates/cortex-cli/src/bootstrap/walker.rs`,
  `crates/cortex-workers/src/fulltext/routing.rs`,
  `crates/cortex-cli/tests/bootstrap_runner.rs`,
  `crates/cortex-workers/tests/fulltext_routing.rs`
- Breaking change: NO (additive — only new extensions accepted)
- User benefit: Vue dashboards in `synap` (and any future Hive repo
  with a Vue GUI) become retrievable via `cortex_query`; closes #3.
