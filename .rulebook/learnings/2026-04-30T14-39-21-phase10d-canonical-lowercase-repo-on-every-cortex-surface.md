# phase10d — canonical lowercase repo on every Cortex surface
**Source**: manual
**Date**: 2026-04-30
**Related Task**: phase10d_repo_casing_normalization
**Tags**: repo, casing, scope, canonical, phase10d, metadata
The 2026-04-29 audit caught `repo` casing diverging across surfaces: `/v1/status.indexed_repos` returned lowercase, `/v1/dashboard/overview.recent_repos` returned capitalized, and `scope.repo: "Cortex"` queries silently dropped because the orchestrator lowercased the scope before matching.

Phase10d collapses every surface onto `to_ascii_lowercase`:
1. **Walker emission** — `runner.rs` runs `runner_cfg.repo_id` through `canonical_repo()` (just `to_ascii_lowercase`) before stamping `source.repo`. Original case kept as `repo_label` for diagnostics.
2. **Adapter** — `repo_from_cwd` (Claude Code adapter) lowercases the cwd basename so IPC envelopes agree with bootstrap envelopes.
3. **Lane projections** — both `meili_lane.rs` and `vectorizer_lane.rs` lowercase `LaneHit.repo` and stash the original in `extras.repo_label` when it differs. This handles the pre-phase10d corpus indexed under capitalized repos (still readable; lowercase on read).
4. **Scope filter** — already case-insensitive via `slug_for_repo` (phase10a Meili filter). No new code needed.
5. **Metadata SQLite** — new `cortex-ops repo-canonicalize [--dry-run|--apply]` rewrites `sessions.repo`, `bootstrap_jobs.repo_path`, `bootstrap_seen.repo` using `WHERE col != lower(col)`. Live-backend payload rewrites (Vectorizer/Meili/Nexus documents) deferred — those carry repo as a payload field, not a filterable column, and the lane projection already normalises on read.

Test fixture already lowercased in phase10a. Existing tests that asserted `repo_id == "Fixture"` updated to `"fixture"` since the runner now lowercases internally.

The `repo_label` on the wire (Snippet, RepoCount, etc.) is deferred — `extras.repo_label` carries it on `LaneHit`, dashboard handlers can opt-in later. Wire shape stays canonical.