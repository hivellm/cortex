## 1. Canonicalize at emission
- [ ] 1.1 In `crates/cortex-cli/src/bootstrap/walker.rs`, lowercase `repo` before publishing every envelope
- [ ] 1.2 The Capitalised display name flows separately as `repo_label` (display string only)
- [ ] 1.3 Adapter ingestion (`crates/cortex-adapter-claude-code`) applies the same rule on the IPC path

## 2. Read-side normalization
- [ ] 2.1 In `crates/cortex-api/src/dashboard.rs`, every handler that returns `repo` lowercases the value before serialization
- [ ] 2.2 New `repo_label` field carries the original-case string the GUI shows
- [ ] 2.3 The orchestrator's scope check compares lowercase ↔ lowercase; uppercase scope strings still match (case-insensitive)

## 3. One-shot canonicalize CLI
- [ ] 3.1 NEW `cortex-ops repo-canonicalize [--dry-run] [--apply]`
- [ ] 3.2 Migrates Vectorizer payload `repo` field, Meili documents, Nexus `:Repo`/`:Session` properties, SQLite `sessions.repo`, `bootstrap_jobs.repo_path`
- [ ] 3.3 Default is dry-run; report shows per-store rewrite counts

## 4. Tests
- [ ] 4.1 Unit test asserting `lowercase(repo)` emission across walker + adapter
- [ ] 4.2 Integration test: scope `repo: "Cortex"` returns the same rows as `repo: "cortex"`
- [ ] 4.3 Regression: re-run the relevance harness with the canonical fixture and assert zero buckets are omitted

## 5. Fixture + spec updates
- [ ] 5.1 Lowercase every `repo` in `tests/relevance/queries.toml`
- [ ] 5.2 Update `docs/specs/02-storage-layout.md` §naming and `docs/specs/11-query-api.md` §scope to mandate lowercase

## 6. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 6.1 Update or create documentation covering the implementation
- [ ] 6.2 Write tests covering the new behavior
- [ ] 6.3 Run tests and confirm they pass
