# phase4e — runbook drives Cypher probes through the existing backfill binary
**Source**: manual
**Date**: 2026-04-28
**Related Task**: phase4e_graph_symbol_backfill_runbook
**Tags**: phase4e, operations, runbook, cortex-graph, nexus
When the operations runbook needs ad-hoc Cypher (schema bootstrap + verification probes) but the daemon SDK and HTTP API don't expose a curl-shaped endpoint, extending the existing `cortex-graph-backfill` binary with `--ensure-schema-only` and `--probe <cypher>` flags is cleaner than writing a new binary or a brittle shell-side HTTP client. The binary already wires `LiveNexusClient` with auth and transport selection, so the probes run against the exact same surface the worker uses — a green probe implies a green worker. The shell script then composes those flags plus a small python3 assertion and a hermetic `--dry-run` that prints the four steps without touching anything, which CI uses as the test surface.