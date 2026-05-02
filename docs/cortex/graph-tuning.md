# Graph correlation layer — operator handbook

Operators tuning the phase11k graph correlation layer (static code + markdown analyzers + resolver + sweeper) work this handbook. The renderer's `## Connected files (via IMPORTS_FILE)`, `## Documented under (via DOCUMENTED_BY)`, and `## Cited from (via CITES)` sub-blocks all bottom out in the surfaces below.

## How to spot a missing edge

Symptom: the `Past sessions` / `Consolidated context` block lacks a sub-block you expected, or the dashboard's graph view shows zero hops where you know an import / link / cite exists.

Diagnostic ladder:

1. **Bootstrap pass produced an envelope?**
   ```
   ls $CORTEX_ARCHIVE_ROOT/events/year=*/month=*/day=*/hour=*/bootstrap-graph-static-*.parquet
   ```
   No partition → `cortex-bootstrap --graph-static --graph-archive-root $CORTEX_ARCHIVE_ROOT` was never run for the repo. Run it.

2. **Envelope carries a `graph_patch` payload?**
   ```
   cortex-ops doctor --json | jq '.graph_patch_audit'
   ```
   `nodes_legacy_only > 0` → the producer is a pre-phase11l version. Bump SDK pin + re-run bootstrap.

3. **Patch reached the graph worker?**
   ```
   curl -s http://127.0.0.1:17024/healthz | jq '.extras.jobs_processed_total'
   ```
   Zero or stuck → the live trigger from phase11k §5.2 is not firing. Check `cortex.events.enriched` Synap room — if the room does not exist, the producer is silent (see [`cortex-ingestion env-var fix`](../../../crates/cortex-ingestion/src/config.rs)).

4. **Edge present in Nexus?**
   ```
   curl -s -X POST http://127.0.0.1:17002/data/cypher \
     -d '{"cypher":"MATCH (a:Artifact {_id: $id})-[r]->(b) RETURN type(r), b._id, r.tier LIMIT 50","parameters":{"id":"cortex|src/lib.rs|sha256:abc"}}'
   ```
   Missing → resolver hit a tier mismatch (see next section). Present but missing in renderer → the renderer's `graph_cap` is too low or the edge type is in the catch-all `## Graph neighbours` block.

## How to inspect resolver tier mismatches

The three-tier resolver ([`crates/cortex-workers/src/graph/resolver/mod.rs`](../../../crates/cortex-workers/src/graph/resolver/mod.rs)) decides whether an import lands as `IMPORTS_FILE` (workspace tier 1+2), `IMPORTS_EXTERNAL` (tier 3), or `UNRESOLVED_IMPORT` (fallback). A tier mismatch surfaces as either:

- An import you expected to be intra-workspace landing on `:ExternalPackage` (resolver missed the workspace member). Cause: the bootstrap walker did not pick up the crate's `src/lib.rs`. Verify the path appears under `cortex-bootstrap --graph-static`'s `--graph-archive-root` partition. Common fix: add the crate to the workspace `Cargo.toml` `[workspace.members]` array; re-bootstrap.

- An import landing on `:UnresolvedImport` when both tier-2 and tier-3 should match. Cause: the resolver's `find_by_basename` returned `None` because the workspace has TWO definitions of the same basename (collision). Fix: prefer scoped `use crate::module::Symbol` over a bare `use Symbol` so the analyzer emits a `ResolutionTarget::ModulePath` instead of a `SymbolName`.

- A cross-repo import (e.g. `use vectorizer_sdk::HnswSearch`) landing on `:ExternalPackage` instead of resolving to the sibling `Vectorizer` repo's symbol. Fix: register the SDK in `external_repos.toml` (see below).

To dump the resolver's verdict on a single import, run the analyzer directly via the `cortex-workers` library tests:

```
CORTEX_ANALYZER_TRACE=1 cargo test -p cortex-workers --lib graph::resolver -- --nocapture <test-name>
```

## How to flag a false-positive `:MENTIONS`

The markdown analyzer ([`crates/cortex-workers/src/graph/markdown/mentions.rs`](../../../crates/cortex-workers/src/graph/markdown/mentions.rs)) extracts backtick-token mentions with three-tier disambiguation. Every `:MENTIONS` edge carries a `confidence` prop; values below `0.5` are weak signals the renderer suppresses by default.

To audit false positives:

1. Pull every `:MENTIONS` edge for a doc:
   ```
   MATCH (d:Artifact {repo:'cortex', path:'docs/specs/07-graph-writer.md'})-[r:MENTIONS]->(s)
   RETURN s._id, r.confidence, r.source_line ORDER BY r.confidence ASC
   ```

2. Sort ascending by `confidence`. Anything ≥ 0.9 is a tier-1 / tier-2 hit (high precision). Below 0.5 is a tier-3 fallback (the analyzer guessed at the binding).

3. To suppress a specific token globally, add it to the analyzer's `IGNORED_MENTIONS` list (a follow-up CLI flag tracked under phase11k §6.5). Until that lands, the operator's option is to rephrase the doc — wrap the offending token in italics or quotes instead of backticks.

## How to register a new HiveLLM-internal SDK in `external_repos.toml`

Cross-repo imports (the `vectorizer-sdk` / `nexus-graph-sdk` shape) land as `:IMPORTS_EXTERNAL` by default. Promoting them to `:IMPORTS_FILE` requires declaring the sibling repo so the resolver can walk its `src/lib.rs`:

```toml
# .rulebook/external_repos.toml at the repo root
[vectorizer-sdk]
local_path = "../Vectorizer"
crate_name = "vectorizer"

[nexus-graph-sdk]
local_path = "../Nexus"
crate_name = "nexus_graph"
```

Fields:

- `local_path` (required) — repo-relative path to the sibling clone. The resolver walks `<local_path>/src/lib.rs` with the same `build_rust_module_map_from_root` builder that the host workspace uses.
- `crate_name` (optional) — the crate's `[lib].name` when it diverges from the dependency name (`vectorizer-sdk` exposes a crate named `vectorizer`).

After editing, re-run the bootstrap pass:

```
cortex-bootstrap --graph-static --graph-archive-root $CORTEX_ARCHIVE_ROOT path/to/host/repo
```

The next `cortex-api` boot replays the new partition; the renderer's `## Connected files (via IMPORTS_FILE)` sub-block now surfaces cross-repo hops.

## References

- [`docs/specs/07-graph-writer.md`](../specs/07-graph-writer.md) — schema + edge taxonomy.
- [`docs/specs/12-pre-thinking-injection.md`](../specs/12-pre-thinking-injection.md) §Output — renderer sub-block shapes.
- [`crates/cortex-workers/src/graph/analyzer/`](../../../crates/cortex-workers/src/graph/analyzer/) — per-language code analyzers.
- [`crates/cortex-workers/src/graph/markdown/`](../../../crates/cortex-workers/src/graph/markdown/) — markdown analyzer.
- [`crates/cortex-workers/src/graph/resolver/`](../../../crates/cortex-workers/src/graph/resolver/) — three-tier resolver.
- [`crates/cortex-workers/src/graph/stale_sweeper.rs`](../../../crates/cortex-workers/src/graph/stale_sweeper.rs) — phase11k §5.3 nightly sweeper.
- [`crates/cortex-storage/src/external_repos.rs`](../../../crates/cortex-storage/src/external_repos.rs) — cross-repo SDK declarations loader.
