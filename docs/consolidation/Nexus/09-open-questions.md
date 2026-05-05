# Nexus: Open Questions

Undocumented or unclear items relevant to Cortex's use of Nexus. One bullet per gap with source path inspected.

## Operational / Deployment

- **HNSW parameter tuning for Cortex workload**: Nexus docs mention M, ef defaults but no guidance on profiling recall vs latency trade-offs for Cortex's File/Code label indexes. Inspect: `crates/nexus-core/src/index/hnsw.rs` config section + `docs/performance/tuning.md`.

- **External ID scope across databases**: Nexus consolidation states external IDs are per-database (not global), but unclear if Cortex should use one DB for all repos or separate DBs per environment (dev/staging/prod). Consult: `crates/nexus-core/src/storage/catalog.rs` (external_ids sub-DB initialization).

- **Bulk import performance: batch size vs WAL flush cadence**: Cortex re-indexing ingests 1000s of files; docs don't specify optimal batch size for create_node_with_external_id to avoid WAL thrashing. Inspect: `crates/nexus-core/src/wal/` + `crates/nexus-server/src/http/handlers/cypher.rs` (batch handling).

- **Rate limiting defaults applicable to Cortex**: Nexus config shows per-key: 1k/min, 10k/hour. Unclear if Cortex's bulk re-indexing job would hit those limits or if shared-secret API keys bypass them. Consult: `crates/nexus-server/src/limits.rs`.

## Schema & Graph Model

- **Multi-label constraints in Cortex context**: Nexus supports up to 64 labels per node via bitmap, but consolidation doesn't clarify if Cortex should use multi-label (e.g., File + Testable) or single-label-with-properties. Inspect: `crates/nexus-core/src/storage/nodes.rs` (label_bits layout).

- **Property type enforcement strictness**: Consolidation says "strict, INTEGER ≠ FLOAT" but unclear if Cortex's property mutations (e.g., updating file size during re-index) require explicit type casts or if Nexus auto-coerces. Consult: `crates/nexus-core/src/executor/property.rs` type validation.

- **Cypher UNWIND performance for Cortex batch ingestion**: Nexus docs list UNWIND support, but no benchmark showing if `UNWIND files AS f CREATE (f_node)` is faster than N separate CREATE calls. Inspect: `crates/nexus-core/src/executor/unwind.rs` + benchmarks.

## External IDs & Ingestion

- **File hash collision handling**: Cortex plans to use SHA256(path + content), but docs don't address if two different files happen to have identical hashes (extremely rare). Should Cortex add a tiebreaker (e.g., repo ID)? Consult Nexus team or review: `sdks/rust/examples/external_id.rs`.

- **Re-import semantics if content changes**: Cortex uses MATCH conflict policy (idempotent). If a file's content changes between runs, the node identity (_id) stays the same but properties don't update (MATCH discards new properties). Should Cortex also issue explicit UPDATE queries post-MATCH? Inspect phase 9 decision doc + test: `crates/cortex-workers/tests/nexus_external_id_smoke_it.rs`.

- **DELETE & external ID cleanup**: Consolidation doesn't document whether Cortex must explicitly DELETE nodes for removed files (e.g., deleted from repo) or if Nexus has GC. Inspect: `crates/nexus-core/src/executor/delete.rs` (external ID reverse-index cleanup on delete).

## Querying & Performance

- **Cypher variable-length path cost**: Cortex will use `(file)-[:IMPORTS*1..3]->` to trace dependencies. Consolidation mentions BFS optimization but no cost estimates. Inspect: `crates/nexus-core/src/executor/expand.rs` (variable-length path implementation).

- **KNN + graph traversal latency**: Docs show hybrid KNN queries possible but no latency profile for `CALL vector.knn(label, vec, k) YIELD node MATCH node-[:REL*]->target`. Cortex needs this for semantic search + dependency tracing. Inspect: `crates/nexus-core/src/executor/call.rs` + `nexus-bench/` KNN+expand tests.

- **Relationship indexing**: Nexus has no explicit relationship indexes (only adjacency lists). If Cortex queries `MATCH ()-[r:CALLS]->()`  on millions of CALLS edges, cost unclear. Inspect: `crates/nexus-core/src/executor/match.rs` (relationship filtering strategy).

## Integration & Compatibility

- **SDK backward compatibility**: Consolidation pins Cortex to SDK 2.1.0, but unclear if future Nexus versions maintain wire-protocol compatibility or if Cortex must track SDK bumps. Consult: Nexus `CHANGELOG.md` breaking-change policy.

- **Cypher version evolution**: Nexus 2.1.0 uses "Cypher 25" for some DDL syntax, but unclear which Cypher version the core MATCH/CREATE/etc clauses follow (Neo4j 5.x? custom?). Inspect: `crates/nexus-core/src/parser/` grammar version comments.

- **Long-running query cancellation**: Cortex may issue expensive queries; Nexus supports DELETE /queries/{query_id}, but unclear if cancellation is instantaneous or if in-progress executor work finishes. Inspect: `crates/nexus-server/src/http/handlers/queries.rs` (cancel handler).

## Monitoring & Observability

- **Prometheus metrics retention for Cortex profiling**: Nexus exposes metrics on /prometheus (e.g., cache_hit_ratio, index_lookup_latency), but consolidation doesn't specify if Cortex should scrape metrics per re-index run or aggregate across runs. Consult deployment team.

- **Audit logging scope for external ID mutations**: Consolidation doesn't clarify if create_node_with_external_id calls are audit-logged (for compliance) or if audit is opt-in. Inspect: `crates/nexus-server/src/audit/` (if exists).

## Documentation Gaps (Not Code Specific)

- No end-to-end Cortex ingestion example: docs have file hash external IDs in theory, but no worked example showing Cortex ingesting 100 files, querying by external ID, and handling re-imports. Request: example script or phase11l task artifact.

- No Cortex-specific tuning guide: Nexus has generic cache/HNSW/connection-pool tuning, but no "for code analysis workloads, use X MB cache, M=Y, ef=Z" recommendations. May require profiling in phase11l.

- No data directory size estimation: Cortex needs to provision storage; unclear if 100k files + relationships = 1 GB or 10 GB data/. Request: formula or tool to estimate Nexus data dir size from node/rel counts.
