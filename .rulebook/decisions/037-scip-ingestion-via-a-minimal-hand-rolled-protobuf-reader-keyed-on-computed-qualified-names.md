# 37. SCIP ingestion via a minimal hand-rolled protobuf reader keyed on computed qualified names

**Status**: proposed
**Date**: 2026-07-07
**Related Tasks**: phase27d_scip-precise-extraction, phase23c_ua-extraction-contract, phase30_live-e2e-smoke-and-doctor-wiring

## Context

Phase27d adds rust-analyzer SCIP output as the higher-precision deterministic-facts backend for graph extraction (per the phase23c reconciliation: same extraction contract, same Symbol label, EdgeConfidence::Extracted). Three implementation decisions needed real-data grounding, so the schema was captured from actual rust-analyzer 1.96.0 output on a purpose-built fixture crate (committed as tests/fixtures/scip/rust_analyzer_1_96_fixture.scip; full field map in the task's design.md) rather than coded from memory — which immediately corrected the task's own premise: the emitter produces protobuf, not JSON.

## Decision

1) Parse the protobuf directly with a minimal hand-rolled wire reader (~60 lines: varint + length-delimited + skip) covering exactly the fields Cortex consumes — no prost/protoc build dependency and no Sourcegraph scip CLI requirement, keeping bootstrap/CI setup unchanged. The wire format is stable and the consumed field set is small and pinned by the committed real fixture. 2) Key symbol resolution on the COMPUTED qualified name, not raw SCIP symbol-string equality: real output uses `Worker#` for a struct's definition but bare `impl#[Worker]` for Self-references inside its impl block — two raw strings for one logical entity; normalizing both to `Worker` unifies them without special-cased rewrites. 3) Emit into the existing conventions verbatim: `:Symbol` keyed `repo|rust|qualified_name` (tree-sitter analyzer's scheme), DEFINES/CALLS/REFERENCES at Extracted(1.0) with analyzer="scip", `:ScipExternal` stubs (keyed scheme|manager|package|descriptors) for cross-crate/unresolved targets so edges never dangle, and `local N` symbols emit nothing. 4) Bootstrap/CI invocation (§2.4) is deliberately deferred to an operator decision: requiring rust-analyzer as a bootstrap tool changes operator setup and belongs with the phase30 CI work.

## Alternatives Considered

- prost + scip.proto codegen: rejected — adds protoc to every build environment (the exact class of Windows/CI tooling friction the platform analysis flagged) for a schema of which Cortex consumes a dozen fields
- Sourcegraph scip CLI JSON conversion step: rejected — an extra external Go binary per environment just to convert a format we can read directly
- Raw symbol-string resolution keys: rejected on real fixture evidence — misses Self-references (impl#[T] vs T#) and would emit broken/missing edges for the most common intra-impl call pattern

## Consequences

Positive: zero new build/runtime dependencies for parsing; resolution verified cross-module on real data (Worker::new −CALLS→ storage::Store::open); the committed real fixture pins the schema so an upstream rust-analyzer format change surfaces as a test failure, not silent misparsing. Tradeoffs: the hand-rolled reader only understands the consumed field subset — new SCIP fields require touching the decoder (acceptable: additive protobuf fields skip cleanly by design); qualified-name keying could collide across identical names in different modules IF rust-analyzer ever emits ambiguous descriptors (not observed; the descriptors carry module paths); live graph value still rides the projection unblock + §2.4 operator go-ahead.
