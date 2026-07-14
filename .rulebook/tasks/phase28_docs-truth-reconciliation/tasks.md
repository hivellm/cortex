## 1. Renumbering & cross-reference cleanup
- [x] 1.1 `git mv` 20-opencode-adapter.md → 23-opencode-adapter.md; header `# 20 —` → `# 23 —`
- [x] 1.2 `git mv` 26-eval.md → 29-eval.md; header updated
- [x] 1.3 `git mv` 27-retrieval-rerank.md → 37-retrieval-rerank.md; header updated
- [x] 1.4 `git mv` 28-gui-contract.md → 38-gui-contract.md; header updated
- [x] 1.5 All inbound references rewritten (README, crate READMEs ×2, CHANGELOG links, cortex-eval golden `retrieval.csv` expected-path, live task files phase28_retrieval-eval-gate-live + phase33, and 12 `.rulebook/archive/**` docs); post-sweep grep finds zero references to the old names outside this task's own files (which document the rename)
- [x] 1.6 00-index.md: collisions callout → "Numbering changelog" resolved note; the four rows moved to their new numbers (sorted in place, "(was NN)" note kept); missing row for `45-graph-communities.md` also added while reconciling
- [x] 1.7 Two automated checks: `crates/cortex-cli/tests/spec_numbering.rs` (fails `cargo test` on any duplicate leading number) + `scripts/check-spec-numbering.sh` (shell-native for CI/pre-commit) — both verified failing-free against the renamed tree

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 2.1 Docs: 00-index numbering changelog note; the four renamed specs' own headers; CHANGELOG entry
- [x] 2.2 Tests: `spec_numbering.rs` (the §1.7 gate is itself the test — asserts non-empty scan + zero duplicates)
- [x] 2.3 Verified: `bash scripts/check-spec-numbering.sh` → ok; `cargo test -p cortex-cli --test spec_numbering` 1/1; full cortex-cli suite green
