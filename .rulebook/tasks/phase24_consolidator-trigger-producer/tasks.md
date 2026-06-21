## 1. Trigger producer
- [x] 1.1 Define the trigger envelope builder (`consolidator/trigger_producer.rs`); reuse the daemon's `TRIGGER_STREAM` + the classifier worker's existing `SynapPublisher` instead of a new trait
- [x] 1.2 Implement session-end detection (idle window or explicit session-end envelope) and publish a session-grain trigger — idle-window detector: the Claude `Stop` hook lands as a plain `Kind::Turn` (no session-end envelope), so `trigger_producer::evaluate_idle_sessions` tracks last-seen ms per session and emits a `session_end` trigger for any session quiet past `SESSION_IDLE_MS` (30 min). Wired into the classifier worker (shared `Mutex<BTreeMap>`, lock released before publish) behind the existing `consolidator_trigger_enabled` opt-in flag. 5 unit tests (build, blank-reject, fire-once+prune, active-never-flagged, exact-boundary).
- [~] 1.3 Detection builder DONE: `trigger_producer::topic_threshold_trigger(repo, card, kind, dist_fn, age)` wraps `topic_cards::trigger::Trigger::evaluate` and emits a `nightly_topic` (`{kind, repo}`, parsed by `cortex-consolidator::parse_trigger`) on `Rewrite`, nothing on `Hold`/blank-repo. 4 unit tests (first-emit, event-count, hold, blank-repo); clippy clean. REMAINING (live publish wiring): the classifier worker (where §1.2/§1.4 publish) holds only a SQLite `MetadataStore` — topic cards live in Meili, so the worker can't read the current card to compute `events_since`/`age` for the evaluator. Wiring the publish needs a topic-card-lookup handle in the producer's host (or running this detection where cards are readable) — a distinct plumbing item, NOT the detection logic.
- [x] 1.4 Implement decision-landed detection on Kind::Decision and publish a decision-trace trigger

## 2. Stack wiring
- [x] 2.1 Host the producer in classifier-worker (already consumes the event stream); guard with config flag `CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED` (default off)
- [ ] 2.2 Confirm the daemon dispatches (dispatched>0) end-to-end against the live Synap stream (requires enabling the flag — opt-in Opus spend)

## 3. Backfill
- [x] 3.1 DONE: `cortex-consolidator estimate --repo cortex` → 6083 envelopes; session+topic grains ~$0 (Haiku, realistically a few $ on 3.4M input tok), decision_trace **$12.18** (Opus 4.7; assumes 100 ADRs, actual 27 → ~$3). Worst-case ~$12, concentrated in the Opus decision-trace grain.
- [~] 3.2 Pipeline VALIDATED via `nightly --dry-run true --all`: 216 candidate sessions enumerated, batch plan built, monthly cap $1000, no API call — the backfill path is correctly wired end-to-end. REAL run BLOCKED on two operator decisions: (a) a valid `ANTHROPIC_API_KEY` (the container's is 29 chars; a real key is ~108 `sk-ant-…` — likely a placeholder, so a live run would 401) and (b) spend authorization (~$12 worst-case per §3.1). No money spent.

## 4. Verify retrieval surfaces
- [ ] 4.1 `cortex_similar_sessions` returns hits for a known prior session
- [ ] 4.2 `cortex_topic_search` returns at least one topic card
- [ ] 4.3 `cortex_consolidations_recent` returns the newest consolidation envelopes

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update docs/specs/27-consolidation.md with the live producer wiring
- [ ] 5.2 Write tests covering the trigger producer (one per grain condition)
- [ ] 5.3 Run tests and confirm they pass

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation
- [ ] 6.2 Write tests covering the new behavior
- [ ] 6.3 Run tests and confirm they pass
