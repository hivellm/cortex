## 1. Audit
- [ ] 1.1 For each content-addressable index (`cortex_laws`/governance, per-repo `misc` for knowledge+learning, bootstrap `code`/`docs`), count docs whose id is NOT `bootstrap-`-keyed (stale legacy) vs canonical, mirroring the decisions audit
- [ ] 1.2 Confirm the source of truth for each kind (`.rulebook/knowledge`, `.rulebook/learnings`, `.claude/rules`/laws, repo files for artifacts) and the re-emit path

## 2. Reindex + prune
- [ ] 2.1 Generalise `decisions-reindex` (or add per-kind reindex subcommands) to re-emit each kind through the builder with the stable `bootstrap-` key and prune legacy non-`bootstrap-` docs (guarded, `--dry-run`)
- [ ] 2.2 Run the reindex live for each kind and verify the index collapses to the canonical set with zero non-`bootstrap-` docs
- [ ] 2.3 Extend `doctor-decisions` (or a generalised doctor) to flag non-`bootstrap-` content-addressable docs across all affected indexes

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation (spec 08 per-kind reindex contract; CHANGELOG)
- [ ] 3.2 Write tests covering the new behavior (per-kind reindex unit tests)
- [ ] 3.3 Run tests and confirm they pass (`cargo check` + `clippy -D warnings` + `cargo test --workspace`)
