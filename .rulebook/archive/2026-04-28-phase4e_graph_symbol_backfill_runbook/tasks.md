## 1. Backfill runbook
- [x] 1.1 Add `scripts/backfill-graph-symbols.sh` that bootstraps the schema, runs `cortex-graph-backfill` against `$CORTEX_ARCHIVE_ROOT`, and exits non-zero on any worker error
- [x] 1.2 Embed the two Cypher probes in the runbook so a successful run prints `sym=<n> art=<m>` and the `PreThinkingTool` lookup result
- [x] 1.3 Document the runbook in `docs/operations/graph-backfill.md` with prerequisites (Nexus reachable, archive root mounted, phase4c worker binary on PATH) and the expected output

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 2.1 Update or create documentation covering the implementation — the runbook itself satisfies item 1.3 of this task
- [x] 2.2 Write tests covering the new behavior — shell-script `--dry-run` mode that prints the Cypher it would run without touching Nexus, asserted by a CI step
- [x] 2.3 Run tests and confirm they pass — `bash scripts/backfill-graph-symbols.sh --dry-run` returns 0 and emits the expected probe text
