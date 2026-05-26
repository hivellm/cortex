# Proposal: phase11a_vectorizer_jwt_credential_cache

## Why

The running `cortex-api` daemon serves `/v1/query` with the in-process vector lane stuck on `MemoryVectorLane` semantics: every live call against the Vectorizer SDK returns

```
HTTP 401 Unauthorized: {"error":"unauthorized","message":"Authentication required..."}; no cached credentials to mint a fresh JWT — set CORTEX_VECTORIZER_USER + _PASSWORD (or _EMBEDDER_VECTORIZER_*)
```

Verified live on 2026-04-30 — the MCP probe (`mcp__cortex__cortex_query` for `decision_lookup`, `pre_change_context`, `similar_problems`, `free_search`) returns `results:{}` and `debug.errors.vector` carrying that exact message. Because the boot-time `creds: None` branch in [vectorizer_lane.rs:247](crates/cortex-api/src/vectorizer_lane.rs#L247) cannot mint a fresh JWT, the orchestrator's vector lane is dark on every retrieval intent. The keyword + graph lanes still work, but the fusion result is severely degraded — every `decision_lookup` and `similar_problems` call effectively returns nothing.

Root cause is two-fold:

1. **Env not plumbed into the daemon.** The Docker / local-stack launch path does not export `CORTEX_VECTORIZER_USER` + `CORTEX_VECTORIZER_PASSWORD` (or the `_EMBEDDER_*` aliases) to `cortex-api`, so [main.rs:78-95](crates/cortex-api/src/main.rs#L78-L95) lands in the `VectorizerLane::new(url, None)` branch with no creds cached.
2. **Boot path silently degrades.** When the URL is reachable but auth is missing, the probe still passes (the SDK's `health_check` doesn't require auth), so the lane is wired live and only fails on the first `search_vectors` call. There is no boot-time signal that the lane will 401 on every real query.

## What Changes

1. **Plumb credentials end-to-end.** Add `CORTEX_VECTORIZER_USER` + `CORTEX_VECTORIZER_PASSWORD` to the canonical Cortex stack env (compose file, dev `.env`, GUI launcher), and to the docker-stack adapter env if applicable. Document the precedence (`_API_KEY` > `_USER`+`_PASSWORD` > anonymous) in the runbook.
2. **Boot-time auth probe.** Add `VectorizerLane::probe_authenticated()` so that when creds are present we run a single authenticated round-trip before declaring the lane live. On 401 with cached creds, attempt one immediate refresh; on persistent 401, log ERROR and fall back to memory lane.
3. **Loud boot warning when URL is set but creds are not.** Add a `warn!` for the URL-set-but-anon path explicitly stating "every authenticated search will return 401".
4. **Periodic JWT warmup (optional, behind env flag).** A periodic `tokio::spawn` that calls `refresh_token()` so the cached JWT never reaches the SDK after expiry. Gated by `CORTEX_VECTORIZER_JWT_WARMUP_SECS` (default disabled).

## Impact

- Affected code:
  - [crates/cortex-api/src/main.rs](crates/cortex-api/src/main.rs) — boot warn + optional warmup spawn.
  - [crates/cortex-api/src/vectorizer_lane.rs](crates/cortex-api/src/vectorizer_lane.rs) — authenticated probe.
  - Compose / `.env` / GUI launcher env wiring (separate, listed in tasks.md).
- Breaking change: NO — env vars are additive, behaviour stays the same when unset.
- User benefit: every `cortex_query` MCP call against `decision_lookup` / `similar_problems` / `pre_change_context` / `free_search` returns vector hits instead of empty `results:{}` and a 401 in `debug.errors.vector`.

## Source

- Live MCP probe transcript (2026-04-30): every intent returns `errors.vector = "...HTTP 401...no cached credentials..."`.
- [vectorizer_lane.rs:247-252](crates/cortex-api/src/vectorizer_lane.rs#L247-L252) — exact error path that fired.
- [main.rs:60-128](crates/cortex-api/src/main.rs#L60-L128) — boot selection that landed on the no-creds branch.
