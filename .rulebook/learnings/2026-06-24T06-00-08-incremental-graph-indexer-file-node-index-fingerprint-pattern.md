# Incremental graph indexer: file_node_index + fingerprint pattern
**Source**: manual
**Date**: 2026-06-24
**Related Task**: phase23b_ua-incremental-indexer
**Tags**: graph, indexer, incremental, fingerprint, bitemporal, sqlite
## Pattern: fingerprint-gated incremental graph merge

Phase23b UA incremental indexer. Key design choices and traps:

1. **file_node_index table** (`(repo_id, file_path, node_id) PRIMARY KEY`) is essential for O(1) node lookup by file path. Without it you'd scan Nexus on every change. Populate it during graph extraction; query it during merge.

2. **Equal-hash fast path MUST be checked before spawning git subprocess.** `if from == to { return Ok(vec![]); }` — avoids spawning git diff for every heartbeat/no-op call.

3. **`ChangeTier: Default` doesn't work** when deriving `Default` on structs that contain `ChangeTier`. Implement the struct init explicitly with `tier: ChangeTier::Noop` rather than derive. The enum has no sensible default.

4. **Cosmetic detection is extension-only** (`.md`/`.txt`/`.rst`/`.adoc`/`.asciidoc`). You can't detect whitespace-only changes from `git diff --name-status` without reading file content. Accept this as conservative — err toward triggering re-analysis.

5. **Bitemporal close via Cypher**: `MATCH (n) WHERE n.id IN [...] SET n.valid_to='...', n.lifecycle='superseded'` — no hard deletes, preserves ADR-018 history. The existing `StaleEdgeSweeper` handles dangling edges.

6. **`consolidation_triggers_for_reindex` gating**: NOOP + PARTIAL emit nothing; ARCH + FULL emit `{"kind":"nightly_topic","repo":repo_id}`. This is intentional — avoid re-synthesizing topic cards on trivial doc/comment changes.

7. **Multi-dir detection heuristic**: structural additions/deletions/renames spanning 2+ distinct top-level directories → `ArchitectureUpdate`. Conservative proxy for "did the module structure change."

8. **Fixture-repo tests**: Use `tempfile::TempDir` + `std::process::Command` to run real `git init` / `git commit` in the test. This is the only way to exercise the `git diff` subprocess without mocking it.

9. **Anti-pattern — `GraphClientError::Internal`**: This variant doesn't exist in `nexus_client.rs`. Use `GraphClientError::Nexus(String)` for wrapping arbitrary Nexus-level errors.
