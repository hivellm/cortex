# Proposal: phase11s_pipeline_drainage_recovery

## Why

The 2026-05-03 phase11p reindex surfaced three independent worker-resilience bugs that together cause silent event loss in the ingestion pipeline. Each bug was discovered live during the post-phase11k corpus reindex, and each blocks downstream coverage in a different way:

### Gap 1 — `cortex-classifier-worker` dormant for 26 hours

`docker logs cortex-classifier-worker` showed the worker stopped logging at `2026-05-02T01:59:02 UTC` after a `synap consume cortex.events.raw: HTTP error: error sending request for url (http://synap:15500/api/v1/command)` warning. It never recovered. By 2026-05-03 04:14 (when the operator manually `docker restart` revived it), every envelope published to `cortex.events.raw` during that 26-hour window had piled up unclassified — no `cortex.events.classified` produced, so the embedder / fulltext / graph workers downstream had nothing to consume.

The worker's `compose.yml` block sets `restart: unless-stopped`, which catches the process crash but NOT a stuck process. The `/healthz` endpoint stayed green because it only reports "process alive" — there is no liveness probe on the consume loop. The bug is the absence of a watchdog: a synap-consume failure should either (a) crash the process so docker restarts it, or (b) self-heal via reconnect with bounded retries.

### Gap 2 — `cortex-graph-worker` Synap consumer-group offset is ephemeral

`crates/cortex-workers/src/graph/worker.rs` carries the comment:

> "Synap 0.11 has no durable consumer-group surface in the SDK; the worker tracks its own offset ephemerally."

Result: every container restart resets the offset (defaults to "latest"). Events published while the worker is down or rebuilding are silently lost. The phase11p reindex landed during three back-to-back graph-worker rebuilds (Cypher SET-r hotfix v1, then v2, then nexus_client.rs sibling fix); each restart dropped a window of events. Post-rebuild cypher 0 errors, but only 8 of 20 sibling repos surface as Repo nodes in Nexus — the 12 missing repos (compressionprompt, expert, hivegpu, hivehubcloud, lexum, rulebook, transmutation, transmutationlite, umicp, vectorizersync, tmltextmate, tmldocs) all have full Meili coverage (which uses spec-08's separate consumer that DOES persist offsets via Meili's task uid system) but zero Nexus Artifact nodes.

### Gap 3 — `cortex-embedder-worker` Vectorizer auth broken

`docker logs cortex-embedder-worker` shows continuous:

> `embed vectorizer_error event_id=01KQP0... detail=other: HTTP 401 Unauthorized: {"error":"unauthorized","message":"Authentication required. Provide a valid JWT token or API key."}`

The container is wired with `CORTEX_EMBEDDER_VECTORIZER_USER=admin` + `CORTEX_EMBEDDER_VECTORIZER_PASSWORD=cortex-dev-admin` (per `.env`), and a manual `POST /auth/login` against the same Vectorizer with those credentials does succeed (returns `access_token`). The embedder either (a) does not refresh its JWT after the 1-hour expiry the Vectorizer issues, or (b) sends the token under the wrong header. Vectorizer coverage today is **92,954 vectors vs Meili's 290,608 docs (32 %)** — much of that gap is the embedder failing to authenticate over long runtimes.

## What Changes

In this repo only:

### §1 — Classifier-worker self-healing watchdog

1. Wrap the consume loop in `cortex-classifier-worker/src/main.rs` with an outer supervisor: on N consecutive `synap consume` errors (default 5), `process::exit(1)` so docker's `restart: unless-stopped` brings the container back fresh.
2. Add a per-tick `last_consume_ts` field to the `/healthz` payload; update on every successful consume call. Operators can monitor with `curl /healthz | jq .extras.last_consume_ts` for staleness.
3. Update the existing `cortex-ops doctor` command to flag classifier-worker as degraded when `last_consume_ts` is more than `<staleness_threshold_ms>` (default 60_000 ms) old.

### §2 — Graph-worker durable consumer offset

1. Audit `crates/cortex-workers/src/graph/worker.rs` for the offset persistence path. The current TODO ("Synap 0.11 has no durable consumer-group surface") is now stale: Synap 0.12 (running on the live stack — verified via `/health` returning `version: "0.12.0"`) has consumer-group support per the upstream changelog.
2. Wire `cortex-graph-worker` to the Synap 0.12 durable-consumer-group API. Persistent offset key: `cortex-graph-{instance_id}` (single-instance default; the worker pool already shares one consumer cursor in-process).
3. On worker boot, resume from the persisted offset; on every successful batch flush, advance the offset.
4. If the Synap SDK does not yet expose the durable-consumer-group surface, fall back to: persist the last-processed event_id to the local SQLite `metadata` DB (`cortex-graph-{instance_id}.last_event_id`) and on boot, replay the stream from `event_id > $last`. The replay path uses the existing `consume(room, Some(offset), Some(max))` API.

### §3 — Embedder Vectorizer JWT refresh

1. Audit `crates/cortex-workers/src/embedder/vectorizer_client.rs` for the auth path. Confirm whether the JWT is fetched once at boot and never refreshed, or whether the refresh logic exists but is broken.
2. Add a token-cache wrapper that:
   - Tracks the JWT's `expires_in` (Vectorizer returns 3600 seconds = 1 hour).
   - Refreshes the JWT 60 seconds before expiry.
   - Falls back to a re-login on any 401 response (defensive — handles server-side token rotation).
3. Add a regression test that mock-Vectorizers a 401, asserts the embedder retries with a fresh login, and assert no envelopes are dropped during the refresh.

### §4 — Drain-recovery integration test

1. Bring up a 3-container ephemeral stack (Synap + Vectorizer + Nexus) via testcontainers / docker-compose.test.yml.
2. Bootstrap a fixture repo, kill+restart each worker mid-flow, assert that final corpus counts match the bootstrap event count exactly.
3. Gate behind `CORTEX_DRAIN_RECOVERY_IT=1` (live-stack required).

### §5 — Backlog drain runbook

Document the operator playbook for "the pipeline silently dropped events":

1. Check classifier `last_consume_ts` via `cortex-ops doctor`.
2. If stale, `docker restart cortex-classifier-worker`.
3. Check graph-worker offset against Synap stream length; force replay via `cortex-ops graph replay --since=<event_id>` (new subcommand from §2).
4. Check embedder Vectorizer auth via `cortex-ops doctor` (already wired).
5. Re-bootstrap any repos whose Nexus / Vectorizer counts lag behind Meili by > 10 %.

## Impact

- **Affected code:** `crates/cortex-workers/src/{classifier,graph,embedder}/worker.rs`, `crates/cortex-workers/src/embedder/vectorizer_client.rs`, `crates/cortex-cli/src/bin/cortex-ops.rs` (new `graph replay` subcommand + doctor checks), `docker-compose.yml` (no changes required — the existing `restart: unless-stopped` policy handles the new exit-on-stuck path).
- **Affected docs:** `docs/cortex/pipeline-drainage-runbook.md` (new), CHANGELOG entry under `[Unreleased]` Operations.
- **Breaking change:** NO. The watchdog exit on stuck consume is a process-internal behaviour — restart policy is already `unless-stopped`. Durable consumer-group migration is additive: pre-existing offsets are preserved by the new path's first-pass scan from `event_id > 0` if no persisted offset is found. JWT refresh is internal to the embedder.
- **User benefit:** post-bootstrap pipeline drainage becomes deterministic. Vectorizer coverage rises from 32 % toward parity with Meili. Operators no longer need to detect dormant workers manually.
- **Cost:** zero LLM tokens. Bounded extra HTTP traffic for JWT refreshes (~24 calls/day at 1-hour TTL).

## Source

Live audit on 2026-05-03 against the running stack (commits `9717e81`, `6a3ceba`, `3459a82`, `3b20fb6`):
- classifier-worker logs: dormant since `2026-05-02T01:59:02Z`.
- graph-worker source: `crates/cortex-workers/src/graph/worker.rs` ephemeral-offset comment.
- embedder logs: continuous HTTP 401 against Vectorizer (verified via `curl http://127.0.0.1:17001/auth/login` which DOES return a token under the same credentials).
