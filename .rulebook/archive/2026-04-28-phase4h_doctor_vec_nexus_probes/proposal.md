# Proposal: phase4h_doctor_vec_nexus_probes

## Why

`phase4d_indexing_consistency_doctor` shipped the archive ↔ Meili
axis of `cortex-ops doctor-consistency`. Vectorizer and Nexus
probes were carved out because each needs its own auth flow:
Vectorizer's admin endpoints sit behind `/auth/login`
(`CORTEX_EMBEDDER_VECTORIZER_USER` / `_PASSWORD`) and Nexus speaks
Cypher over the bolt protocol.

Without those probes, the doctor cannot answer "is the partition
that lives in Meili also in the vector lane and the graph?" — the
exact question the 2026-04-27 audit asked.

## What Changes

- `VectorizerProbe`: authenticate via `/auth/login`, list
  collections, parse `cortex-{repo}-{family}` collection names,
  map to `(repo, family) → vector_count`.
- `NexusProbe`: run `MATCH (a:Artifact)-[:IN_REPO]->(r:Repo) RETURN
  r.name, count(a)` against the configured `CORTEX_NEXUS_URL`. Map
  to `(repo, family=N/A) → artifact_count` since Nexus is
  repo-grain only.
- Extend `DoctorReport` and `CoverageRow` with `vec_vectors` /
  `nexus_artifacts` columns. Update `render_coverage_markdown` to
  emit them.
- Tolerance: `vec_to_meili_ratio_max` (default 50) — when
  `vec_vectors / meili_docs > ratio_max` and both are positive,
  mark the row suspicious (not failed). Chunking can legitimately
  multiply but 100× warrants a manual look.
- Update the spec-08 cross-backend consistency doctor section.

## Impact

- Affected specs: spec-08 (doctor section gains the two probes).
- Affected code:
  - `crates/cortex-ops/src/doctor.rs` — extend `DoctorReport` +
    `CoverageRow`, add `VectorizerProbe` / `NexusProbe`
  - `crates/cortex-ops/src/main.rs` — wire env-driven probe
    construction
  - tests: unit tests against in-memory probes
- Breaking change: NO. New columns are additive; v1 reports
  remain valid as a subset.
- Depends on: phase4d (the doctor scaffolding) + phase4c
  (Nexus's `IN_REPO` graph state).
- User benefit: the doctor reports the full backend matrix, not
  just two of four.

## Source

- Carved out of `phase4d_indexing_consistency_doctor` items
  1.2–1.3 because each backend probe needs its own auth flow
  and HTTP client wrapper, expanding scope past one PR.
