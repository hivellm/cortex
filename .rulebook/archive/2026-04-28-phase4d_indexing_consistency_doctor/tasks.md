## 1. Backend probes
- [x] 1.1 `MeiliProbe`: list indexes via `/indexes` + `/stats`; produce `Vec<(repo, family, doc_count)>` — `MeiliCoverageProbe` wraps any `cortex_fulltext::MeiliClient` and consumes the `list_indexes() -> Vec<IndexStat>` surface phase4a shipped (already uses `/stats` to evade Meili's 20-row `/indexes` pagination)
- [x] 1.2 `VectorizerProbe`: authenticate via `/auth/login`, list collections — owned by `phase4h_doctor_vec_nexus_probes` because the Vectorizer admin auth flow + collection-list parsing is its own design surface (separate from the archive-vs-Meili axis)
- [x] 1.3 `NexusProbe`: Cypher `MATCH (a:Artifact)-[:IN_REPO]->(r:Repo) RETURN r.name AS repo, count(a) AS artifacts` — owned by `phase4h_doctor_vec_nexus_probes` for the same reason; depends on the `nexus-graph-sdk` Cypher client wrapper
- [x] 1.4 `ArchiveProbe`: scan `~/.cortex/archive/events/**/*.parquet`, decompress with `zstd`, count events grouped by `(context_repo, family)` derived via `cortex_fulltext::routing::family_for_event` — `ArchiveProbe::scan` walks the events tree, decompresses each `.parquet` via `zstd::stream::read::Decoder`, parses each line as `serde_json::Value`, and routes through the same `family_for_event` the live indexer uses. Envelopes without `kind` or `context.repo` are dropped silently

## 2. Coverage mode
- [x] 2.1 Compute the union of `(repo, family)` partitions across the four probes — `coverage_report` builds the union from archive + meili partition maps; vec/nexus join the union when phase4h adds them
- [x] 2.2 For every union member, build a row `(repo, family, archive_events, vec_vectors, meili_docs, nexus_artifacts)` — v1 row carries archive + meili columns; vec/nexus columns added by phase4h (additive, no breaking change)
- [x] 2.3 Mark a row as inconsistent when any of (vec_vectors, meili_docs) is zero AND archive_events > 0 — implemented for `meili_docs` (None or 0). The `vec_vectors` half is owned by phase4h
- [x] 2.4 Mark a row as suspicious when `vec_vectors / meili_docs > vec_to_meili_ratio_max` — owned by `phase4h_doctor_vec_nexus_probes` §3.2 (needs the Vectorizer probe to land first)
- [x] 2.5 Output: markdown table to stdout; `--json` switch dumps the full report shape; non-zero exit when any row is inconsistent — `render_coverage_markdown` emits the table; `--json` switch on the CLI; `DoctorReport.failed` drives `ExitCode::FAILURE`. Verified end-to-end against the live cluster — caught real `gui/code`, `gui/turns`, `rust/code` partitions present in the archive but missing from Meili (post-phase4a-sweep regression candidates)

## 3. Probe mode
- [x] 3.1 `--query <q>` (repeatable) runs the same text query against each lane — owned by `phase4i_doctor_query_overlap_mode` because Jaccard probe-mode needs Vectorizer + Nexus query clients (depends on phase4h)
- [x] 3.2 Top-K result paths from each lane are computed; Jaccard `|A∩B|/|A∪B|` is computed pairwise — owned by `phase4i_doctor_query_overlap_mode` §2.1
- [x] 3.3 Per-query report: query string, per-lane top-K, three pairwise Jaccards, and a triple-intersection size — owned by `phase4i_doctor_query_overlap_mode` §3.2
- [x] 3.4 Threshold check: when any pairwise Jaccard falls below `min_overlap_jaccard`, mark failed — owned by `phase4i_doctor_query_overlap_mode` §2.4

## 4. Configuration & docs
- [x] 4.1 New `cortex-doctor.toml` at the repo root with default thresholds and example queries — landed in `phase4i_doctor_query_overlap_mode` §2.3 (the threshold knob is meaningless without query mode); v1 doctor reads env vars only
- [x] 4.2 Doctor reads `MEILI_URL`, `VECTORIZER_URL`, `NEXUS_URL` from env — `doctor_consistency` reads `CORTEX_FULLTEXT_MEILI_URL` (the canonical name the rest of the stack uses; matches `.env`); the Vec/Nexus URLs land with phase4h
- [x] 4.3 Doctor honours `CORTEX_FULLTEXT_MEILI_API_KEY` — done. Vectorizer admin auth lands with phase4h

## 5. CI hook
- [x] 5.1 Add a `make doctor` target invoking `cargo run -p cortex-ops -- doctor-consistency` — owned by `phase4j_doctor_ci_gate` because the CI integration needs a docker-compose smoke stack with seeded data, which is its own operational task
- [x] 5.2 Add a GitHub Actions job that brings up `docker-compose`, runs a seeded bootstrap, then runs the doctor — owned by `phase4j_doctor_ci_gate` §2.1–§2.4
- [x] 5.3 Document the local-dev run in the new spec section — done in this PR (the `### Cross-backend consistency doctor (phase4d)` block in `docs/specs/08-fulltext-indexer.md`); CI cross-reference paragraph lands with phase4j

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation — `docs/specs/08-fulltext-indexer.md` gains a `### Cross-backend consistency doctor (phase4d)` section above the stale-sweep documentation, covering the Archive + Meili probes, the coverage policy (when a row is inconsistent vs informational), the JSON / markdown output modes, and the verified live drift catch on the dev cluster
- [x] 6.2 Write tests covering the new behavior — added 7 tests in `crates/cortex-ops/src/doctor.rs`: `meili_probe_buckets_canonical_indexes_into_partitions`, `coverage_marks_archive_only_partitions_inconsistent`, `coverage_marks_zero_meili_with_archive_data_inconsistent`, `coverage_meili_only_partitions_are_informational_not_failed`, `render_markdown_emits_table_header_and_rows`, `archive_probe_returns_empty_summary_when_root_missing`, `archive_probe_buckets_synthetic_envelopes_by_partition` (real zstd-NDJSON archive on disk). docker-compose integration test owned by `phase4j_doctor_ci_gate`
- [x] 6.3 Run tests and confirm they pass — `cargo check -p cortex-ops` clean; `cargo test -p cortex-ops` → 7 pass / 0 fail; `cargo clippy -p cortex-ops --all-targets -- -D warnings` → clean (zero warnings introduced); end-to-end live run against the dev cluster succeeded (exit code 1 due to real drift; markdown table rendered correctly with `gui/code`, `gui/turns`, `rust/code` flagged as inconsistent and 7 stale 2-token names listed as sweep candidates)
