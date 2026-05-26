# Proposal: phase6c_relevance_score_aware_rrf

## Why

Reciprocal Rank Fusion (`crates/cortex-api/src/fusion.rs:16-49`) uses pure positional rank `1/(60+rank)` and discards the lane-native scores that the lanes already capture into `LaneHit.score`. When the keyword lane returns 1 doc and the vector lane returns 50, the keyword doc gets rank 1 → `1/61 ≈ 0.0164`; a *single hit* from the graph lane (rank 1) gets the same `1/61` — i.e. a single weak graph hit can outrank dense, semantically-correct vector hits.

This becomes worse as `phase4c` lands richer graph edges: more graph hits flow into fusion; positional-only RRF will surface bad-but-confident graph results above dense vector top-3. The Meili lane's `_rankingScore` is captured into `LaneHit.score` ([`crates/cortex-api/src/meili_lane.rs:103-105`](../../../crates/cortex-api/src/meili_lane.rs)) but never consulted by `rrf_fuse` — the data is there, the fusion just throws it away.

R2 step 5 in the relevance plan, closes F-005.

Source: `docs/analysis/relevance/01-findings.md` §F-005; `docs/analysis/relevance/02-execution-plan.md` §R2; `docs/analysis/relevance/03-knowledge-and-memory.md` ADR — Score-aware RRF.

## What Changes

### Score blend in `rrf_fuse`
Replace the pure positional formula with a weighted blend:

```
fused_score(hit) = α · positional + (1 − α) · normalized_native
where
  positional        = 1 / (K + rank)
  normalized_native = lane.normalize(hit.score)
  α                 = 0.7   // tunable; default biases toward RRF stability
  K                 = 60    // unchanged
```

### Lane-side score normalisation
Lane-native scores live on different scales:
- Vectorizer cosine similarity: `[0, 1]` already — no transform.
- Meili `_rankingScore`: `[0, 1]` already — no transform.
- Nexus graph: today returns `0.0`. Until F-002 closes (`phase4c`), the graph lane's normalized score stays at `0.0`, which means it contributes only via positional rank. After F-002, the graph lane SHALL stamp a path-length-derived score on each `LaneHit.score` (`1.0` for direct neighbour, `0.5` for 2-hop, `0.25` for 3-hop) — out of scope for this task; documented as a follow-up assertion.

A trait method `LaneHit::normalized_score(&self) -> f32` returns `self.score.clamp(0.0, 1.0)`. Lanes that already produce `[0,1]` scores need no change.

### Configurability
- Read `α` from `CORTEX_RRF_ALPHA` env (default `0.7`); reject values outside `[0.0, 1.0]` at boot with `tracing::warn!` + fall back to default.
- Read `K` from `CORTEX_RRF_K` env (default `60`); reject `<= 0`.

### Observability
Stamp `fusion_alpha` + `fusion_k` on the audit envelope so phase6e's harness can attribute regressions to fusion-tuning changes.

### Regression invariants
The existing tests in `crates/cortex-api/src/fusion.rs::tests` cover positional invariants. Extend with:
1. **"Weak graph hit doesn't outrank dense vector top-3"** — fixture: vector lane top-3 with native score `[0.92, 0.88, 0.85]`, graph lane top-1 with native score `0.10`; assert graph hit lands at position ≥ 4 in fused order.
2. **"All-equal native scores reduce to positional RRF"** — when every hit has `native_score = 0.5`, fused order MUST match the pure-positional baseline within float epsilon.
3. **"Boundary alpha"** — `α = 1.0` MUST exactly reproduce today's positional-only behaviour (regression escape hatch); `α = 0.0` MUST sort by native score alone.

## Impact

- Affected specs: [`docs/specs/11-query-api.md`](../../../docs/specs/11-query-api.md) (fusion algorithm + env knobs).
- Affected code: `crates/cortex-api/src/fusion.rs` (blend formula); `crates/cortex-api/src/lanes.rs` (`normalized_score` helper); `crates/cortex-api/src/audit.rs` (stamp `fusion_alpha` / `fusion_k`); `crates/cortex-api/src/main.rs` (read env knobs at boot).
- Breaking change: NO — default `α = 0.7` produces a different fused order than today, but no API shape changes. Operators wanting today's behaviour set `CORTEX_RRF_ALPHA=1.0`.
- Depends on: nothing structurally; the harness in `phase6e` is what proves the change actually helps. **Sequencing recommendation**: land this AFTER `phase6e` so the alpha tuning has a measurement substrate. Without the harness, picking `α = 0.7` is guesswork.
- User benefit: fused bundles stop being whipsawed by sparse-lane single-hit results; dense semantic matches retain their position when a weak graph hit shows up alongside.
