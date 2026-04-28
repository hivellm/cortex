## 1. Watcher background task
- [ ] 1.1 NEW `crates/cortex-api/src/health/silent_drop.rs`
- [ ] 1.2 Define `SilentDropWatcher` struct with `pairs: Vec<PairConfig>`, `state: HashMap<String, AlertState>`, `client: reqwest::Client`
- [ ] 1.3 Implement `tick()` async fn that polls divergence endpoint and updates state per pair
- [ ] 1.4 Spawn from `cortex-api/src/main.rs` as a `tokio::spawn` task with `tokio::time::interval(poll_interval)`

## 2. Alert state machine
- [ ] 2.1 NEW `crates/cortex-api/src/health/alerts.rs`
- [ ] 2.2 Define `enum AlertState { Ok, Warn { since: Instant, consecutive: u8 }, Critical { since: Instant } }`
- [ ] 2.3 Implement `transition(prev: &AlertState, sample: DivergenceSample, cfg: &PairConfig) -> AlertTransition`
- [ ] 2.4 Implement debouncing: ok→warn requires 2 consecutive polls; warn→critical requires 1 poll exceeding critical threshold; recovery requires 5 consecutive ok polls
- [ ] 2.5 Unit tests covering all 9 transition combinations

## 3. Alert envelope emission
- [ ] 3.1 On each transition, build `Envelope { kind: "law_violation", payload: { law_id: "silent-drop-<pair>", severity, upstream_count, downstream_count, delta_growth_60s, ... } }`
- [ ] 3.2 POST to cortex-ingestion `/v1/events/batch` so the envelope lands in archive + lane like any other event
- [ ] 3.3 Insert into the cortex-api `MemoryKeywordLane` immediately so Live Timeline reflects it without waiting for archive_loader refresh
- [ ] 3.4 Persist last alert state per pair to `~/.cortex/alerts/<pair>.json` for restart-safe dedup

## 4. Configuration
- [ ] 4.1 Define `cortex.toml` schema with `[silent_drop]` section: `enabled`, `poll_interval_secs`, `pairs: [...]`, `webhook_url`, `write_to_handoff`
- [ ] 4.2 Reader in `cortex-api/src/config.rs` returns sane defaults if section missing
- [ ] 4.3 Default pair list shipped in code: ipc→dispatcher, dispatcher→publisher, publisher→ingestion, ingestion→archive_loader

## 5. Escalation hooks
- [ ] 5.1 Optional webhook POST on transitions: payload includes alert envelope + current divergence row
- [ ] 5.2 Optional `.rulebook/handoff/_pending.md` append on critical transitions; line prefixed with `[silent-drop alert]`

## 6. CLI: cortex doctor alerts
- [ ] 6.1 Reuse `cortex-doctor` from phase8d; add `alerts` subcommand
- [ ] 6.2 Reads `~/.cortex/alerts/*.json` and prints active alerts + state + age
- [ ] 6.3 Exit 0 if no critical alerts, 2 if any

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Document the silent-drop architecture in `docs/architecture.md` and add a runbook in `docs/runbooks/silent-drop.md`; CHANGELOG entries on cortex-api + cortex-doctor
- [ ] 7.2 Tests: state machine transitions (unit); watcher integration test driving fake divergence endpoint; envelope emission test asserting the envelope shape and that it lands in the lane
- [ ] 7.3 Run `cargo test -p cortex-api -p cortex-doctor` and confirm all pass
