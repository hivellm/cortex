# Vectorizer auth — cortex-api credentials

> **Phase:** phase11a · **Owner:** Core team · **Status:** 🟢 Implemented

The `cortex-api` daemon talks to the Vectorizer through `vectorizer-sdk`
and caches a JWT on its in-process `VectorizerLane`. Without
credentials the SDK still answers `/health` (so the boot probe passes)
but every `search_vectors` call returns

```
HTTP 401 Unauthorized: {"error":"unauthorized", "message":"..."}
```

and the orchestrator's vector lane is dark for every retrieval intent
(`pre_change_context`, `decision_lookup`, `similar_problems`,
`free_search`).

## Env vars (resolved at boot)

The boot path in [`crates/cortex-api/src/main.rs`](../../crates/cortex-api/src/main.rs) reads the following keys in this **strict precedence**:

| Order | Env key | Behaviour |
|-------|---------|-----------|
| 1 | `CORTEX_VECTORIZER_API_KEY` (or `VECTORIZER_API_KEY`) | Long-lived bearer. Wins outright; no `/auth/login` is run. |
| 2 | `CORTEX_VECTORIZER_USER` + `CORTEX_VECTORIZER_PASSWORD` | Run `/auth/login` once at boot, cache the JWT, run `refresh_token()` reactively on 401. |
| 3 | `CORTEX_EMBEDDER_VECTORIZER_USER` + `_PASSWORD` | Alias for (2). Lets a single `.env` file feed both `cortex-api` and `cortex-embedder` without duplicating the keys. |
| 4 | *(none of the above)* | Anonymous. Boots successfully, logs a `WARN` saying every authenticated search will 401, and leaves the live lane wired so that local `MemoryVectorLane` fallback paths still work. |

## Optional periodic JWT warmup

```
CORTEX_VECTORIZER_JWT_WARMUP_SECS=0
```

When this is set to a positive integer, `cortex-api` spawns a Tokio
task on boot that calls `VectorizerLane::refresh_token()` every
`N` seconds. The default (`0` / unset) leaves refresh reactive — the
first 401 in `vectorizer_lane.rs::search` triggers a re-mint and
retry. Use the warmup loop only when you observe a sustained burst
of 401-then-retry pairs in the logs (cheap and harmless to enable).

## Where the env values land

- **docker-compose**: [`docker-compose.yml`](../../docker-compose.yml) →
  `cortex-api.environment` block. The compose file passes through
  `CORTEX_VECTORIZER_USER` / `_PASSWORD` from the host `.env`, falling
  back to `CORTEX_EMBEDDER_VECTORIZER_USER` / `_PASSWORD`, falling
  back to `admin` / `cortex-dev-admin` (the dev-stack default).
- **Local `.env`**: [`.env.example`](../../.env.example) → the
  `Vectorizer auth (phase11a)` block. Copy to `.env` and edit.
- **CI smoke**: [`scripts/ci/boot-stack.sh`](../../scripts/ci/boot-stack.sh)
  inherits whatever the runner has exported. The smoke flow currently
  relies on the `MemoryVectorLane` fallback path (no live Vectorizer)
  so credentials are not strictly required, but exporting them keeps
  the smoke flow forward-compatible if the suite ever adds a live
  Vectorizer container.

## Boot sequence

1. Resolve credentials per the precedence table above.
2. Build the `VectorizerLane` (with cached creds when present).
3. Run `probe_authenticated()` — one cheap authenticated round-trip
   (`list_collections`). On 401 with cached creds, run
   `refresh_token()` once and retry; on persistent 401, log `ERROR`
   and fall back to `MemoryVectorLane` so the daemon stays up.
4. Optionally spawn the warmup loop (when
   `CORTEX_VECTORIZER_JWT_WARMUP_SECS > 0`).

## Diagnosing a misconfigured boot

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| `WARN cortex_api: vector lane: URL set but no credentials configured` | Anonymous boot path. | Set `CORTEX_VECTORIZER_USER` + `_PASSWORD` in the env (host `.env` for compose, or the runner's environment for CI). |
| `ERROR cortex_api: probe_authenticated 401 after refresh` | Credentials are wrong. | Verify the values against the running Vectorizer (`curl -X POST $VECTORIZER_URL/auth/login -d '{"username":"...","password":"..."}'`). |
| Every `cortex_query` returns `errors.vector = "...HTTP 401...no cached credentials..."` | Boot landed on the anonymous branch. | Same as the first row — credentials are missing. |
| Repeated 401-then-retry pairs in logs after long idle periods | JWT expired between calls. | Enable `CORTEX_VECTORIZER_JWT_WARMUP_SECS` (50 min is a safe default for the SDK's 1 h JWT). |
