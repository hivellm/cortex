## 1. Backend probes
- [ ] 1.1 `MeiliProbe`: list indexes via `/indexes` + `/stats`; produce `Vec<(repo, family, doc_count)>`
- [ ] 1.2 `VectorizerProbe`: authenticate via `/auth/login`, list collections, produce `Vec<(repo, family, vector_count)>` (parse the `cortex-{repo}-{family}` name)
- [ ] 1.3 `NexusProbe`: Cypher `MATCH (a:Artifact)-[:IN_REPO]->(r:Repo) RETURN r.name AS repo, count(a) AS artifacts`; map to `Vec<(repo, family=N/A, artifact_count)>` (Nexus is repo-grain only without family)
- [ ] 1.4 `ArchiveProbe`: scan `~/.cortex/archive/events/**/*.parquet`, decompress with `zstd`, count events grouped by `(context_repo, family)` derived via `cortex_fulltext::routing::family_for_event`

## 2. Coverage mode
- [ ] 2.1 Compute the union of `(repo, family)` partitions across the four probes
- [ ] 2.2 For every union member, build a row `(repo, family, archive_events, vec_vectors, meili_docs, nexus_artifacts)`
- [ ] 2.3 Mark a row as inconsistent when any of (vec_vectors, meili_docs) is zero AND archive_events > 0
- [ ] 2.4 Mark a row as suspicious when `vec_vectors / meili_docs > vec_to_meili_ratio_max` (config-driven; default 50)
- [ ] 2.5 Output: markdown table to stdout; `--json` switch dumps the full report shape; non-zero exit when any row is inconsistent

## 3. Probe mode
- [ ] 3.1 `--query <q>` (repeatable) runs the same text query against each lane via the existing `cortex-api` clients (or direct backend HTTP)
- [ ] 3.2 Top-K (default 10) result paths from each lane are computed; Jaccard `|A∩B|/|A∪B|` is computed pairwise (vec↔meili, vec↔nexus, meili↔nexus)
- [ ] 3.3 Per-query report: query string, per-lane top-K, three pairwise Jaccards, and a triple-intersection size
- [ ] 3.4 Threshold check: when any pairwise Jaccard for a query falls below `min_overlap_jaccard`, mark the run failed and exit non-zero

## 4. Configuration & docs
- [ ] 4.1 New `cortex-doctor.toml` at the repo root with default thresholds and example queries
- [ ] 4.2 Doctor reads `MEILI_URL`, `VECTORIZER_URL`, `NEXUS_URL` from env (matching the existing `.env` shape)
- [ ] 4.3 Doctor honours `MEILI_MASTER_KEY` and the same Vectorizer admin auth used by `cortex-embedder` (`/auth/login` with `CORTEX_EMBEDDER_VECTORIZER_USER` / `_PASSWORD`)

## 5. CI hook
- [ ] 5.1 Add a `make doctor` target invoking `cargo run -p cortex-ops -- doctor consistency`
- [ ] 5.2 Add a GitHub Actions job (or extend existing) that brings up `docker-compose`, runs a seeded bootstrap, then runs the doctor; non-zero exit fails the workflow
- [ ] 5.3 Document the local-dev run in the new spec section

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation (new `## Doctor` section in spec-13 or a fresh spec-14)
- [ ] 6.2 Write tests covering the new behavior (unit tests against mocked backends with `wiremock`; integration test against a docker-compose seeded fixture)
- [ ] 6.3 Run tests and confirm they pass (`cargo check -p cortex-ops` → `cargo clippy -p cortex-ops --all-targets -- -D warnings` → `cargo test -p cortex-ops`)
