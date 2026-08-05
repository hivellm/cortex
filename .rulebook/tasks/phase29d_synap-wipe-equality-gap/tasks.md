## 1. Implementation
- [x] 1.1 Upstream: file a hivellm/synap issue requesting a room-generation discriminator in `stream.stats` (`created_at` epoch or a monotonic `generation` id) — today's payload (name, message_count, max_offset, min_offset, total_published, total_consumed, subscriber_count, dropped) cannot distinguish "wiped room refilled to N events" from "healthy caught-up room with N events". Filed: https://github.com/hivellm/synap/issues/257
- [ ] 1.2 Cortex-side interim mitigation (until the upstream field ships): extend the phase29c stale-ahead heal in all four `LiveSynapConsumer`s with published-decrease detection — remember the last-seen `total_published` per room across polls; when a fresh stats probe reports `total_published` LOWER than remembered, the room was wiped → reset cursor to 0 (and force-rewind the graph ledger). This closes every wipe case except the exact-equal-published coincidence, which shrinks over time as rooms accumulate events.
- [ ] 1.3 When the synap release with the generation/created_at field lands: replace the heuristics (offset>head_next + published-decrease) with the definitive generation comparison, and drop the interim bookkeeping.

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
