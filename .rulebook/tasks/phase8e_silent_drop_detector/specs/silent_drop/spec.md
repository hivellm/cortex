# Spec: Silent-drop detector

## ADDED Requirements

### Requirement: always-on background watcher

cortex-api MUST spawn a background task `silent_drop_watcher` at boot
that polls the internal divergence aggregator (phase8b) on a
configurable interval (default 30 seconds) and updates a per-pair
state machine.

The watcher MUST NOT block the main HTTP server or the lane refresh
tasks; failures (transport errors when polling) SHALL log at WARN
and skip the tick.

#### Scenario: watcher starts at boot
Given cortex-api is starting up
When `main()` finishes wiring the dashboard router
Then a `tokio::spawn(silent_drop_watcher::run(...))` MUST be active
     AND its first tick MUST occur within `poll_interval_secs` of boot.

### Requirement: debounced state transitions

For each tracked counter pair, the watcher MUST maintain a state
`AlertState ∈ { Ok, Warn, Critical }` and apply the following
transitions:

- `Ok → Warn`: after **2 consecutive** polls observe
  `delta_growth_60s > pair.warn_delta_60s`.
- `Warn → Critical`: a single poll observing
  `delta_growth_60s > pair.critical_delta_60s` is sufficient.
- `Critical → Warn` or `Warn → Ok`: after **5 consecutive**
  polls observe `delta_growth_60s` ≤ the corresponding threshold.

A single transient burst MUST NOT alert; this debouncing prevents
the lane from being spammed by normal traffic spikes.

#### Scenario: transient spike does not alert
Given a pair was in `Ok` state
And one poll observes `delta_growth_60s = 100`
And the next poll observes `delta_growth_60s = 0`
When the watcher processes both polls
Then no alert envelope is emitted
     AND the state remains `Ok`.

#### Scenario: sustained drop alerts on second consecutive poll
Given a pair was in `Ok` state with `warn_delta_60s = 10`
And two consecutive polls observe `delta_growth_60s = 50`
When the watcher processes the second poll
Then the state MUST transition to `Warn`
     AND exactly one alert envelope MUST be emitted.

### Requirement: alert envelope shape

Each transition MUST produce an envelope with `kind: "law_violation"`
and a payload of shape:
```
{
  "law_id": "silent-drop-<upstream>-<downstream>",
  "severity": "warn" | "critical",
  "upstream_count": <u64>,
  "downstream_count": <u64>,
  "delta": <i64>,
  "delta_growth_60s": <i64>,
  "since": "<RFC-3339>",
  "message": "<human-readable>"
}
```

The envelope MUST be POSTed to `cortex-ingestion /v1/events/batch`
(so it lands in the archive) AND inserted into the in-memory
`MemoryKeywordLane` (so Live Timeline reflects it without waiting
for the archive_loader refresh).

#### Scenario: envelope reaches the timeline
Given a state transition emits an envelope at time T
When `GET /v1/dashboard/timeline/recent` is called at T+1s
Then the envelope MUST appear in the response
     AND its `kind` MUST be `law_violation`.

### Requirement: persistent alert state

The watcher MUST persist its state to `~/.cortex/alerts/<pair>.json`
on every transition. On boot, the watcher MUST read existing state
files to avoid re-emitting alerts for issues that were already
flagged in the previous run.

#### Scenario: restart-safe dedup
Given the watcher emitted a Critical alert and persisted to disk
When cortex-api is restarted while the underlying issue persists
Then the watcher MUST NOT emit a duplicate Critical envelope for the
     same pair on its first post-boot tick
     UNTIL the state machine transitions through Warn → Critical again.

### Requirement: optional escalation hooks

When `[silent_drop].webhook_url` is configured, every transition MUST
also POST the envelope to that URL (best-effort; failures logged at
WARN, never block the watcher).

When `[silent_drop].write_to_handoff = true`, every Critical
transition MUST append a single line to
`.rulebook/handoff/_pending.md` so the next session sees the alert.

#### Scenario: handoff append on critical
Given `write_to_handoff = true`
And a pair transitions Warn → Critical
When the watcher processes the transition
Then `.rulebook/handoff/_pending.md` MUST gain one new line prefixed
     with `[silent-drop alert]` containing the pair name and severity.
