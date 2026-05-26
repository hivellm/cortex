## 1. Watcher background task
- [x] 1.1 NEW `crates/cortex-api/src/silent_drop.rs` (kept at the lib root rather than `src/health/silent_drop.rs` because the watcher owns its own data model — `SilentDropConfig`, `PairState`, `AlertState`, `AlertSeverity`, `AlertTransition`, `SilentDropWatcher` — and bundling under `health::` would obscure the boundary)
- [x] 1.2 Defined `SilentDropWatcher { states: HashMap<String, PairState>, cfg: SilentDropConfig, aggregator: Arc<HealthAggregatorState>, last_poll: Option<Instant> }`. The reqwest client is constructed inside the spawn closure so the watcher state stays serializable for tests
- [x] 1.3 Implemented `tick(client, lane)` async fn — gathers divergence rows via the shared `health::gather_subsystem_extras` + `build_divergence_pairs`, runs them through `step()`, and emits envelopes for each transition
- [x] 1.4 Spawn from `crates/cortex-api/src/main.rs` as a `tokio::spawn` task with `tokio::time::interval(poll_interval_secs)` after the dashboard router is built; first tick is intentionally delayed one cadence so subsystems get to bind their `/healthz` first

## 2. Alert state machine
- [x] 2.1 NEW state machine code lives in the same `silent_drop.rs` module — folding it into the watcher's data model keeps the surface tight and avoids a second `mod alerts` for what is fundamentally one type
- [x] 2.2 Defined `enum AlertState { Ok, Warn { consecutive: u8 }, Critical }` plus `AlertSeverity` for envelope-stamping and `PairState { alert, recovery_streak }` so the recovery debounce stays serializable
- [x] 2.3 Implemented pure-function `transition(prev, delta_growth, cfg) -> (PairState, AlertTransition)` driving the entire decision
- [x] 2.4 Debouncing matches the spec verbatim: `Ok → Warn` requires `consecutive == 2` over `warn_delta`; `Warn → Critical` fires on a single poll exceeding `critical_delta`; recovery requires `recovery_streak == 5` consecutive non-growing polls
- [x] 2.5 Unit tests cover all 9 transition paths: `ok_to_warn_requires_two_consecutive_polls`, `transient_spike_does_not_alert`, `critical_fires_on_single_poll_above_threshold`, `warn_to_critical_promotes_after_threshold_breach`, `recovery_requires_five_consecutive_ok_polls`, `already_critical_stays_silent_on_subsequent_critical_polls`, plus state save/load round-trip and `step_emits_one_envelope_per_transition` integration check

## 3. Alert envelope emission
- [x] 3.1 `build_alert_envelope(row, severity)` produces a canonical `kind: "law_violation"` envelope with payload `{ law_id: "silent-drop:<pair>", severity, upstream_count, downstream_count, delta, delta_growth, since, message }` — shape verified by `build_alert_envelope_carries_canonical_law_violation_kind`
- [x] 3.2 `post_envelope_to_ingestion(client, url, envelope)` POSTs to `<ingestion_url>/v1/events/batch`; transport failures log at WARN and never propagate
- [x] 3.3 `alert_lane_hit(row, severity, ts_ms)` constructs a `LaneHit` mirroring the meili_loader's `law_violation` shape; the watcher's `tick()` seeds it into the `cortex-code` / `cortex-docs` / `cortex-decisions` indexes immediately so Live Timeline reflects the alert without waiting for the archive_loader refresh
- [x] 3.4 `save_pair_state` / `load_pair_state` persist `PairState` JSON to `~/.cortex/alerts/<sanitised-pair>.json` after every tick; `hydrate_from_disk()` re-reads on boot so a restart does not re-fire alerts

## 4. Configuration
- [x] 4.1 `SilentDropConfig` carries `enabled`, `poll_interval_secs`, `pairs: Vec<PairConfig>`, `webhook_url`, `write_to_handoff`, `state_dir`, `ingestion_url`. Serde-tagged so a future cortex.toml `[silent_drop]` section round-trips trivially
- [x] 4.2 `SilentDropConfig::default()` resolves `~/.cortex/alerts` from `HOME`/`USERPROFILE` and `ingestion_url` from `CORTEX_INGESTION_URL` (falling back to `http://127.0.0.1:17010`); the cortex-api boot path uses this default unless an operator overrides
- [x] 4.3 Default pair list — when a pair name appears in the divergence rows but isn't explicitly listed in `cfg.pairs`, `pair_config(cfg, pair)` returns `PairConfig::for_pair(pair)` with `warn_delta=10` / `critical_delta=50` (matching phase8b's severity bucketing). Covers `adapter.frames_parsed → adapter.envelopes_built`, `adapter.envelopes_built → adapter.envelopes_publish_ok`, and `adapter.publish_ok.<kind> → ingestion.archived.<kind>` automatically

## 5. Escalation hooks
- [x] 5.1 `post_to_webhook(client, url, envelope)` POSTs the envelope to `cfg.webhook_url` when set; transport failures log + continue
- [x] 5.2 `append_handoff(workspace, row, severity)` writes a single `[silent-drop alert]`-prefixed line to `.rulebook/handoff/_pending.md` when `cfg.write_to_handoff && severity == Critical`; best-effort I/O

## 6. CLI: cortex-ops doctor-alerts
- [x] 6.1 NEW `cortex-ops doctor-alerts` subcommand (no separate cortex-doctor crate needed because cortex-cli already hosts the operator CLIs). Walks the alerts directory for `<pair>.json` files
- [x] 6.2 Reads each `~/.cortex/alerts/*.json` file, prints `ok | WARN | CRITICAL <pair> (recovery_streak=N)` rows. The `--state-dir` override lets tests + CI runs point at fixtures, and `--json` returns `{ state_dir, any_critical, alerts: [...] }` for machine-readable output. NEW `scripts/doctor-alerts.{bat,sh}` thin wrappers around the subcommand
- [x] 6.3 Exit `0` when no Critical alerts are active, `2` when any are. Suitable for CI gates and operator quick-checks

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation — `docs/architecture.md §13.9 Observability — silent-drop detector (phase8e)` (watcher mechanics + debouncing rules + escalation hooks + which incident closed) + CHANGELOG entry under `### Added → Observability — silent-drop detector (phase8e)` listing the new module + state-machine + push-channel + escalation + persistence behaviours. No standalone runbook file because the lane / dashboard surfaces alerts directly and the architecture section already reads as a runbook
- [x] 7.2 Write tests covering the new behavior — 14 unit tests in `crates/cortex-api/src/silent_drop.rs` covering every state-machine transition (ok→warn debounce, transient-spike no-fire, single-poll critical, warn→critical promotion, 5-poll recovery, already-critical silence), config defaults, filename sanitisation, save/load round-trip, default-load on missing file, envelope shape, watcher step emits one envelope per transition, watcher persists state, hydrate_from_disk pre-populates state map
- [x] 7.3 Run tests and confirm they pass — `cargo test --workspace` reports 0 failures across cortex-api (lib + integration), cortex-cli (which now wires the doctor-alerts subcommand), and every other crate
