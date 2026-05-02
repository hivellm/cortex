# Nexus external-id migration — operator runbook

Phase11l_nexus-external-ids-migration migrates Cortex's graph layer from the legacy `natural_key` convention to Nexus 2.1's reserved `_id` slot. This runbook walks the cutover end-to-end. ADR-004 (Cortex graph nodes carry their identity in Nexus's reserved `_id` slot) records the rationale.

## Pre-migration checklist

1. **Nexus 2.1.0 container running**:
   ```
   curl -s http://127.0.0.1:17002/health | jq -r '.version'
   # → 2.1.0
   ```
   If the version is `2.0.0` or earlier, bump the `docker-compose.yml` pin (`hivehub/nexus:2.1.0`) and recreate: `docker compose up -d nexus`.

2. **SDK round-trip green**:
   ```
   CORTEX_NEXUS_EXTERNAL_ID_IT=1 cargo test -p cortex-workers --test nexus_external_id_smoke_it
   ```
   All six cases must pass (sha256 + str round-trips, all three conflict policies, absent-id resolution).

3. **Workspace builds clean**:
   ```
   cargo check --workspace --all-targets
   cargo fmt --all --check
   ```
   No errors, no fmt drift on the phase11l-touched crates.

4. **Archive partition writable**:
   ```
   ls $CORTEX_ARCHIVE_ROOT/events/
   # any partition listing — confirms bind mount + permissions
   ```

5. **Backup the current graph DB** (optional but recommended):
   ```
   docker exec cortex-nexus tar czf /tmp/nexus-pre-phase11l.tar.gz /var/lib/nexus
   docker cp cortex-nexus:/tmp/nexus-pre-phase11l.tar.gz ./backup/
   ```

## Drop the legacy graph

Stop the graph worker so it does not re-ingest mid-drop:

```
docker compose stop cortex-graph-worker
```

Run the drop command in `--dry-run` first to verify the per-label counts:

```
cargo run --bin cortex-ops -- graph drop --dry-run --json | jq .
```

When the counts look right, apply:

```
cargo run --bin cortex-ops -- graph drop --confirm
```

Output:

```
cortex-ops graph drop @ http://127.0.0.1:17002  (applied — DESTRUCTIVE)
  Session                          12 345
  Turn                             89 102
  ToolCall                         42 011
  Artifact                          8 873
  Symbol                          124 552
  ...
  TOTAL                           302 198
```

The command is idempotent — re-running on an empty DB reports zero per label.

## Replay (bootstrap + live)

Re-bootstrap every indexed repo via the static-extraction pass (phase11k §5.1):

```
cargo run --bin cortex-bootstrap --release -- \
    --graph-static \
    --graph-archive-root $CORTEX_ARCHIVE_ROOT \
    --workspace .rulebook/cortex-workspace.toml
```

The pass writes one envelope per analyzable file under `$CORTEX_ARCHIVE_ROOT/events/year=*/month=*/day=*/hour=*/bootstrap-graph-static-NNNNN.parquet`. Each envelope carries the inline `payload.metadata.graph_patch` with `nodes[*].external_id` populated.

Restart the graph worker; archive_loader replays the new partitions on boot:

```
docker compose start cortex-graph-worker
docker logs --since 1m cortex-graph-worker | tail -20
```

Expected log line on first batch:

```
graph batch flushed events=N nodes_upserted=M edges_upserted=K outcome=ok
```

## Post-migration verification

1. **Doctor audit** — every `NodeOp` post-migration MUST carry `external_id`:
   ```
   cargo run --bin cortex-ops -- doctor --json \
     | jq '.graph_patch_audit'
   ```
   `nodes_legacy_only` MUST be `0`; `external_id_ratio` MUST be `1.0`.

2. **Smoke a query** — Artifact lookup MUST resolve through the index:
   ```
   curl -s -X POST http://127.0.0.1:17002/data/cypher \
     -H 'content-type: application/json' \
     -d '{"cypher":"MATCH (a:Artifact {_id: $id}) RETURN a._id LIMIT 1","parameters":{"id":"cortex|crates/cortex-workers/src/lib.rs|sha256:<actual-hash>"}}'
   ```
   Returns the canonical `_id` string; `EXPLAIN` should show an `ExternalIdSeek` operator (per Nexus phase9 §4.6).

3. **Gold-set IT** — phase11i headline acceptance gate:
   ```
   CORTEX_RELEVANCE_IT=1 cargo test -p cortex-api --test relevance_eval_it
   ```
   `MRR@10 >= 0.75` MUST hold across the 40-entry gold-set (now includes 10 phase11k-specific entries exercising the new graph paths).

4. **Dashboard smoke** — the graph view colour-coding reads `_id`:
   ```
   curl -s 'http://127.0.0.1:17000/v1/dashboard/graph?session_id=<recent-session>' \
     | jq '.nodes[] | select(.kind=="artifact") | .id' | head -3
   ```
   Each `id` MUST be the `repo|path|sha256:hex` form (no bare `*` sentinels, no `pending|` prefix in the steady state).

## Rollback procedure

If the post-migration verification fails and rollback is needed:

1. Stop the graph worker:
   ```
   docker compose stop cortex-graph-worker
   ```

2. Revert the SDK pin in `Cargo.toml` (`nexus-graph-sdk = "2.0"`) and the Nexus container image (`hivehub/nexus:2.0.0`) — both lines are documented in CHANGELOG.md "Changed" under phase11l §1.3.

3. Restore the pre-migration backup:
   ```
   docker compose stop nexus
   docker cp ./backup/nexus-pre-phase11l.tar.gz cortex-nexus:/tmp/
   docker exec cortex-nexus sh -c 'cd / && tar xzf /tmp/nexus-pre-phase11l.tar.gz'
   docker compose start nexus
   ```

4. `git revert` the phase11l commits in reverse order (commit hashes from `git log --grep 'phase11l'`).

5. Restart the graph worker. archive_loader re-replays the legacy partitions (the dual-read path in cortex-api/src/archive_loader.rs accepts both shapes during the migration window).

## Tail status check (the §10 gate)

After every step above is green:

```
cargo test --workspace --tests
cargo clippy --workspace -- -D warnings
cargo fmt --all --check
```

Note: `cargo clippy --workspace -- -D warnings` may surface pre-existing clippy errors in unrelated crates (`cortex-health`, `cortex-core` doc lints) that pre-date phase11l. Document them in the task tail; do NOT fix as part of phase11l.

## References

- ADR-004 — [Cortex graph nodes carry their identity in Nexus's reserved _id slot](../../.rulebook/decisions/004-*.md).
- [`docs/cortex/graph-tuning.md`](graph-tuning.md) — operator handbook for the underlying graph correlation layer.
- [`docs/specs/07-graph-writer.md`](../specs/07-graph-writer.md) — schema + edge taxonomy (post-phase11l).
- [`Nexus/.rulebook/archive/2026-05-02-phase9_external-node-ids/`](../../../Nexus/.rulebook/archive/2026-05-02-phase9_external-node-ids/) — Nexus side of the migration (catalog index + Cypher executor).
- [`crates/cortex-cli/src/bin/cortex-ops.rs`](../../crates/cortex-cli/src/bin/cortex-ops.rs) — `graph drop` admin command source.
