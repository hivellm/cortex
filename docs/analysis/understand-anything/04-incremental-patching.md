# Incremental Graph Patching — Spec for Cortex

Reconstruction of UA's incremental-update algorithm (`fingerprint.ts` + `staleness.ts` +
`change-classifier.ts`) as a portable spec for Cortex's graph/embedding indexer. See
[findings.md](02-findings.md) F-1, F-2.

---

## 1. UA's algorithm (as observed)

**State stored** (`meta.json`): `gitCommitHash` (last indexed HEAD), `analyzedAt` timestamp.

**Staleness check** (`staleness.isStale()`):
```
changed = `git diff <lastCommitHash>..HEAD --name-only`   // split lines, drop empties
stale   = changed.length > 0
```

**Change classification** (`change-classifier.classifyUpdate()`):
```
structuralCount = structurallyChangedFiles + newFiles + deletedFiles   // cosmetic excluded
SKIP                 if all changes NONE | COSMETIC
PARTIAL_UPDATE       else (localized, no dir reorg)
ARCHITECTURE_UPDATE  if dirsAddedOrRemoved  OR  structuralCount > 10   → rerunArchitecture, rerunTour
FULL_UPDATE          if structuralCount > 30  OR  structuralCount > 50% of files
```
Cosmetic = whitespace/comment-only (no AST-significant delta).

**Merge** (`mergeGraphUpdate()`):
```
1. removed = nodes where node.filePath ∈ changedFiles
2. nodes  := nodes \ removed
3. edges  := edges where source ∉ removed.ids AND target ∉ removed.ids   // prune dangling
4. {newNodes,newEdges} = analyze(changedFiles)                            // re-extract only changed
5. nodes ++= newNodes ; edges ++= newEdges
6. meta.gitCommitHash = HEAD ; meta.analyzedAt = now
```

**Trigger surface** (`hooks.json`): PostToolUse on `git (commit|merge|cherry-pick|rebase)` and
SessionStart when `meta.gitCommitHash != git rev-parse HEAD`.

---

## 2. Gaps to close for Cortex

| UA limitation | Cortex requirement |
|---------------|--------------------|
| Node↔file by `filePath` string match | Cortex needs an explicit `file_path → {node_ids}` index for O(1) removal; also handle renames (`git diff --name-status` R) so a moved file rebinds, not delete+create |
| No bitemporal | Cortex closes edges with `valid_to = now` instead of hard delete — history is queryable (timeline work depends on it) |
| Embeddings not re-computed in merge (UA's are external) | Cortex must re-embed only the changed files' nodes and upsert vectors |
| Cosmetic detection is heuristic | Cortex can reuse the same NONE/COSMETIC/STRUCTURAL classification from its AST extractor |
| Single repo | Cortex is multi-repo — key the fingerprint by `repo_id` |

---

## 3. Proposed Cortex algorithm (Rust, worker-side)

```text
fn reindex(repo: RepoId) {
  let last = store.last_indexed_commit(repo);            // None on first run → FULL
  let head = git_head(repo);
  if last == Some(head) { return; }                     // no-op

  let diff = git_name_status(last, head);                // Vec<(Status, path[, old_path])>
  let class = classify(&diff);                           // SKIP|PARTIAL|ARCH|FULL
  if class == SKIP { store.set_last_indexed(repo, head); return; }

  let changed = diff.affected_paths();
  // 1. bitemporal-close nodes/edges for changed files (valid_to = now), keep history
  graph.close_for_files(repo, &changed, now);
  // 2. handle renames: rebind node identity old_path → new_path
  graph.rebind_renames(repo, diff.renames(), now);
  // 3. re-extract changed files (deterministic) → facts
  let facts = extractor.run(repo, &changed);
  // 4. LLM annotate under reconciliation contract (see extraction-contract.md)
  let (nodes, edges) = annotate(facts);                 // reconciled
  // 5. upsert nodes/edges with valid_from = now
  graph.upsert(repo, nodes, &edges, now);
  // 6. re-embed only changed nodes, upsert vectors
  embedder.reembed(repo, nodes.ids());
  // 7. if class >= ARCH: invalidate architecture summaries / topic cards for repo
  if class.rerun_architecture() { consolidation.invalidate_arch(repo); }
  if class == FULL { consolidation.invalidate_all(repo); }

  store.set_last_indexed(repo, head);
}
```

**Idempotency gate (test):** running `reindex` twice with no new commits ⇒ second call is a no-op
(`last == head`), graph byte-identical.

**Cosmetic gate (test):** a comment-only commit classifies SKIP ⇒ zero graph writes, zero
re-embeds, but `last_indexed_commit` still advances to HEAD.

---

## 4. Thresholds — adopt or tune

UA's 10 / 30 / 50% thresholds are repo-size-naive. For Cortex multi-repo, make them config per
repo profile (a 50-file repo and a 5000-file repo shouldn't share absolute counts). Default to
UA's numbers, expose `arch_threshold`, `full_threshold_files`, `full_threshold_pct`.

---

## 5. Why this beats full rebuild

A typical commit touches 1–5 files. Full rebuild re-extracts + re-embeds the entire repo (minutes,
$$). Patch re-does ~5 files (sub-second) and only invalidates architecture-level synthesis when the
change is structural enough to warrant it. This is the single highest-ROI borrow from UA.
