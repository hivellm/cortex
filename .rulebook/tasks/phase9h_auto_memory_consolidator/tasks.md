## 1. Discovery
- [ ] 1.1 NEW `crates/cortex-ops/src/memory_consolidate.rs`
- [ ] 1.2 Helper `resolve_project_slug(cwd)` mirroring Claude Code's slug rule (replace `:` and `/` with `--`)
- [ ] 1.3 Helper `read_memory_dir(path) -> Vec<MemoryFile { path, frontmatter, body }>` parsing YAML frontmatter strictly
- [ ] 1.4 Files without complete frontmatter are excluded from clustering and a warning is emitted per such file

## 2. Embedding + clustering
- [ ] 2.1 Embed each body via the embedder worker (`POST /v1/embed`); reuse the existing `cortex-embedder` API client
- [ ] 2.2 Pairwise cosine similarity inside each `type` group (`user|feedback|project|reference`)
- [ ] 2.3 Greedy cluster: for each unassigned file, attach to the highest-similarity existing cluster whose representative is ≥ `threshold` (default 0.78); otherwise start a new cluster
- [ ] 2.4 Clusters of size 1 remain untouched

## 3. Sonnet merge
- [ ] 3.1 Add prompt template `consolidate_auto_memory` in `cortex-classifier/src/prompts.rs`: "Produce one memory entry preserving every concrete instruction; keep the strictest constraint when two entries conflict; output frontmatter then body"
- [ ] 3.2 `merge_cluster(cluster) -> Result<MemoryFile, MergeError>` runs the prompt, parses the model output as frontmatter+body, validates fields
- [ ] 3.3 Conflict guard: re-embed the merged body and compare to each source body's vector; if any pair < 0.6 cosine, return `MergeError::DriftedTooFar` and the cluster remains intact

## 4. Apply step
- [ ] 4.1 Move source files into `memory/_archive/<RFC3339>/` keyed by run timestamp (preserves history)
- [ ] 4.2 Write the merged file as `consolidated_<short-hash>.md` in `memory/`
- [ ] 4.3 Regenerate `MEMORY.md` from the surviving files' frontmatter, one line per entry capped at 150 chars
- [ ] 4.4 `MEMORY.md` MUST keep its existing "no frontmatter, plain index" shape

## 5. CLI / wiring
- [ ] 5.1 `cortex memory consolidate [<project-slug>] [--dry-run] [--apply] [--threshold 0.78] [--max-clusters N]`
- [ ] 5.2 Default mode is `--dry-run`; `--apply` is required to mutate
- [ ] 5.3 Surface in `bin/cortex.bat` as a top-level subcommand
- [ ] 5.4 Cost ledger update on `--apply` (`classifier_spend.day`)

## 6. Spec / docs
- [ ] 6.1 Add §"Auto-memory consolidator" to `docs/specs/19-retention.md`
- [ ] 6.2 Update `bin/cortex.bat --help` text

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
