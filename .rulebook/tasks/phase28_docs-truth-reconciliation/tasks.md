## 1. Renumbering & cross-reference cleanup
- [ ] 1.1 Rename `docs/specs/20-opencode-adapter.md` → `docs/specs/23-opencode-adapter.md`; update its `# NN — Title` header
- [ ] 1.2 Rename `docs/specs/26-eval.md` → `docs/specs/29-eval.md`; update its header
- [ ] 1.3 Rename `docs/specs/27-retrieval-rerank.md` → `docs/specs/37-retrieval-rerank.md` (fewer inbound references than `27-consolidation.md`); update its header
- [ ] 1.4 Rename `docs/specs/28-gui-contract.md` → `docs/specs/38-gui-contract.md` (fewer inbound references than `28-phantom-link-verifier.md`); update its header
- [ ] 1.5 Grep `docs/`, `crates/`, `.rulebook/` for every inbound reference to the four old filenames and update each to the new filename
- [ ] 1.6 Update `docs/specs/00-index.md`: replace the "Known numbering collisions" callout with a resolved-changelog note and fix the table rows for the four moved specs
- [ ] 1.7 Add an automated check (script or `cortex-ops doctor` check) that fails when two files in `docs/specs/` share a leading number

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
