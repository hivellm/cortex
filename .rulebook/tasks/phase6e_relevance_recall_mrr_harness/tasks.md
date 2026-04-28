## 1. Crate scaffold
- [ ] 1.1 Create `crates/cortex-relevance-eval/` with `Cargo.toml` declaring a `[[bin]] name = "cortex-relevance-eval"` and dependencies on `serde`, `serde_yaml`, `reqwest`, `tokio`, `clap`, `tracing`, `tracing-subscriber`, `cortex-api` (workspace path)
- [ ] 1.2 Add the crate to the workspace members list in the root `Cargo.toml`
- [ ] 1.3 Module layout: `src/main.rs` (CLI entry), `src/queries.rs` (fixture loader), `src/harness.rs` (run + score), `src/report.rs` (JSON emit)

## 2. Labeled query set
- [ ] 2.1 Author `tests/relevance/queries.yaml` with ≥50 entries — ≥10 per intent across `pre_change_context`, `decision_lookup`, `similar_problems`, `law_check`, `explain`
- [ ] 2.2 Each entry: `id` (stable, `rel-NNN`), `intent`, `scope` (`{repo, files?, topics?}`), `query`, `expected_doc_ids` (≥1), optional `notes`
- [ ] 2.3 Source the expected doc ids by hand-curating real envelopes from the seeded archive — `cortex-Cortex-{misc,decisions,turns,governance}` indexes are the source of truth
- [ ] 2.4 Add a `tests/relevance/README.md` documenting the curation process so future contributors can extend the set without breaking determinism

## 3. Harness — fetch + score
- [ ] 3.1 In `harness.rs`, accept `--api-url` (default `http://127.0.0.1:17000`) and `--query-set` (default `tests/relevance/queries.yaml`) via `clap`
- [ ] 3.2 Boot loop: for each query, POST to `/v1/query` with the fixture body; parse response.results.snippets → `Vec<String>` of doc ids
- [ ] 3.3 Score per query: `recall_at_10 = expected_ids.iter().any(|id| top10.contains(id))` (boolean); `mrr = 1.0 / (rank_of_first_match as f32)` or `0.0` when absent
- [ ] 3.4 Backend health check: hit `/v1/status` first; when any backend (vector/keyword/graph) reports unhealthy, set `report.omitted_intents = [...]` and exclude the buckets that depend on it from the run
- [ ] 3.5 Per-intent + global aggregates: `recall_at_10_pct = matches / total * 100.0`, `mrr_avg = sum / total`

## 4. Report
- [ ] 4.1 In `report.rs`, define `RelevanceReport { generated_at, git_sha, api_version, per_intent: BTreeMap<Intent, IntentScores>, global: IntentScores, omitted_intents, queries: Vec<QueryResult> }`
- [ ] 4.2 Emit pretty-printed JSON to `target/relevance/<git-sha>.json` (auto-create the directory)
- [ ] 4.3 Print a human-readable summary to stdout (table per intent + global) for local runs
- [ ] 4.4 Exit code: `0` on success; `2` on regression detected (CI gate uses this); `1` on harness error (config / network / fixture)

## 5. Regression detection
- [ ] 5.1 Add `--baseline <path>` flag — when supplied, load that previous JSON report and compute deltas
- [ ] 5.2 Hard gate: global `recall_at_10` MUST be within `2%` of baseline (absolute pp); same for `mrr_avg`
- [ ] 5.3 Soft gate: per-intent metrics within `2%` print warnings but do not fail the run
- [ ] 5.4 Surface the deltas + the worst 5 regressed queries (by id) in the stdout summary

## 6. CI workflow
- [ ] 6.1 Add `.github/workflows/relevance.yaml` (or extend the existing CI workflow if it lives elsewhere): job runs on PRs touching any retrieval-path file pattern
- [ ] 6.2 Steps: build harness → start cortex-api against the seeded fixture stack → run harness → upload `target/relevance/<sha>.json` as artifact
- [ ] 6.3 Cache previous main report as a CI artifact named `relevance-baseline.json`; pass `--baseline` to the harness so the regression gate fires
- [ ] 6.4 Document the CI integration in `docs/specs/16-dashboard.md` so the GUI's tasks/relevance views can surface the JSON report stream

## 7. Persistence to `.rulebook/learnings/`
- [ ] 7.1 On merge to main, the CI step ALSO copies `target/relevance/<sha>.json` to `.rulebook/learnings/relevance/<YYYY-MM-DD>-<sha>.json`
- [ ] 7.2 Commit + push via the standard CI bot path; gate the commit so identical-content reports are not duplicated

## 8. Spec docs
- [ ] 8.1 Add a "Relevance harness" subsection to `docs/specs/11-query-api.md` describing the `recall@10` + `MRR` metrics, the 2% regression gate, and the report shape
- [ ] 8.2 Cross-link from `docs/analysis/relevance/01-findings.md` §F-008 (mark closed-by phase6e on merge)

## 9. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 9.1 Update or create documentation covering the implementation — `docs/specs/11-query-api.md` per §8 plus the `tests/relevance/README.md` curation guide per §2
- [ ] 9.2 Write tests covering the new behavior — unit tests in `harness.rs` for the scoring math (recall/MRR boundary cases including ties + missing); integration test that runs the harness against a fixture lane with a known answer set and asserts the emitted report matches a golden snapshot
- [ ] 9.3 Run tests and confirm they pass — `cargo clippy -p cortex-relevance-eval --all-targets -- -D warnings`, `cargo test -p cortex-relevance-eval`, `cargo run -p cortex-relevance-eval -- --query-set tests/relevance/queries.yaml --api-url $CORTEX_API_URL` against a live dev stack reports a numeric baseline
