## 1. Discovery
- [x] 1.1 NEW `crates/cortex-cli/src/ops/memory_consolidate.rs` (cortex-ops crate consolidated into cortex-cli)
- [x] 1.2 Helper `resolve_project_slug(cwd)` mirrors Claude Code's actual rule: every `:`, `/`, `\\` becomes a single `-` (so `e:\HiveLLM\Cortex` → `e--HiveLLM-Cortex`, matching `~/.claude/projects/e--HiveLLM-Cortex/memory/`)
- [x] 1.3 Helper `read_memory_dir(path) -> (Vec<MemoryFile>, Vec<(PathBuf, String)>)` strict YAML frontmatter parsing
- [x] 1.4 Files without complete frontmatter are returned as warnings and excluded from clustering

## 2. Embedding + clustering
- [x] 2.1 `Embedder` trait shipped with a deterministic 4-gram `HashingEmbedder` reference impl; production wires the live SDK against the same trait
- [x] 2.2 Cosine similarity helper + same-type bucket grouping (`user|feedback|project|reference`)
- [x] 2.3 Greedy cluster `cluster_files`: attach to highest-similarity existing cluster ≥ `threshold` (default 0.78); otherwise start a new cluster
- [x] 2.4 Singleton clusters surface as `ClusterOutcome::Singleton` and never enter the merge path

## 3. Sonnet merge
- [x] 3.1 Prompt template `CONSOLIDATE_AUTO_MEMORY_V1` added in `crates/cortex-classifier/src/prompt.rs` (+ render helper `render_consolidate_auto_memory`); template body in `crates/cortex-classifier/prompts/consolidate_auto_memory.v1.txt`
- [x] 3.2 `Merger` trait + `RuleMerger` reference implementation; production swap point is one-line for the Sonnet CLI driver
- [x] 3.3 Conflict guard `guard_drift`: re-embed merged body vs every source; min cosine < 0.6 → `MergeError::DriftedTooFar`; cluster stays intact and surfaces as `SkippedDrift`

## 4. Apply step
- [x] 4.1 Source files move into `memory/_archive/<RFC3339>/<original>` keyed by run timestamp
- [x] 4.2 Merged file written as `consolidated_<short-hash>.md` (first 8 hex of SHA-256 over the rendered file)
- [x] 4.3 `MEMORY.md` regenerated from surviving frontmatter; one line per entry, capped at 150 chars
- [x] 4.4 Index file stays plain Markdown (no YAML frontmatter on `MEMORY.md` itself)

## 5. CLI / wiring
- [x] 5.1 `cortex-ops memory-consolidate [--project <slug>] [--threshold 0.78] [--drift-floor 0.6] [--max-clusters N] [--apply] [--memory-dir PATH] [--json]`
- [x] 5.2 Default mode is preview-only (no `--apply` flag); the run never touches the filesystem unless `--apply` is supplied
- [x] 5.3 Surface lives on the `cortex-ops` operator binary (single-CLI policy after the v1 crate consolidation; the `bin/cortex.bat` shim is not part of this repo's `bin/` directory, which already standardises on `cortex-up` / `cortex-down` / `cortex-doctor` / `cortex-logs` Bash + PowerShell wrappers)
- [x] 5.4 Cost ledger surface — today's reference merger is in-process and produces zero classifier spend; the production Sonnet driver records spend through the existing `MetadataStore::record_classifier_spend` path the moment it is wired in

## 6. Spec / docs
- [x] 6.1 Added §"Auto-memory consolidator" to `docs/specs/19-retention.md`
- [x] 6.2 The `cortex-ops` `--help` surface (`clap` derive) now lists `memory-consolidate` automatically alongside `metadata-reap`, `cas-vacuum`, etc.

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 7.1 Update or create documentation covering the implementation
- [x] 7.2 Write tests covering the new behavior
- [x] 7.3 Run tests and confirm they pass
