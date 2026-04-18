# 14 — Governance engine (enforcement, punishment ladder, trust score)

> **Status:** 🟡 Draft · **Owner:** Core team · **Depends on:** 13, 10

## Goal

Turn Violations (spec 13) into **action**: block tool calls at the edge, inject reminders into the next turn's system prompt, down-weight untrustworthy models in the router, and recompute per-`(model, repo)` **trust scores** nightly. Also support multi-event detectors (e.g., "10 failed tests in a row") through a materialized-view layer that the per-event detector sandbox can't express.

## Scope

**In:**
- `cortex-governance` crate: consumer of `cortex.events.law_violation` + `cortex.events.enriched`.
- **Enforcement ladder** implementation (tiers 1–4 from architecture §5.4).
- **Reminder injector** — hook that merges pending reminders into the next pre-thinking bundle (spec 12).
- **Materialized-view detectors** for cross-event laws (SQL-style over recent events).
- **Trust-score** job: nightly batch + on-demand recompute.
- API endpoints: `/v1/laws/check` (sync; used by spec 10), `/v1/governance/status`, `/v1/governance/trust`.
- Router integration: publish trust scores to Rulebook (if present) via a small feed.

**Out:**
- Law authoring / detectors (spec 13).
- Law storage / graph shape (spec 07).
- Human review UX (spec 16).
- Cross-tenant / federated governance (HivehubCloud phase).

## Inputs / Outputs

### HTTP endpoints

```
POST /v1/laws/check                 # sync, tight budget (300 ms caller)
POST /v1/governance/trust/recompute
GET  /v1/governance/status
GET  /v1/governance/trust?model=claude-sonnet&repo=Vectorizer
```

#### `POST /v1/laws/check`

Request (adapter `PreToolUse`):

```jsonc
{
  "event": { /* spec 01 envelope, kind=tool_call.requested */ },
  "session": { "session_id": "...", "authorizations": [ ... ] }
}
```

Response:

```jsonc
{
  "violations": [
    { "law_id": "LAW-007", "severity": "critical",
      "message": "LAW-007: --no-verify is not authorized.",
      "tier": 3, "block": true, "evidence": { ... } }
  ]
}
```

Blocking caller uses `block: true`; additional laws with `block: false` but non-zero `tier` are captured for async handling.

#### `GET /v1/governance/trust?model=X&repo=Y`

```jsonc
{
  "model": "claude-sonnet", "repo": "Vectorizer",
  "score": 0.82,                              // 0..1
  "window_days": 30,
  "violations_by_severity": { "info": 12, "notable": 3, "critical": 0 },
  "decisions_followed": 47, "decisions_contradicted": 2,
  "last_computed_at": "2026-04-17T04:00:00Z",
  "trend": "+0.04"
}
```

### Trust-score inputs

For each `(model, repo)`:

| Signal                                    | Weight | Source                                           |
|-------------------------------------------|--------|--------------------------------------------------|
| Violations (severity-weighted)             | 0.45    | `LawViolation` count in window                   |
| Decision adherence                         | 0.35    | Turns citing vs contradicting existing Decisions |
| Task success rate                          | 0.15    | `Turn.outcome` labeled success/failure            |
| Human overrides against the model          | 0.05    | User explicitly authorized a blocked action      |

Defaults; weights configurable per deployment.

### Pending reminder format

Stored in Synap KV under `governance:reminders:<session_id>`:

```jsonc
[
  { "law_id": "LAW-012", "message": "Benchmark HNSW recall before merging.",
    "tier": 2, "emitted_by": "violation:LV-01HY...", "expires_at": 1713390000000 }
]
```

Spec 12 reads this list when assembling the pre-thinking bundle.

## Design

### Enforcement ladder

Mapping **(severity → default tier)**, overridable per-law via `enforcement_tier` in the Law frontmatter:

| Severity  | Default tier | Behavior                                                            |
|-----------|:------------:|---------------------------------------------------------------------|
| `info`    | 1            | Annotation only (dashboard + audit stream)                           |
| `notable` | 2            | Reminder in next turn's pre-thinking block                           |
| (custom)  | 3            | Block the offending tool call (requires `mode=blocking` + sync path) |
| `critical`| 3            | Block                                                                 |
| (custom)  | 4            | Block + down-weight model in router                                   |

Tier 4 is never the default — a human (or policy file) must opt a law in.

### `/v1/laws/check` flow

```
request
  │
  ▼
LawRegistry.evaluate_blocking(event, session)      (spec 13; ≤100 ms)
  │
  ▼
for each violation v:
    persist asynchronously to Parquet + emit cortex.events.law_violation (spec 07 writes nodes)
    if v.tier >= 3 → block=true
    if v.tier == 2 → enqueue reminder (KV)
    if v.tier == 4 → trust_score.penalize(model, repo, now)
response.violations ← collected list
```

The persistence is fire-and-forget from the caller's perspective; a durable local queue guarantees at-least-once delivery even if Synap stutters (same queue mechanic as spec 10's overflow WAL).

### Reminder injector

- Reminders have a default TTL of **30 min** (configurable per law).
- When spec 12 assembles a bundle, it merges active reminders for the session into the **Laws** section, annotated `[reminder]`.
- Reminders are dropped after 1 emission by default (flag `emit_policy: once|until_acknowledged|sticky`).
- "Acknowledged" = model issued a tool call that does not repeat the offending pattern within 5 subsequent turns.

### Materialized-view detectors

Some laws are **windowed**: "10 failed tests in a row", "same file edited 5× in 10 min", "2 regressions after the same fix". These cannot be expressed in a single-event detector.

Approach: **small SQL views over recent events** materialised every 30 s.

- Store: embedded SQLite in `cortex-governance` (rebuilt from the Synap stream on restart; bounded to the last 24 h).
- View definitions live alongside the Law file:

```markdown
---
id: LAW-030
mode: observational
severity: notable
detector:
  runtime: materialized
  view: views/law-030.sql
  threshold: "count >= 10"
---
```

`views/law-030.sql`:

```sql
-- Count consecutive failed test runs per repo
WITH recent_tests AS (
  SELECT session_id, repo, ts, outcome
  FROM events
  WHERE kind = 'tool_call.completed'
    AND tool_name IN ('Bash')
    AND payload LIKE '%npm test%'
    AND ts > unixepoch('now', '-30 minutes') * 1000
)
SELECT session_id, repo, COUNT(*) AS count
FROM recent_tests
WHERE outcome = 'failure'
GROUP BY session_id, repo
HAVING COUNT(*) >= 10
```

The governance engine evaluates the view every 30 s; rows → synthesised Violations with evidence pointing at the constituent event IDs.

### Trust-score job

- **Nightly** (04:00 UTC): scan the last 30 days of events + violations per `(model, repo)`; recompute scores; publish to `cortex.governance.trust`.
- **On-demand:** `POST /v1/governance/trust/recompute?model=X&repo=Y` forces a scoped recompute (<5 s, bounded work).
- Formula:

```
raw = Σ (weight_i × normalized_signal_i)
score = smoothstep(0.0, 1.0, raw)              // maps to [0,1]
```

Scores are persisted in SQLite (same store as the materialized views) + mirrored to Nexus as properties on the `Model → Repo` edge (`USED_MODEL.trust_score`).

### Router integration

- If Rulebook is installed and running on the same machine, the engine POSTs trust deltas to Rulebook's `trust-feed` endpoint.
- Rulebook decides how to use them (model selection, prompt adjustments).
- Without Rulebook, scores are visible in the dashboard and via the API but do not affect routing.

### Rate control / denial-of-reminder

Prevent reminder spam:

- Per `(session_id, law_id)`, at most **1 active reminder** at a time; new hits overwrite TTL.
- Per session total reminders capped at **10** simultaneously; oldest dropped on overflow.

### Failure modes

| Failure                               | Handling                                                               |
|---------------------------------------|------------------------------------------------------------------------|
| `LawRegistry` load failure            | `/v1/laws/check` returns 503; adapter fails-open per spec 10           |
| Detector timeout in sync path         | Law is skipped; metric; violation captured async if detector later finishes |
| Materialized view SQL error           | View disabled; metric; other laws unaffected                           |
| Reminder KV write failure             | Reminder dropped; metric; violation still persists                     |
| Trust-score recompute failure         | Previous day's score remains valid; retry next night                   |
| Router feed push failure              | Retry with backoff; if Rulebook is offline >1 h, alert + stop retrying until healthy |

### Observability

```
cortex.gov.check.total            counter, labels: result (allow|block)
cortex.gov.check.latency_ms       histogram
cortex.gov.violations.total       counter, labels: law_id, tier
cortex.gov.reminders.queued       counter, labels: law_id
cortex.gov.reminders.emitted      counter, labels: law_id
cortex.gov.views.evaluations      counter, labels: view
cortex.gov.views.violations       counter, labels: view
cortex.gov.trust.recompute.ms     histogram
cortex.gov.trust.score_delta      histogram, labels: model
```

## Acceptance criteria

- [ ] `/v1/laws/check` against a `Bash git commit --no-verify` tool-call event returns `block: true` + `violations[0].law_id = LAW-007` in ≤200 ms (including registry eval).
- [ ] A `tier=2` violation enqueues a reminder under `governance:reminders:<session>`; spec 12 bundle test shows the reminder in the Laws section.
- [ ] A `tier=4` violation triggers a trust-score penalty visible via `/v1/governance/trust?model=X&repo=Y`.
- [ ] Materialized view: 10 synthetic failed-test events within 30 min produce one Violation of LAW-030; further failures within the same window do not produce duplicates.
- [ ] Nightly trust-score recompute handles 30 days of data for 10 `(model, repo)` pairs in ≤60 s on dev hardware.
- [ ] On-demand recompute endpoint scopes to a single `(model, repo)` and finishes ≤5 s.
- [ ] Reminder TTL: a reminder with 30 min default expires; subsequent bundle omits it; counter records the expiry.
- [ ] Reminder dedup: 5 rapid-fire hits on the same `(session_id, law_id)` result in 1 active reminder.
- [ ] Rate control: 20 distinct laws violated in one session → 10 active reminders, oldest dropped; log records drops.
- [ ] Router feed: mock Rulebook receiver gets a trust delta on recompute; Rulebook offline → retries + eventual alert.
- [ ] Engine restart: materialized-view SQLite is rebuilt from the Synap stream within 60 s; views report stable results after warm-up.
- [ ] Telemetry counters non-zero after a synthetic soak.

## Decisions

1. **Sync-blocking only for critical / tier-3+.** Anything weaker is observational, to keep the adapter's hot path honest.
2. **Reminders, not runtime prompt surgery.** We inject via the pre-thinking bundle, not by editing the model's context mid-generation. Predictable, auditable.
3. **Embedded SQLite for materialized views.** No extra service; restart-reproducible from the Synap stream; 24-h retention is enough for every law we can articulate.
4. **Trust score is per-(model, repo).** "Claude Sonnet in Vectorizer" is a different thing from "Claude Sonnet in Rulebook". Per-repo isolation matches reality.
5. **Tier 4 is opt-in.** Down-weighting a model silently is a big lever; we require an explicit law-frontmatter flag.
6. **Durable local queue + at-least-once.** Persistence is eventual; enforcement is synchronous. The queue is the bridge.
7. **Don't block based on trust score alone.** Trust decides routing (via Rulebook), not per-call enforcement. Separation of concerns.

## Open questions

1. **Reward signal.** Today we only penalize. Do we also *reward* (e.g., models that consistently cite decisions)? Leaning toward a small positive signal in the trust formula once we have data.
2. **Session-level trust override.** A model can be trustworthy repo-wide but unreliable on a specific subsystem. Do we split per-(model, subtree)? Defer to Phase 2 quality pass.
3. **User ignoring reminders.** If the *human* disregards a tier-2 reminder repeatedly, is that a signal we surface? Probably yes, but scoped carefully — user-level trust is a political hot potato.

## References

- Architecture §5.4 (laws, punishment ladder, trust), §11 Phase 3 (governance rollout).
- Spec 01 — Event schema.
- Spec 07 — Graph writer (`Law`, `LawViolation`, `USED_MODEL.trust_score`).
- Spec 10 — Claude Code adapter (sync caller of `/v1/laws/check`).
- Spec 12 — Pre-thinking (reminder injection consumer).
- Spec 13 — Laws DSL (detector contract; frontmatter fields).
- Spec 15 — Deep Analysis (can cite trust-score history).
- Spec 16 — Dashboard (law + trust UI).
- Rulebook: `e:/HiveLLM/Rulebook` (optional router integration).
