## §1. Turns clean re-embed (raw-JSON → NL)

- [ ] §1.1 Wire/trigger a `cortex-claude-archive` replay of historical sessions so turn events re-flow through classifier + embedder.
- [ ] §1.2 Drop `cortex-cortex-turns` and re-embed through the stable-id embedder; confirm turn vectors carry NL projection (not raw JSON) and top turn cosine for an NL query rises above ~0.23.

## §2. NL summary quality (clear the static-summary ceiling)

- [ ] §2.1 Replace the raw-JSON-snippet static `summary` with a human-readable per-kind NL projection (no raw JSON), OR wire the CLI-mode classifier (local logged-in `claude` CLI, never the Anthropic API).
- [ ] §2.2 Re-embed the affected collections; confirm top vector cosine for the audit query rises above the ~0.45 static ceiling.

## §3. Dashboard latency series repoint

- [ ] §3.1 Repoint the cortex-api `pre_thinking_p95_ms` dashboard series (and GUI) to the adapter `/healthz` `pre_thinking_latency_ms` source (or add a new series).

## §4. Live verification of phase26e §2/§3 surfaces

- [ ] §4.1 After the host adapter daemon is redeployed, confirm `pre_thinking_cache_hit_total` increments on a repeated query within the TTL and `pre_thinking_latency_ms.p95` < 200ms under normal session load.

## §5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] §5.1 Update or create documentation covering the implementation
- [ ] §5.2 Write tests covering the new behavior
- [ ] §5.3 Run tests and confirm they pass
