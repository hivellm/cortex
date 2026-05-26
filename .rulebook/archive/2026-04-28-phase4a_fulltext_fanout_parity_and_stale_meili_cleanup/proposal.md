# Proposal: phase4a_fulltext_fanout_parity_and_stale_meili_cleanup

## Why

Empirical audit on 2026-04-27 22:36 UTC against the live stack
exposed two structural inconsistencies between the indexing
backends:

1. **Meilisearch is missing entire repos that the Vectorizer already
   has.** The fulltext store has only the `Cortex` repo indexed,
   while the Vectorizer has `Cortex`, `Rulebook`, and `Vectorizer` —
   all fed by the same enriched-event stream.

   ```
   backend       cortex   rulebook   vectorizer
   vectorizer    17,629   9,264      101,293     ← vectors
   meilisearch     589        0           0      ← docs
   ```

   The fulltext index naming scheme is correct in code
   ([crates/cortex-fulltext/src/routing.rs:105-107](../../../crates/cortex-fulltext/src/routing.rs#L105-L107))
   — `cortex-{repo}-{family}` — and `index_for_event` reads
   `event.context_repo` ([routing.rs:125-137](../../../crates/cortex-fulltext/src/routing.rs#L125-L137)).
   So the routing is not the bug; the worker either never received
   non-Cortex events or stopped before catching up. Diagnostic
   step inside this task pins the root cause before fixing.

2. **Six stale Meili indexes from an older naming scheme.** The
   following indexes exist, are empty, but were never dropped
   after the prefix migration to `cortex-{repo}-{family}`:

   ```
   cortex-code, cortex-decisions, cortex-docs,
   cortex-governance, cortex-misc, cortex-turns
   ```

   They pollute `/indexes` listings, corrupt index-count metrics,
   and risk being targeted by old client code that still hardcodes
   the un-slugged names.

A keyword lane that only sees one of three indexed repos is, in
practice, a single-repo lane — pre-thinking bundles for any prompt
about Rulebook or Vectorizer fall back to the vector lane only,
and the BM25-as-embedding scores there are weak (top-1 score 0.136
on the audit's "classifier worker" probe).

## What Changes

- A short diagnostic phase inside the task identifies why the
  worker never indexed non-Cortex events (consumer offset stale?
  stream filter? worker crashed before replay?). Findings recorded
  in `.rulebook/learnings/` before any code change.
- `cortex-fulltext` worker gains a **replay-missing-repos** path:
  on startup, list Meili indexes, compute the set of `(repo,
  family)` partitions present in the event store but absent from
  Meili, and replay those events from the archive (idempotent —
  Meili upserts by `id`, which is `content_hash`-derived).
- `cortex-fulltext` boot sequence runs a **stale-index sweep**:
  for every Meili index whose name does not match
  `^cortex-{repo}-{family}$` (where `{family}` is one of
  `code|docs|decisions|turns|governance|misc`), drop it if and
  only if it is empty. Non-empty mis-named indexes are warned
  about, not deleted, to protect against typos in the regex.
- Defensive guard at construction: `routing::index_name` returns
  a name with exactly three hyphen-separated tokens. Add a
  `debug_assert!` + integration test asserting the invariant on
  every routing call.
- Drop the six known stale indexes
  (`cortex-{code,decisions,docs,governance,misc,turns}`) as a
  one-shot migration on first boot (idempotent via `404 Not
  Found` acceptance).

## Impact

- Affected specs: spec-08 (fulltext indexer — adds
  replay-missing and stale-sweep on boot).
- Affected code:
  - `crates/cortex-fulltext/src/worker.rs` — replay-missing path
  - `crates/cortex-fulltext/src/main.rs` (or boot module) —
    stale-sweep
  - `crates/cortex-fulltext/src/meili_client.rs` — `list_indexes`
    + `delete_index` helpers if not already present
  - `crates/cortex-fulltext/src/routing.rs` — invariant guard
  - tests: a `wiremock`-or-Meili integration test seeded with
    cortex-only data, then asserts the worker replays the
    rulebook partition when a rulebook event lands in the archive
- Breaking change: NO. Operation is idempotent; existing
  `cortex-cortex-*` indexes are untouched.
- User benefit: keyword search gains 80k+ documents of coverage
  (rulebook + vectorizer), bringing parity with the Vectorizer.

## Source

- Audit data captured 2026-04-27 22:36 UTC against running stack
  (cortex-vectorizer healthy 7h, cortex-meilisearch healthy 2d).
- Vectorizer collections probed via `/auth/login` + `/collections`.
- Meili indexes probed via `/indexes` and `/stats` with master
  key.
- Bootstrap state confirmed at
  [.cortex-bootstrap.state.json](../../../.cortex-bootstrap.state.json).
