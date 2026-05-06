## 1. Type generation
- [ ] 1.1 Add `ts-rs` (or equivalent) as a dev-dependency on `cortex-api`.
- [ ] 1.2 Add `#[derive(TS)]` on every type that crosses the HTTP boundary: `QueryRequest`, `QueryResponse`, `RetentionStateBody`, `ConsolidationsStateBody`, etc.
- [ ] 1.3 Build script `scripts/generate-gui-types.{sh,ps1}` runs `cargo test --features ts-export -p cortex-api` and copies the emitted `.ts` files to `gui/src/lib/api.generated.ts`.
- [ ] 1.4 `gui/src/lib/api.ts` re-exports from `api.generated.ts` plus any client-only utility types.

## 2. Contract diff
- [ ] 2.1 New `pnpm -C gui run check-contract` that re-runs the build script and asserts `git diff --exit-code gui/src/lib/api.generated.ts`.
- [ ] 2.2 If non-zero exit, the developer must regenerate or accept the change.

## 3. CI gate
- [ ] 3.1 New CI step `gui-contract`: runs `pnpm -C gui run check-contract`. Fails the job on diff.
- [ ] 3.2 Document in `.github/workflows/ci.yml` + `docs/specs/21-dashboard.md` + CONTRIBUTING.md.

## 4. Tail (mandatory)
- [ ] 4.1 Update `docs/specs/21-dashboard.md` + `CHANGELOG.md`.
- [ ] 4.2 Tests: regenerate types, modify one Rust type, confirm CI gate fires.
- [ ] 4.3 `cargo check --workspace && cargo clippy -- -D warnings && pnpm -C gui run check-contract && pnpm -C gui test` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [ ] 99.1 Update or create documentation covering the implementation.
- [ ] 99.2 Write tests covering the new behavior.
- [ ] 99.3 Run tests and confirm they pass.
