## 1. VectorizerProbe
- [x] 1.1 Wrap `vectorizer-sdk`'s authenticated admin client; authenticate via `/auth/login` (`CORTEX_EMBEDDER_VECTORIZER_USER` + `_PASSWORD`)
- [x] 1.2 List collections; parse `cortex-{repo}-{family}` names into `PartitionKey`; sum vector counts per partition
- [x] 1.3 Surface non-canonical collection names in a separate `non_canonical_vectorizer_collections` field on `DoctorReport`

## 2. NexusProbe
- [x] 2.1 Wrap `nexus-graph-sdk`'s Cypher client; query `MATCH (a:Artifact)-[:IN_REPO]->(r:Repo) RETURN r.name AS repo, count(a) AS artifacts`
- [x] 2.2 Map result rows to `(repo, family=N/A)` — graph is repo-grain only without family
- [x] 2.3 Add a `nexus_artifacts: Option<u64>` column to `CoverageRow` and render it in the markdown table

## 3. Coverage policy update
- [x] 3.1 Extend `coverage_report` to read `vec_partitions` + `nexus_repo_counts` and stitch them into rows
- [x] 3.2 Add `vec_to_meili_ratio_max` tolerance (default 50); mark a row `suspicious=true` when both counts are positive and the ratio exceeds the bound (does NOT fail the run)
- [x] 3.3 Update `render_coverage_markdown` to add `vec` and `nexus` columns and a `suspicious` status

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 4.1 Update or create documentation covering the implementation — extend the `### Cross-backend consistency doctor (phase4d)` section in `docs/specs/08-fulltext-indexer.md` with the two new probes and the suspicious-row policy
- [x] 4.2 Write tests covering the new behavior — unit tests against in-memory `VectorizerProbe` / `NexusProbe` traits seeded with synthetic counts; assert the table widens correctly and the ratio-bound flag fires
- [x] 4.3 Run tests and confirm they pass — `cargo check -p cortex-ops` → `cargo clippy -p cortex-ops --all-targets -- -D warnings` → `cargo test -p cortex-ops`
