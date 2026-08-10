# Proposal: phase29d_synap-wipe-equality-gap

## Why

The phase29c stale-ahead-offset self-heal fires on `cursor > head_next`.
Found live (2026-08-05, second synap-restart battery round): when the
pre-wipe cursor happens to EQUAL the post-wipe head, the condition never
fires and the consumer silently skips the first post-wipe event(s).

Observed sequence (classifier, `cortex.events.raw`):

1. Room has 1 event (offset 0); classifier consumes it → in-memory cursor 1.
2. `docker restart cortex-synap` → room wiped.
3. One new event arrives (K-BATTERY-3) → recreated room: offset 0, head_next 1.
4. Classifier polls `from_offset=1` → empty. Stale check: `1 > 1` = false → no heal.
5. Offset 0 is never delivered to this consumer. Later events (offset 1+)
   flow normally — the loss is bounded to the events published before the
   cursor value is exceeded, NOT a permanent starvation.

Why stats alone cannot solve it: `stream.stats` exposes no room
identity/generation — `total_published` resets on wipe, so "wiped and
refilled to N" is indistinguishable from "healthy caught-up at N"
whenever the counts coincide. `total_consumed` heuristics break on
shared rooms (`cortex.events.enriched` has three consumers).

Evidence: live raw `{message_count:1, total_consumed:0}` while the
healed-code classifier idled silently >12 min (no heal warn, no error) —
cursor arithmetically pinned at 1 == head_next. Recovery required a
worker restart (fresh cursor 0). phase29c's `>` heal remains correct and
live-proven for the common case (graph healed 56789→0 in one poll).

## What Changes

1. Upstream (hivellm/synap): request a room-generation discriminator in
   `stream.stats` (`created_at` epoch or monotonic `generation`).
2. Cortex interim: published-decrease detection in all four
   `LiveSynapConsumer`s (remembered `total_published` dropping ⇒ wipe ⇒
   cursor reset), closing every case except the exact-equal coincidence.
3. When the upstream field ships: replace both heuristics with the
   definitive generation comparison.

## Impact

- Affected specs: docs/specs/05-classifier.md (consumer resilience notes),
  docs/specs/03-local-stack.md (synap pin when the upstream release lands)
- Affected code: crates/cortex-workers/src/{classifier_worker,fulltext,embedder,graph}/worker.rs,
  crates/cortex-workers/tests/synap_room_selfheal_it.rs
- Breaking change: NO
- User benefit: no silently-skipped events after a synap restart; the
  self-heal becomes wipe-complete instead of wipe-mostly.
