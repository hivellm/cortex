## 1. Trigger producer
- [ ] 1.1 Define the trigger-producer trait + envelope shape (session-end, topic-threshold, decision-landed) reading the existing grain rules
- [ ] 1.2 Implement session-end detection (idle window or explicit session-end envelope) and publish a session-grain trigger
- [ ] 1.3 Implement topic-threshold detection via the existing topic_cards::trigger evaluator and publish a topic-grain trigger
- [ ] 1.4 Implement decision-landed detection on Kind::Decision and publish a decision-trace trigger

## 2. Stack wiring
- [ ] 2.1 Host the producer in classifier-worker (already consumes the event stream); guard with a config flag
- [ ] 2.2 Confirm the daemon dispatches (dispatched>0) end-to-end against the live Synap stream

## 3. Backfill
- [ ] 3.1 Run `cortex-consolidator estimate` to preview spend before any live run
- [ ] 3.2 Backfill consolidations from existing history (run-session / run-topic / nightly) so the empty indexes populate

## 4. Verify retrieval surfaces
- [ ] 4.1 `cortex_similar_sessions` returns hits for a known prior session
- [ ] 4.2 `cortex_topic_search` returns at least one topic card
- [ ] 4.3 `cortex_consolidations_recent` returns the newest consolidation envelopes

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 5.1 Update docs/specs/27-consolidation.md with the live producer wiring
- [ ] 5.2 Write tests covering the trigger producer (one per grain condition)
- [ ] 5.3 Run tests and confirm they pass
