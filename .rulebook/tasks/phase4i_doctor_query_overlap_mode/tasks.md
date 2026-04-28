## 1. Per-lane query traits
- [ ] 1.1 Define `QueryProbe` trait in `cortex-ops::doctor::probe` returning `Vec<String>` (top-K paths) for a given `(query, k)`
- [ ] 1.2 Implement `MeiliQueryProbe` over every canonical index (parallel fan-out)
- [ ] 1.3 Implement `VectorizerQueryProbe` over every canonical collection
- [ ] 1.4 Implement `NexusQueryProbe` via Cypher `CONTAINS` substring match on `Artifact.body`

## 2. Jaccard computation + thresholds
- [ ] 2.1 Compute pairwise Jaccards `|A∩B|/|A∪B|` between (vec, meili), (vec, nexus), (meili, nexus)
- [ ] 2.2 Compute triple-intersection size `|A∩B∩C|`
- [ ] 2.3 Read `min_overlap_jaccard` from `cortex-doctor.toml` (default 0.2)
- [ ] 2.4 Mark the run failed when any pair falls below the threshold

## 3. CLI surface
- [ ] 3.1 `--query <q>` (repeatable) on `doctor-consistency`
- [ ] 3.2 Per-query JSON entry in `DoctorReport.queries: Vec<QueryReport>`
- [ ] 3.3 Markdown rendering: one block per query with per-lane top-K + Jaccards

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation — extend the spec-08 doctor section with the probe-mode contract
- [ ] 4.2 Write tests covering the new behavior — unit tests with synthetic per-lane top-K results; assert Jaccards round-trip and threshold flag fires
- [ ] 4.3 Run tests and confirm they pass — `cargo check -p cortex-ops` → `cargo clippy -p cortex-ops --all-targets -- -D warnings` → `cargo test -p cortex-ops`
