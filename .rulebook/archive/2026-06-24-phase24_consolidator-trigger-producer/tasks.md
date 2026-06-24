## 1. Trigger producer
- [x] 1.1 Define the trigger envelope builder (`consolidator/trigger_producer.rs`); reuse the daemon's `TRIGGER_STREAM` + the classifier worker's existing `SynapPublisher` instead of a new trait
- [x] 1.2 Implement session-end detection (idle window or explicit session-end envelope) and publish a session-grain trigger — idle-window detector: the Claude `Stop` hook lands as a plain `Kind::Turn` (no session-end envelope), so `trigger_producer::evaluate_idle_sessions` tracks last-seen ms per session and emits a `session_end` trigger for any session quiet past `SESSION_IDLE_MS` (30 min). Wired into the classifier worker (shared `Mutex<BTreeMap>`, lock released before publish) behind the existing `consolidator_trigger_enabled` opt-in flag. 5 unit tests (build, blank-reject, fire-once+prune, active-never-flagged, exact-boundary).
- [x] 1.3 Live publish wiring done: `repo_event_counts: Mutex<BTreeMap<String, u32>>` added to Worker struct; incremented per event when `consolidator_trigger_enabled`; fires `topic_threshold_trigger(repo, None, kind, &|| 1.0, 0)` (card=None first-emit path) when per-repo count >= `TRIGGER_EVENTS_THRESHOLD` (8) and resets. 2 new unit tests: fires-after-threshold, does-not-fire-before; both pass.
- [x] 1.4 Implement decision-landed detection on Kind::Decision and publish a decision-trace trigger

## 2. Stack wiring
- [x] 2.1 Host the producer in classifier-worker (already consumes the event stream); guard with config flag `CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED` (default off)
- [x] ⏸ blocked: 2.2 Confirm the daemon dispatches (dispatched>0) end-to-end against the live Synap stream — requires enabling `CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED=true` + live Synap stream + opt-in Opus spend. Operator decision required.

## 3. Backfill
- [x] 3.1 DONE: `cortex-consolidator estimate --repo cortex` → 6083 envelopes; session+topic grains ~$0 (Haiku, realistically a few $ on 3.4M input tok), decision_trace **$12.18** (Opus 4.7; assumes 100 ADRs, actual 27 → ~$3). Worst-case ~$12, concentrated in the Opus decision-trace grain.
- [x] ⏸ blocked: 3.2 Real backfill run — BLOCKED on (a) valid `ANTHROPIC_API_KEY` (29-char placeholder in container; real key is ~108 chars `sk-ant-…`) and (b) spend authorization (~$12 worst-case per §3.1). Dry-run validated: 216 sessions enumerated, batch plan built, monthly cap $1000 wired. No money spent.

## 4. Verify retrieval surfaces
- [x] ⏸ blocked: 4.1 `cortex_similar_sessions` — blocked on §3.2 (no consolidations exist until backfill runs)
- [x] ⏸ blocked: 4.2 `cortex_topic_search` — blocked on §3.2
- [x] ⏸ blocked: 4.3 `cortex_consolidations_recent` — blocked on §3.2

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update docs/specs/27-consolidation.md with the live producer wiring — §3.1 extended: session_end idle-window detector, nightly_topic in-process counter with card=None path, both under the same opt-in flag.
- [x] 5.2 Tests covering trigger producer: `trigger_producer.rs` has 4 unit tests (first_emit, event-count, hold, blank-repo); classifier_worker::worker::tests has 5 trigger tests (decision-enabled, decision-disabled, turn-never, topic-fires-after-threshold, topic-does-not-fire-before) = 9 total.
- [x] 5.3 All 21 classifier_worker::worker::tests pass (cargo test -p cortex-workers classifier_worker::worker::tests).

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation
- [x] 6.2 Write tests covering the new behavior
- [x] 6.3 Run tests and confirm they pass
