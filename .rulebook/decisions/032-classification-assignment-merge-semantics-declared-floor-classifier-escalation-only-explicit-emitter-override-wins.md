# 32. Classification assignment merge semantics: declared floor + classifier escalation-only; explicit emitter override wins

**Status**: proposed
**Date**: 2026-06-23
**Related Tasks**: phase21_data-classification-access-control

## Context

A fact's classification can come from three sources: a path/kind rule in `cortex.toml` (declared at bootstrap time), content-based detection by the classifier worker (ML/heuristic), and an explicit field on the envelope from a trusted emitter. These can conflict. We need a deterministic merge rule that prevents downgrade (an operator-declared `confidential` fact must never be silently demoted to `internal` by the classifier) while allowing the classifier to surface sensitivity it can detect.

## Decision

Three-source merge with a total order: `level = max(declared, detected)` and `compartments = union(declared, detected)`, with explicit emitter values as an upper override. Specifically: (1) declared — the path/kind rule in `cortex.toml` is the floor; the result is always at least as sensitive as declared. (2) detected — the classifier worker's content-based output may only escalate level (`max`) and only union compartments; it can NEVER lower a declared level or remove a declared compartment. (3) explicit — an `Envelope.class_level` / `Envelope.class_compartments` set by a trusted emitter (e.g. a custom ingest client that knows the data provenance) is the upper override and wins over both declared and detected when present, subject to the operator having configured that emitter as trusted. The merge runs as a single pure function `merge_classification(declared, detected, explicit) -> Classification` with exhaustive unit tests over the precedence chain. A classifier that would lower a declared classification MUST log a warning and be ignored.

## Alternatives Considered

- Classifier wins over declared — rejected: a misconfigured or adversarial classifier could downgrade a `restricted` fact to `public`, leaking it. The declared floor is the operator's intent and must be inviolable.
- Union of all sources for level (max of all three) with no explicit override — rejected: a trusted emitter that knows the document's authoritative classification should be able to set it directly; requiring it to always pass through the path-rule system is inflexible.
- Last-write wins — rejected: non-deterministic (depends on processing order); re-runs would produce different results.

## Consequences

Pros: the declared floor is inviolable, preventing downgrade attacks; the classifier can add sensitivity it detects without operator overhead; the explicit override path handles ingestion pipelines with authoritative classification metadata; the merge is a pure, testable function with no side effects. Cons: an operator who sets an overly restrictive path rule cannot have the classifier lower it (they must change the rule); the explicit override requires trust configuration (operators must explicitly mark an emitter as trusted).
