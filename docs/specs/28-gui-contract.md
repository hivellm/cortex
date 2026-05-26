# Spec 28 — GUI ↔ Rust contract test

Status: **active** (phase14d)
Authors: phase14d_gui-rust-contract-test.

`gui/src/lib/api.ts` mirrors Rust route signatures. Drift was silent: the GUI compiled fine when a Rust handler changed shape, then crashed at runtime against the live API. The phase14a `Consolidations` daemon-health panel surfaced this drift in dashboard review.

Phase14d closes the loop with a build-time codegen pipeline + a CI diff gate.

## §1 Pipeline

```
Rust wire type → #[derive(ts_rs::TS)]
              → cargo test --features ts-export
              → scripts/generate-gui-types.sh bundles emitted .ts
              → gui/src/lib/api.generated.ts
              → gui/src/lib/api.ts re-exports symbols
              → CI fails any PR whose generated bundle drifts
```

The pipeline is one-directional: Rust is the source of truth. The generated bundle is committed to git so consumers can read it without re-running the codegen.

## §2 Adding a new wire type

1. On the Rust type's declaration, add:
   ```rust
   #[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
   #[cfg_attr(
       feature = "ts-export",
       ts(export, export_to = "../../gui-types/")
   )]
   ```
2. Add the type to [`crates/cortex-api/src/ts_export.rs::export_all_wire_types`].
3. Regenerate:
   ```bash
   bash scripts/generate-gui-types.sh
   ```
4. Update `gui/src/lib/api.ts` if the symbol should be re-exported under that module.
5. Commit `gui/src/lib/api.generated.ts` alongside the Rust change.

## §3 Local check

```bash
pnpm -C gui run check-contract
```

Runs the regen script and asserts `git diff --exit-code gui/src/lib/api.generated.ts`. Exit 0 = no drift; non-zero = the operator must regenerate or accept the change.

## §4 CI gate

`.github/workflows/gui-contract.yml` runs `pnpm -C gui run check-contract` on every PR touching `crates/cortex-api/**`, the generated bundle, the api.ts shim, or the gen script itself. Failure modes:

- **Type added in Rust but bundle stale**: CI diff fails, operator runs the regen + commits.
- **Type removed in Rust but bundle still carries it**: same diff failure.
- **Type renamed in Rust**: same diff failure.

Job also runs `pnpm run typecheck` so any GUI consumer that references a removed/renamed symbol surfaces at the same gate.

## §5 Initial coverage

`ts_export::export_all_wire_types` carries the following types as of phase14d:

| Type | Source | Used by |
|---|---|---|
| `Severity` | `cortex_api::health::Severity` | freshness + divergence panels |
| `FreshnessRow` | `cortex_api::health::FreshnessRow` | `/v1/health/freshness` |
| `GrainHealth` | `cortex_api::health::consolidator::GrainHealth` | `/v1/health/consolidator` |
| `ConsolidatorHealthReport` | same module | `/v1/health/consolidator` |
| `ConsolidationFilter` | `cortex_api::dashboard::ConsolidationFilter` | `/v1/dashboard/consolidations` |

Operator extends the list per the §2 contract. Every PR that adds a wire-crossing type SHOULD also derive `TS` on it.

## §6 Known limitations

- `ts-rs` v10 does not parse `#[serde(skip_serializing_if = "Option::is_none")]` perfectly — the generated bundle emits `T | null` instead of `T | undefined`. Manual narrowing in the GUI consumer is currently the workaround. Bumping to ts-rs v12 (with `serde-compat` improvements) is a forward-compat task.
- Types with non-`TS`-implementing fields (e.g. `axum::body::Body`) can't be derived. Refactor wire types into pure data structs that the handler builds from internal state.
- The bundle uses one big file rather than per-type files to keep `git diff` reviewable. A future task can split it if the bundle grows past a few hundred symbols.
