## 1. Fingerprint persistence
- [x] 1.1 Persist per-repo `last_indexed_commit_hash` (keyed by `repo_id`) in `cortex-storage`
- [x] 1.2 Round-trip read/write + missing-fingerprint resolves to `None` (first run)

## 2. Staleness + changed-file resolver
- [x] 2.1 `git diff <last>..HEAD --name-status` resolver yields changed/new/deleted/renamed sets
- [x] 2.2 Equal hashes (`last == HEAD`) short-circuit to a no-op
- [x] 2.3 Unit test on a fixture repo (add/modify/delete/rename cases)

## 3. Change classifier
- [x] 3.1 Implement `classify` returning tiers `NOOP` / `PARTIAL_UPDATE` / `ARCHITECTURE_UPDATE` / `FULL_UPDATE` from structural-change count
- [x] 3.2 Thresholds configurable per repo (default 10 / 30 / 50%); cosmetic-only diff returns `NOOP`
- [x] 3.3 Table-driven tests per threshold boundary

## 4. Node-id to file index + merge
- [x] 4.1 Maintain `file_path -> {node_ids}` index for O(1) lookup
- [x] 4.2 `merge_graph_update`: bitemporal-close nodes/edges for changed files (no hard delete), prune dangling edges
- [x] 4.3 Rename rebind: `R old->new` rebinds node identity instead of delete+create
- [x] 4.4 Re-extract changed files, upsert nodes/edges with `valid_from = now`
- [x] 4.5 Re-embed only changed-file nodes; upsert vectors

## 5. Scheduler gating + trigger
- [x] 5.1 Gate consolidation/topic-card re-synthesis: invalidate arch on `ARCHITECTURE_UPDATE`, invalidate all on `FULL_UPDATE`
- [x] 5.2 Wire staleness check into existing commit capture + SessionStart hook; advance fingerprint even on `NOOP`
- [x] 5.3 Idempotency: a second run with no new commit produces a no-op (graph byte-identical)

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 6.1 Update or create documentation covering the implementation
- [x] 6.2 Write tests covering the new behavior (idempotency gate, cosmetic-noop gate, rename rebind)
- [x] 6.3 Run tests and confirm they pass
