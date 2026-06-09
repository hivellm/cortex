## 1. Bounded live-write backfill
- [ ] 1.1 Add `--apply` + `--limit <N>` to `cortex-ops graph backfill`: project archived envelopes through the real `GraphWriter` (not count-only) for at most `N` events newer than `--since`.
- [ ] 1.2 Run the payload-driven backfill (`--apply`) against the live graph; confirm SUPERSEDES / CONTRADICTS / EMITTED_BY / ANSWERED_BY / CITES edges land via `doctor-graph-coverage`.

## 2. Classifier-replay for classifier-driven kinds
- [ ] 2.1 Re-enrich a bounded window of archived envelopes through the live classifier so CALLS / IMPORTS / DEFINES / RETURNS / ABOUT / MENTIONS_FILE / RELATES_TO carry real relations/entities/topics.
- [ ] 2.2 Project the enriched window live; gate the window size so sustained edge-writes stay under the nexus#12 stall threshold.

## 3. Acceptance
- [ ] 3.1 `cortex-ops doctor-graph-coverage` reports all 12 kinds present and above the §4.2 floor; capture the JSON output as the acceptance artifact.

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation
- [ ] 4.2 Write tests covering the new behavior
- [ ] 4.3 Run tests and confirm they pass
