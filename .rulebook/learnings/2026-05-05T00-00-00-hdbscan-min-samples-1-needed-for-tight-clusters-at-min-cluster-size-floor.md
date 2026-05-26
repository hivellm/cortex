# HDBSCAN: `min_samples = 1` needed for tight clusters at the `min_cluster_size` floor

**Captured**: 2026-05-05
**Source task**: `phase11p_consolidator-live-read-path` §1.3
**Tags**: clustering, hdbscan, density-based-spatial-clustering, defaults

## What happened

The phase11p `LiveTopicSource` runs HDBSCAN over the inline
embedding arrays carried on per-turn envelopes. With the
consolidator's spec floor (`MIN_CLUSTER_SIZE = 3`) wired through
to `HdbscanHyperParams::builder().min_cluster_size(3).build()`,
the unit + integration tests reliably collapsed every cluster to
the `-1` noise label — even fixtures with two visually obvious
clusters of 5 tightly-packed 2-D points each.

## Root cause

`hdbscan` 0.10's `min_samples` defaults to `min_cluster_size`. The
core-distance computation gates a point as "core" only when it has
≥ `min_samples` similarly-distant neighbours within the K-d tree.
At `min_cluster_size = 3` and 5 points per cluster, the third-nearest
neighbour distance for a point near the cluster centre is dominated
by inter-cluster spread; the algorithm rejects every point as
non-core and labels them all noise.

## Fix

Pin `min_samples = 1` explicitly. With that flag the core-distance
gate becomes the distance to the single nearest neighbour, which
is small inside a tight cluster. Real clusters of `min_cluster_size`
points then pass the density threshold and cluster cleanly.

Code: [`crates/cortex-workers/src/consolidator/source/topic.rs`](../../crates/cortex-workers/src/consolidator/source/topic.rs)

```rust
let hp = HdbscanHyperParams::builder()
    .min_cluster_size(self.min_cluster_size)
    .min_samples(1) // ← phase11p §1.3
    .build();
```

## When to revisit

If we ever raise `min_cluster_size` past ~5 with denser embedding
arrays (≥ 32-D vectors from real Vectorizer collections instead
of synthetic 2-D toy data), the default `min_samples =
min_cluster_size` may behave correctly — the core-distance gate
is more meaningful at scale. Re-test with the live data shape
before changing this away from `1`.

## Cross-refs

- `tests/pruner_safety_it.rs` — sister IT pattern using inline
  embeddings; same pin not strictly needed there because the test
  doesn't run HDBSCAN.
- `phase11j §2.1` — original task that pinned `min_cluster_size`
  but left `min_samples` unspecified; phase11p §1.3 closes the gap.
