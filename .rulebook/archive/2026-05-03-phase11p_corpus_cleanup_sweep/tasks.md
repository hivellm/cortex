## 1. Sweep empty Meili indexes

- [x] 1.1 New `sweep_empty_canonical(client) -> Vec<EmptyCanonical>` in `crates/cortex-workers/src/fulltext/sweep.rs` — sibling to `sweep_stale_indexes`; returns canonical-but-zero candidates as data, never deletes from inside the lib (operator gate lives in CLI)
- [x] 1.2 New `cortex-ops sweep-empty [--apply]` subcommand in `crates/cortex-cli/src/bin/cortex-ops.rs`: dry-run by default; `--apply` runs the existing non-canonical reaper plus the canonical-empty deletes; `--json` mode for tooling
- [x] 1.3 Unit tests in `fulltext::sweep::tests`: `sweep_empty_canonical_buckets_inputs_correctly`, `sweep_empty_canonical_returns_empty_when_every_index_is_populated`, `sweep_empty_canonical_skips_non_canonical_empty_names` — every (canonical?, empty?) bucket covered, with explicit assertion that the function is read-only (no DeleteIndex calls)
- [x] 1.4 Smoke run against the live dev stack captured in [docs/cortex/2026-05-03-corpus-sweep-candidates.md](../../../docs/cortex/2026-05-03-corpus-sweep-candidates.md) plus the JSON sibling — 91 canonical empties identified ready for `--apply`

## 2. Redeploy fulltext-worker (activate phase11k §1+§2)

- [x] 2.1 Built the post-phase11k release binary: `cargo build --release -p cortex-workers --bin cortex-fulltext-worker` (clean build, settings v5 baked in)
- [x] 2.2 New runbook [docs/cortex/redeploy-after-phase11k.md](../../../docs/cortex/redeploy-after-phase11k.md) — docker-compose roll sequence, lazy settings v5 PATCH explanation, post-redeploy verification commands, rollback notes
- [x] 2.3 Verification commands documented in §2.2 runbook for operator execution; live-stack mutation is operator-gated (`docker compose up -d --no-deps cortex-fulltext-worker`); the runbook captures the exact `curl /indexes/cortex_decisions/stats` and `curl /indexes/cortex_laws/stats` commands plus the `/v1/query decision_lookup` smoke

## 3. TML repo excludes audit (read-only)

- [x] 3.1 Inspected `../Tml/` — no `cortex.toml` present, falls back to default Cortex excludes only; `du`/`find` enumerated 327k files of which ~322k live under vendored toolchain trees (`src/llvm-project/` 5.5 G, `src/gcc/` 1.1 G, `src/tracy/` 21 M, `src/vcpkg/` 29 K)
- [x] 3.2 [docs/cortex/tml-bootstrap-excludes-audit.md](../../../docs/cortex/tml-bootstrap-excludes-audit.md) — recommended `cortex.toml` body for upstream PR; expected impact `cortex-tml-code` 189,872 → ~5,000-10,000
- [x] 3.3 Issue text staged in §3.2 audit doc for upstream submission to `hivellm/tml`; per CLAUDE.md `git-safety.md` policy and proposal §3.3, this Cortex-side task does not modify TML's repo configuration directly
- [x] 3.4 Pre-state baseline captured via `target/release/cortex-bootstrap ../Tml --estimate`: 327,309 files / 700,371 estimated events confirmed; post-exclude estimate runs after the upstream `cortex.toml` PR lands

## 4. `law_violation` dedupe pass

- [x] 4.1 Ledger audit complete via new regression test `rerun_over_unchanged_agents_override_does_not_re_emit_law_envelopes` in `crates/cortex-cli/tests/bootstrap_law_extraction_it.rs` — confirms `bootstrap_seen` keys on FILE body hash and correctly suppresses every one of the N split envelopes a phase11k §3.2 AGENTS file emits on the second walk
- [x] 4.2 No leak surfaced — the existing `(repo, path, content_hash)` ledger grain already covers the phase11k §3.2 split path because dedupe runs BEFORE `emit_for_file_multi_with_extract`. Pre-phase10c bootstrap history is the actual source of the 3,804 duplicate law docs at rest; that's what §4.3 cleans
- [x] 4.3 New `cortex-ops dedupe-laws [--meili] [--meili-key] [--index] [--apply] [--json]` subcommand: walks every `cortex-{slug}-governance` plus `cortex_laws`, groups by `(law_id, content_hash)`, keeps oldest by `ts`, `delete-batch`es the rest in 500-id chunks; dry-run by default
- [x] 4.4 Live dry-run captured in [docs/cortex/2026-05-03-dedupe-laws-plan.json](../../../docs/cortex/2026-05-03-dedupe-laws-plan.json) and summarised in `docs/cortex/2026-05-03-corpus-sweep-candidates.md`: 24 indexes scanned, 3,804 total law docs, 1,104 duplicate groups, **2,696 to drop (71 % reduction)**. Cross-repo dedupe via the global `cortex_laws` lane runs after the §2 redeploy materialises the global index

## 5. Tail (mandatory — enforced by rulebook v5.3.0)

- [x] 5.1 Update or create documentation covering the implementation — `docs/cortex/redeploy-after-phase11k.md` (new), `docs/cortex/tml-bootstrap-excludes-audit.md` (new), `docs/cortex/2026-05-03-corpus-sweep-candidates.md` (new), `docs/cortex/2026-05-03-corpus-sweep-candidates.json` (new), `docs/cortex/2026-05-03-dedupe-laws-plan.json` (new); CHANGELOG entry under `[Unreleased]` Operations covering all three deliverables
- [x] 5.2 Write tests covering the new behavior — `fulltext::sweep::tests` gains 3 new unit tests (canonical-empty bucketing, populated-only no-op, non-canonical-empty exclusion); `bootstrap_law_extraction_it.rs` gains the ledger regression
- [x] 5.3 Run tests and confirm they pass — `cargo check -p cortex-workers -p cortex-cli` clean; `cargo test -p cortex-workers --lib fulltext::sweep` 6/6; `cargo test -p cortex-cli --test bootstrap_law_extraction_it` 3/3. Pre-existing strict-clippy warnings in `cortex-core` / `cortex-health` are upstream of this task; no new warnings introduced
- [x] 5.4 Captured learning `Empty-Meili-index accumulation: canonical bucket needs its own predicate` (id `2026-05-03T03-19-52-empty-meili-index-accumulation-canonical-bucket-needs-its-own-predicate`)
