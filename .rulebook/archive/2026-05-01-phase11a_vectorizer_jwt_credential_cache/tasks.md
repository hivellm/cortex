## 1. Stack env wiring
- [x] 1.1 Add `CORTEX_VECTORIZER_USER` and `CORTEX_VECTORIZER_PASSWORD` to the canonical compose file under the `cortex-api` service env block
- [x] 1.2 Add the same vars to `.env.example` (and `.env` if checked-in dev defaults exist) with safe placeholders
- [x] 1.3 Plumb the same vars into the GUI / Electron stack launcher so local `npm run stack` boots cortex-api with creds — N/A (GUI/Electron does not own a stack launcher; docker-compose is the canonical launch path, covered by 1.1)
- [x] 1.4 Document the precedence (`_API_KEY` > `_USER`+`_PASSWORD` > anon) in `docs/ops/`

## 2. Boot-time loud signalling
- [x] 2.1 In `cortex-api/src/main.rs` boot path, when `CORTEX_VECTORIZER_URL` is set but neither `_API_KEY` nor `_USER`+`_PASSWORD` are present, log a `tracing::warn!` saying every authenticated search will 401
- [x] 2.2 Same path: include the resolved URL and the env keys checked so operators see what to set

## 3. Authenticated probe
- [x] 3.1 Add `VectorizerLane::probe_authenticated()` that calls one cheap authenticated SDK method (`list_collections` or equivalent)
- [x] 3.2 Boot path uses `probe_authenticated()` when creds are configured, falls back to current `probe()` otherwise
- [x] 3.3 On `probe_authenticated()` 401 with cached creds, run one `refresh_token()` and retry once
- [x] 3.4 On persistent 401, log `ERROR` and fall back to `MemoryVectorLane` so the daemon stays up

## 4. Optional JWT warmup loop
- [x] 4.1 Read `CORTEX_VECTORIZER_JWT_WARMUP_SECS` env (integer, default disabled when unset/0)
- [x] 4.2 When enabled and creds are cached, `tokio::spawn` a periodic loop that calls `refresh_token()` at the configured interval
- [x] 4.3 Loop honours daemon shutdown via the existing graceful-shutdown signal channel — Tokio drops the spawned task when `axum::serve` returns at process exit (this binary has no explicit shutdown channel; the spawn is a non-blocking timer with no resources to release)

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation
- [x] 5.2 Write tests covering the new behavior
- [x] 5.3 Run tests and confirm they pass
