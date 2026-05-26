# Empty-Meili-index accumulation: canonical bucket needs its own predicate
**Source**: manual
**Date**: 2026-05-03
**Related Task**: phase11p_corpus_cleanup_sweep
**Tags**: phase11p, spec-08, meili, ops, sweep, lib-vs-cli-boundary
The phase4a `sweep_stale_indexes` reaper was scoped to NON-canonical names (`cortex-decisions`, `cortex-code`, etc — the legacy two-token migration leftovers). It correctly auto-deletes those on every boot.

But canonical names (`cortex-{slug}-{family}`) survive the predicate even when they hold zero documents. On 2026-05-03 the live cluster carried 91 such empty-canonical indexes — typos / abandoned repos (`cortex-csharp-*`, `cortex-go-*`, `cortex-rust-*`, `cortex-tests-*`, `cortex-x-*`) plus partially-empty per-repo families on real repos.

Why the lib did not auto-delete canonical empties: empty-canonical can be a LEGITIMATE transient state right after a settings PATCH but before the first upsert lands (per-repo lazy materialisation flow in `MeiliFulltextIndexer::ensure_settings`). Auto-deleting would force the next upsert to recreate the index — wasted PATCH cycles.

Phase11p §1 split the responsibility: `sweep_empty_canonical` returns the candidate list as data; the destructive call lives in `cortex-ops sweep-empty --apply` so the operator decides. The CLI is dry-run by default; `--apply` is explicit.

Pattern: when a sweep is sometimes-correct, return candidates from the lib and let the CLI gate the destructive op. Don't bake operator policy into a library predicate.