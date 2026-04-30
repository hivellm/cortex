# Proposal: phase10d_repo_casing_normalization

## Why

The audit confirmed that `repo` is stored inconsistently across
surfaces:

- `/v1/status.indexed_repos` returns lowercase: `["cortex",
  "vectorizer", "nexus", …]`
- `/v1/dashboard/overview.recent_repos` returns Capitalised:
  `[{"repo": "Cortex"}, {"repo": "Nexus"}, …]`
- `/v1/dashboard/sessions[].repos` carries Capitalised
- The relevance harness's `scope.repo = "Cortex"` is dropped
  silently because the orchestrator lower-cases the scope
  before checking against `indexed_repos`. Result: 40/50
  queries from the canonical fixture are omitted.

The casing diverges because the bootstrap walker preserves the
on-disk directory name (`Cortex`) while the seeded lane snapshot
that backs `/v1/status` lowercases for "tolerant matching". The
two paths never agree, so legitimate scoped queries silently fall
out of the orchestrator.

## What Changes

1. Pick **one canonical case** for `repo` everywhere: lowercase.
   It matches `indexed_repos`, the docker-compose conventions,
   and Vectorizer/Meili/Nexus collection names that already
   lowercase the repo segment.
2. Bootstrap walker lowercases `repo` at emission time. Existing
   capitalised events in the lane stay readable; the orchestrator
   normalises both sides of the comparison.
3. `DashboardState` projection layer applies a single
   `repo.to_ascii_lowercase()` before any handler returns the
   value. The Capitalised display name moves to a separate
   `repo_label` field so the GUI can keep `Cortex` for the user
   while the wire stays canonical.
4. Relevance harness query set updated in place (`tests/relevance/
   queries.toml`) so every `scope.repo` is lowercase.
5. Backfill: a one-shot `cortex-ops repo-canonicalize` migrates
   existing rows in Vectorizer + Meili + Nexus + the metadata
   `sessions` / `bootstrap_jobs_daily` tables.

## Impact

- Affected specs: `docs/specs/02-storage-layout.md` §naming,
  `docs/specs/11-query-api.md` §scope, `docs/specs/16-dashboard.md`.
- Affected code: `crates/cortex-cli/src/bootstrap/walker.rs`,
  `crates/cortex-api/src/lanes.rs`, `crates/cortex-api/src/
  dashboard.rs`, `crates/cortex-api/src/types.rs` (add `repo_label`),
  `tests/relevance/queries.toml`.
- Breaking change: NO for new envelopes. The repo-canonicalize
  one-shot is opt-in.
- User benefit: `scope.repo: "Cortex"` and `scope.repo: "cortex"`
  both work; the relevance harness stops omitting buckets and
  surfaces the real recall floor.
