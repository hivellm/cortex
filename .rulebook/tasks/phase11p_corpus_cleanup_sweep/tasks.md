## 1. Sweep empty Meili indexes

- [ ] 1.1 Extend `crates/cortex-workers/src/fulltext/sweep.rs::sweep_empty_non_canonical` with a sibling `sweep_empty_canonical(client) -> Vec<String>` that lists canonical `cortex-{slug}-{family}` indexes whose `numberOfDocuments == 0`; do not delete from inside the lib — return the list so the caller decides
- [ ] 1.2 New `cortex-ops sweep-empty [--apply]` subcommand in `crates/cortex-cli/src/bin/cortex-ops.rs`: lists candidates from §1.1 plus the existing non-canonical sweep; default is dry-run that prints `would drop: <uid>` per candidate; `--apply` calls `client.delete_index` for each
- [ ] 1.3 Unit test for §1.1 against a `MemoryMeiliClient` seeded with a mix of (canonical, populated), (canonical, empty), (non-canonical, populated), (non-canonical, empty); assert each lands in the right bucket
- [ ] 1.4 Integration smoke for §1.2: run `cortex-ops sweep-empty` against the dev stack; capture the candidate list into `docs/cortex/2026-05-03-corpus-sweep-candidates.md` for operator review before `--apply`

## 2. Redeploy fulltext-worker (activate phase11k §1+§2)

- [ ] 2.1 Build the post-phase11k binary: `cargo build --release -p cortex-workers --bin cortex-fulltext-worker`
- [ ] 2.2 New runbook `docs/cortex/redeploy-after-phase11k.md` documenting the docker-compose restart sequence + the rolling settings v5 PATCH the new build pushes on first contact with each per-repo index
- [ ] 2.3 Verification: post-redeploy, `curl /indexes/cortex_decisions/stats` and `curl /indexes/cortex_laws/stats` return non-zero `numberOfDocuments` after the next governance bootstrap; capture the before/after numbers in the runbook

## 3. TML repo excludes audit (read-only)

- [ ] 3.1 Read `../Tml/cortex.toml` and `../TmlDocs/cortex.toml`; cross-reference `cortex-tml-code` (189,872 docs) against the path distribution under `crates/cortex-cli/src/bootstrap/walker.rs::walk_repo` to identify the top 10 directories contributing the most files
- [ ] 3.2 Produce `docs/cortex/tml-bootstrap-excludes-audit.md` with the recommended `[cortex.exclude].paths` additions (likely `target/`, `dist/`, `build/`, `node_modules/`, `vendor/`, `generated/`, plus any `.generated.{ts,go,rs}` files surfaced)
- [ ] 3.3 Open a tracking issue in the upstream `hivellm/tml` repository linking to the audit doc; do NOT modify TML's `cortex.toml` from this repo
- [ ] 3.4 Sanity check: re-bootstrap one Hive sibling repo with the proposed excludes via `cortex-bootstrap --estimate-only` to confirm the doc count drops by ≥ 80 % without losing real source files

## 4. `law_violation` dedupe pass

- [ ] 4.1 Audit `crates/cortex-cli/src/bootstrap/runner.rs::run_repo_with_dedup` to confirm whether the `bootstrap_seen` ledger trips on AGENTS / spec re-emits when the body is unchanged; write a regression test that re-runs the bootstrap twice over the same fixture and asserts the second run's `events_published` is 0 for the AGENTS path
- [ ] 4.2 If §4.1 surfaces a leak (the phase11k §3.2 split path emits one envelope per `## LAW-` heading; each envelope's content_hash is independent — confirm the ledger keys on the per-envelope hash, not the file's hash), patch the ledger to dedupe at the per-envelope grain
- [ ] 4.3 New `cortex-ops dedupe-laws [--apply]` subcommand: walks each `cortex-{slug}-governance` index, groups documents by `(law_id, content_hash)`, and `DELETE` every duplicate keeping the oldest (`ts ASC`); dry-run prints `would dedupe: <law_id> in <index>: keep <doc_id>, drop <N> siblings`
- [ ] 4.4 Acceptance: after `--apply`, the dashboard `kind_breakdown` counter for `law_violation` drops from 3,804 to under 500; capture the before/after numbers in `docs/cortex/2026-05-03-corpus-sweep-candidates.md`

## 5. Tail (mandatory — enforced by rulebook v5.3.0)

- [ ] 5.1 Update or create documentation covering the implementation — `docs/cortex/redeploy-after-phase11k.md` (new), `docs/cortex/tml-bootstrap-excludes-audit.md` (new), `docs/cortex/2026-05-03-corpus-sweep-candidates.md` (new); CHANGELOG entry under `[Unreleased]` Operations
- [ ] 5.2 Write tests covering the new behavior — §1.3 unit test for sweep buckets; §4.1 regression test for ledger dedupe; coverage ≥ 95 % on `crates/cortex-workers/src/fulltext/sweep.rs`
- [ ] 5.3 Run tests and confirm they pass — `cargo check -p cortex-workers -p cortex-cli`, `cargo clippy --all-targets -- -D warnings` (touched files only), `cargo fmt --check`, `cargo test -p cortex-workers --lib fulltext::sweep`, `cargo test -p cortex-cli --tests`. All green before archive.
- [ ] 5.4 Capture learning: `rulebook_learn_capture` on the empty-Meili-index accumulation pattern (canonical-but-zero indexes survive the existing sweep because the predicate only matches NON-canonical names; canonical empties need a sibling predicate)
