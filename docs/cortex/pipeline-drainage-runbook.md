# Pipeline drainage runbook

When the post-bootstrap counts diverge — Meili shows N envelopes
but Vectorizer or Nexus carries < N/2 — work through this runbook
in order. It captures the three failure classes phase 11s
hardened against and the structural recovery commands.

| Symptom | Likely cause | Recovery |
|---------|--------------|----------|
| Classifier worker container alive but `cortex.events.enriched` shows zero new envelopes for hours | §1 — consume loop dormant after a transient Synap blip | `cortex-ops doctor` flags `classifier-worker stuck`; `docker restart cortex-classifier-worker` |
| Specific repos missing from Nexus while Meili has full coverage | §2 — graph-worker offset reset to "latest" across a container restart, dropping the rebuild window | `cortex-ops graph replay --since=<known_good_offset>`; `docker restart cortex-graph-worker` |
| Embedder logs show continuous `HTTP 401 Unauthorized` against Vectorizer | §3 — JWT expired and the static-token client never refreshed | Embedder auto-refreshes when built via `with_credentials`; read `/healthz.extras.jwt_refresh_total` to confirm; legacy bins built via `new()` need a `docker restart` to re-login |

## 1. Detect

Run `cortex-ops doctor` against the live host. The post-§1.4 doctor
emits one row per worker:

```
ok     classifier-worker            http://classifier:8081/healthz (last_consume_ts 12345 ms ago, consecutive_errors=0)
ok     vectorizer                   http://vectorizer:17001/health
ok     nexus                        http://nexus:17002/health
ok     synap                        http://synap:17004/health
ok     meilisearch                  http://meili:17003/health
```

A `fail   classifier-worker` row with `degraded: classifier-worker stuck`
is the §1.2 supervisor signal — restart the container. Set
`CORTEX_CLASSIFIER_HEALTH_URL` for the doctor to probe the worker.

## 2. Restart classifier worker

```bash
docker restart cortex-classifier-worker
```

The §1.2 supervisor causes the worker to exit non-zero on
`CORTEX_CLASSIFIER_MAX_CONSUME_ERRORS` consecutive consume errors
(default 5). Docker's `restart: unless-stopped` policy then brings
it back fresh. After the restart, `last_consume_ts_ms` advances
within seconds and `consume_errors_consecutive` resets to 0.

## 3. Replay missing graph-worker window

If the graph worker dropped a window of envelopes during a restart
and Nexus is missing repos:

```bash
# Inspect the current persisted offset.
cortex-ops graph replay --since=0 --dry-run

# Rewind to a known-good offset (e.g. before the lost window).
cortex-ops graph replay --since=<offset_before_loss>

# Restart so the worker resumes from offset+1.
docker restart cortex-graph-worker
```

The §2.3 SQLite-backed `consumer_offsets` table preserves the
cursor across restarts under the natural-key
`(consumer_id, stream)`. The `--metadata-db` flag overrides the
default DB path; use this when the operator host's metadata DB
lives outside the worker's `CORTEX_HOME`.

## 4. Verify embedder JWT auth

```bash
curl -sS http://embedder:8082/healthz | jq .extras.jwt_refresh_total
```

`jwt_refresh_total = 0` after > 1 hour of uptime indicates the
embedder is running in legacy mode (built via `LiveVectorizerClient::new`
with a pre-minted JWT). Switch the deploy to credentials-mode:

```bash
# `.env` already provides CORTEX_EMBEDDER_VECTORIZER_USER +
# _PASSWORD. The §3.2 auto-refresh path activates when the
# password is NOT a JWT (3-segment dot-separated string).
docker restart cortex-embedder-worker
```

After the restart, `jwt_refresh_total ≥ 1` (boot login) and
advances every ~59 minutes via the §3.2 refresh-buffer.

## 5. Re-bootstrap divergent repos

When per-repo Meili / Vectorizer / Nexus counts still diverge by
> 10 % after the above:

```bash
# Read per-repo coverage table.
scripts/doctor/check-pipeline-coverage.sh

# Re-emit the laggard repos through the canonical bootstrap path.
cortex-bootstrap walk --repo=<slug>
```

`scripts/doctor/check-pipeline-coverage.sh` (§5.2) prints the per-repo
counts and flags any repo whose Nexus or Vectorizer count is < 50 %
of the Meili count.

## Related

- [ADR-008 — durable consumer offset via SQLite](../../.rulebook/decisions/008-durable-consumer-offset-via-sqlite.md)
- [`docs/specs/06-embedder.md`](../specs/06-embedder.md) — embedder spec.
- [`docs/specs/07-graph-writer.md`](../specs/07-graph-writer.md) — graph writer spec.
