## §1. Turns clean re-embed (raw-JSON → NL)

- [ ] §1.1 ⏸ blocked: destructive op needing operator authorization. Wire/trigger a `cortex-claude-archive` replay of historical sessions so turn events re-flow through classifier + embedder.
- [ ] §1.2 ⏸ blocked: requires §1.1 + an explicit operator "yes, drop turns". Drop `cortex-cortex-turns` and re-embed through the stable-id embedder; confirm turn vectors carry NL projection (not raw JSON) and top turn cosine for an NL query rises above ~0.23.

## §2. NL summary quality (clear the static-summary ceiling)

- [x] §2.1 Replaced the raw-JSON-snippet static `summary` with a clean per-kind NL extraction in `classifier/statics.rs` (`nl_summary_snippet` + `summarize_value`): Turn→messages, ToolCall→`tool: k=v`, Decision→title/status/body, etc., generic body/content/text fallback, whitespace collapsed; the embedder hot-path `nl_projection` was intentionally left untouched. Tests: `nl_summary_snippet_is_clean_prose_not_raw_json`; statics 19/19, clippy clean. (CLI-classifier path remains an alternative but is operator-credential-gated.) Built ahead of §1 because §1 is operator-authorization-blocked (LAW-CORTEX-001 exemption #2).
- [ ] §2.2 ⏸ blocked: needs an embedder/classifier docker redeploy + re-embed (operator window). Re-embed the affected collections; confirm top vector cosine for the audit query rises above the ~0.45 static ceiling.

## §3. Dashboard latency series repoint

- [ ] §3.1 Repoint the cortex-api `pre_thinking_p95_ms` dashboard series (and GUI) to the adapter `/healthz` `pre_thinking_latency_ms` source (or add a new series).

## §4. Live verification of phase26e §2/§3 surfaces

- [ ] §4.1 After the host adapter daemon is redeployed, confirm `pre_thinking_cache_hit_total` increments on a repeated query within the TTL and `pre_thinking_latency_ms.p95` < 200ms under normal session load.

## §5. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] §5.1 Update or create documentation covering the implementation
- [ ] §5.2 Write tests covering the new behavior
- [ ] §5.3 Run tests and confirm they pass
