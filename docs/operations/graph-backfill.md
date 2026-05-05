# Graph symbol backfill (phase4e runbook)

[`scripts/graph/backfill-graph-symbols.sh`](../../scripts/graph/backfill-graph-symbols.sh)
replays the archived event stream through the live `cortex-graph-worker`
so the existing Nexus instance gains the `Symbol` nodes and `DEFINES`
edges that
[phase4c](../../.rulebook/tasks/phase4c_graph_richer_edges_defines/proposal.md)
shipped. Phase4c lands the schema, mapper, Cypher, and tests; this
runbook closes the loop by populating the historical graph against a
live cluster.

The runbook is **idempotent** — every node uses its natural key and
every edge is `MERGE`d. Re-running after a worker upgrade is safe; the
expected steady state is "no changes to row counts."

## Prerequisites

| Requirement | Notes |
|---|---|
| `cortex-graph-backfill` on `PATH` | Built from
  [crates/cortex-graph](../../crates/cortex-graph) at or after the
  phase4c merge. Override the binary lookup with
  `CORTEX_GRAPH_BACKFILL_BIN=/path/to/binary`. |
| `CORTEX_NEXUS_URL` | The live Nexus instance the worker writes
  against. Same value the streaming worker uses (`http(s)://…` or
  `nexus://…`). |
| `CORTEX_ARCHIVE_ROOT` | Absolute path to the archive directory
  (`raw-*.parquet` is zstd-compressed NDJSON). On the dev machine this
  is `~/.cortex/archive`. |
| Optional auth env | If the Nexus instance is behind credentials, the
  `cortex-graph` config crate's standard env vars (`CORTEX_NEXUS_USER`,
  `CORTEX_NEXUS_PASSWORD`, …) apply unchanged — the runbook does not
  short-circuit them. |
| `python3` | Used by the runbook to assert probe shape. The script
  fails fast if it is missing. |

## Steps

```sh
# Live run — idempotent.
CORTEX_NEXUS_URL=https://nexus.local:7474 \
CORTEX_ARCHIVE_ROOT=/var/lib/cortex/archive \
  bash scripts/graph/backfill-graph-symbols.sh
```

The script executes four steps in order, fails fast on the first
non-zero exit, and logs the Cypher rows of each probe so an operator
can copy-paste them into a postmortem if anything looks off.

1. **Bootstrap the graph schema.**
   Calls `cortex-graph-backfill --ensure-schema-only`, which applies
   the constraints + indexes from
   [`schema::SCHEMA_STATEMENTS`](../../crates/cortex-graph/src/schema.rs)
   — including the `symbol_natural_key` constraint phase4c added.
2. **Replay the archive.**
   Calls `cortex-graph-backfill --archive-root $CORTEX_ARCHIVE_ROOT`.
   The binary walks the zstd NDJSON, maps each envelope through the
   production
   [`map_event_to_patch`](../../crates/cortex-graph/src/mapper.rs), and
   writes the patches via the same `NexusGraphWriter` the live worker
   uses.
3. **Probe Symbol + Artifact counts.** Executes
   `MATCH (s:Symbol)-[:DEFINES]->(a:Artifact) RETURN count(s) AS sym,
   count(DISTINCT a) AS art` and asserts both columns are `> 0`.
   Prints `sym=<n> art=<m>` for the run log.
4. **Probe the canonical PreThinkingTool DEFINES.** Executes
   `MATCH (s:Symbol {name: "PreThinkingTool"})-[:DEFINES]->(a:Artifact)
   RETURN a.repo AS repo, a.path AS path` and asserts at least one row
   matches `repo="Cortex"` and `path="crates/cortex-mcp-server/src/tools.rs"`.

## Expected output (live mode)

```
[1/4] bootstrapping graph schema against https://nexus.local:7474...
schema bootstrap ok (15 statements applied or already present)
[2/4] replaying archive at /var/lib/cortex/archive...
…batch flushed messages…
backfill done
[3/4] probing Symbol + Artifact counts...
{"columns":["sym","art"],"rows":[[<sym>,<art>]],"execution_time_ms":<ms>}
sym=<sym> art=<art>
[4/4] probing PreThinkingTool DEFINES Artifact...
{"columns":["repo","path"],"rows":[["Cortex","crates/cortex-mcp-server/src/tools.rs"]],…}
  Cortex :: crates/cortex-mcp-server/src/tools.rs

phase4e backfill complete.
```

## Dry-run mode

```sh
bash scripts/graph/backfill-graph-symbols.sh --dry-run
```

Prints the four steps with the exact Cypher each one would send,
without touching Nexus, the archive, or env vars. CI runs this in a
hermetic job so the script's surface stays in lockstep with the
expected probes.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `nexus smoke probe failed` | The Nexus URL or credentials are wrong, or Nexus is not running. The smoke probe uses the same SDK transport as the streaming worker, so failure here = real worker traffic would also fail. |
| `ensure_schema failed: SchemaDrift` | Nexus is on a Cypher dialect that does not understand a constraint statement. Bring the Nexus version in line with what `cortex-graph` declares (currently 1.15) before re-running. |
| Probe 3 returns `sym=0 art=0` | Either the archive root is wrong or the worker binary on `PATH` predates phase4c (`Symbol` nodes are not emitted). Confirm the binary is at or after the phase4c merge. |
| Probe 4 returns no rows | The archive does not contain the `tools.rs` file that defines `PreThinkingTool`, or the file moved. Re-check the expected path against the current source tree. |

## Re-running

The runbook is safe to re-run on every worker upgrade. Steady state on
a clean cluster is: probe 3 unchanged across runs (any growth comes
from new archive content, not from re-replaying old envelopes), probe 4
unchanged unless the file actually moved.
