## §1. Bug #2 — Divergence checker: counter alignment + alert format

- [ ] §1.1 Find where the divergence checker reads `ingestion.archived.tool_call` (Synap KV key name or health endpoint field)
- [ ] §1.2 In the ingestion router/health module, publish `ingestion.archived.{kind}` to the location the checker expects after each archive batch
- [ ] §1.3 Remove the synthetic law_violation POST from the divergence reporter; replace with a structured `tracing::warn!` or internal metric counter
- [ ] §1.4 Verify: after fix, divergence endpoint shows `downstream > 0` for tool_call and severity drops from "critical" to "ok"
- [ ] §1.5 Verify: ingestion logs no longer contain rejected "01ALERT…" envelopes

## §2. Bug #6 — Fulltext worker: fallback extraction

- [ ] §2.1 Read the fulltext extractor source; identify the field(s) it tries to extract and the condition that triggers `skipped_empty`
- [ ] §2.2 Implement fallback chain: `summary` → `payload.text` → `payload.output.stdout` → minimal doc from `kind + path + event_id`
- [ ] §2.3 Ensure `skipped_empty` only fires for genuinely empty envelopes (all fallback fields absent or blank)
- [ ] §2.4 Verify: restart fulltext worker; confirm `skipped_empty_total` growth drops to near-zero for normal tool_call and turn events

## §3. Bug #7 — Frames/envelopes ratio: exclude non-capture hooks

- [ ] §3.1 Find the divergence pair definition for `adapter.frames_parsed → adapter.envelopes_built`
- [ ] §3.2 Subtract `PreToolUse` and `UserPromptSubmit` counts from the upstream counter (or define a separate `capture_frames` metric that excludes them)
- [ ] §3.3 Set the expected ratio threshold to ~85% (not 100%) in the divergence check configuration
- [ ] §3.4 Verify: divergence endpoint no longer shows this pair as "critical"

## §4. Tail (mandatory)

- [ ] §4.1 Update `docs/analysis/cortex/12-live-audit-2026-06-09.md` — mark bugs #2, #6, #7 as fixed with commit reference
- [ ] §4.2 Write tests: ingestion counter publish after archive; fulltext extractor fallback for summary-less events; divergence ratio with non-capture frames excluded
- [ ] §4.3 Run `cargo check && cargo test -p cortex-workers -p cortex-api` and confirm all pass
