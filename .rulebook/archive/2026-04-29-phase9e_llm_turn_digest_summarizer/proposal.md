# Proposal: phase9e_llm_turn_digest_summarizer

## Why

Quantization (9a) and Parquet rollup (9b) shrink storage, but they keep
every record one-per-row. A repo with 10 000 daily turns over a year
yields 3.6 M individual `:Turn` nodes plus 3.6 M Vectorizer vectors plus
3.6 M Meili documents — most of which are noisy back-and-forth that
nobody will ever query individually.

What the user actually wants from old data is a dense weekly digest:
"on this repo, week 17/2026, here is what was decided / what bugs were
hunted / which subsystems were touched" — readable in seconds, queryable
by topic. That is the Sonnet-driven layer the user explicitly asked for
when scoping Phase 9.

## What Changes

1. NEW subcommand `cortex-retention turn-digest`.
2. Bucketization: groups turns older than `digest_after_days` (default
   30) by `(repo, ISO_year_week, top_topic)` where `top_topic` is the
   classifier's highest-confidence topic for the turn.
3. For each non-empty bucket:
   - Fetch the turn texts from the Parquet archive (or its summary if
     PII-redacted by 9d).
   - Call Sonnet via the classifier client with a "produce a 200-400
     token narrative covering decisions made, bugs found, files touched,
     subsystems impacted" prompt.
   - Persist the digest as a `:Memory { memory_type='turn_digest',
     repo, year_week, topic, body, source_event_ids[] }` node.
   - Embed the digest into `cortex.memory.fp32`.
   - Emit `cortex.events.enriched` with `kind="memory"` so the existing
     pipeline picks it up.
   - Add a Nexus edge `(:Memory)-[:SUMMARIZES]->(:Turn)` for every
     source turn.
4. After a bucket is digested, the source turns become eligible for
   demotion: the next 9a sweep (or this command with `--demote`) moves
   them straight to `cortex.cold.binary`, skipping `pq`. Their Meili
   bodies are pruned to summary-only by 9f.
5. Cost-aware: budget per run (`cortex.toml [retention.digest]
   max_usd_cents_per_run`), tracked via `classifier_spend`. When the
   budget is hit the runner stops cleanly and resumes on the next call.
6. Strictly idempotent: a bucket already digested (existing
   `:Memory{kind='turn_digest', repo, year_week, topic}`) MUST NOT be
   re-summarized. Re-running with `--rebuild` rewrites the digest
   in place.

## Impact

- Affected specs: NEW `docs/specs/19-retention.md` §"LLM turn digest",
  reference from `docs/specs/05-classifier.md`.
- Affected code: NEW `crates/cortex-retention/src/turn_digest.rs`,
  `crates/cortex-classifier/src/prompts.rs` (new
  `digest_turns_to_memory` prompt), small additions in
  `cortex-graph` for the `:SUMMARIZES` edge.
- Breaking change: NO. The `:Memory` label already exists; we add a new
  `memory_type` value.
- User benefit: turns the long tail of conversational memory into a
  small set of dense, navigable digests; cuts storage and query cost on
  Vectorizer and Meili without losing the information that matters.
