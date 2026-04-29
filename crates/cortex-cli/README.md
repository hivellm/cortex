# cortex-cli

> Specs: [`09-bootstrap-cli.md`](../../docs/specs/09-bootstrap-cli.md),
> [`02-storage-layout.md`](../../docs/specs/02-storage-layout.md),
> [`11-query-api.md`](../../docs/specs/11-query-api.md)

Cortex's operator-facing CLIs in a single crate. Consolidates what used to
be three separate binaries (`cortex-bootstrap`, `cortex-ops`,
`cortex-relevance-eval`) so they share one dependency graph and one set of
SDK clients.

If a script or runbook used to call `cortex_bootstrap::Walker`, the import
is now `cortex_cli::bootstrap::Walker` — the public API is preserved.

## Binaries

| Binary                    | Module          | Purpose                                                                                  |
|---------------------------|-----------------|------------------------------------------------------------------------------------------|
| `cortex-bootstrap`        | `bootstrap`     | Walks the 17 Hive repos and republishes existing content on `cortex.events.bootstrap`.   |
| `cortex-ops`              | `ops`           | Operator workflows: layout `plan`, cross-backend `doctor`, retention sweeps, seed jobs.  |
| `cortex-relevance-eval`   | `relevance_eval`| Recall@k / MRR harness against `cortex-api` (phase6e / F-008).                           |

## Modules

- **`bootstrap`** — repo walker, checkpoint store, Synap publisher,
  preflight + sizing estimator. Reads per-repo config from
  `bootstrap/<repo>.toml` and emits one `cortex.events.bootstrap` per file.
- **`ops`** — `plan` serializes the storage layout declared in
  `cortex-storage`; `doctor` cross-checks Vectorizer / Nexus / Meilisearch
  / Synap against that plan and reports drift; retention helpers drive the
  sweepers in `cortex-retention`.
- **`relevance_eval`** — runs a fixed query set against the live
  `cortex-api`, computes recall@k and MRR, and writes a JSON report for
  regression tracking.

## Usage

```bash
# Bootstrap all repos listed in bootstrap/workspace.toml
cargo run -p cortex-cli --bin cortex-bootstrap -- --workspace bootstrap/workspace.toml

# Plan + doctor
cargo run -p cortex-cli --bin cortex-ops -- plan   --slice all
cargo run -p cortex-cli --bin cortex-ops -- doctor --slice all

# Relevance harness
cargo run -p cortex-cli --bin cortex-relevance-eval -- \
    --queries tests/fixtures/queries.jsonl \
    --api-url http://localhost:7411
```

As a library:

```rust
use cortex_cli::bootstrap::{Walker, RunnerConfig};
use cortex_cli::ops::{plan, doctor};
```

The crate is `#![forbid(unsafe_code)]`.

## Configuration

`cortex-bootstrap` reads:

| Variable                  | Notes                                                       |
|---------------------------|-------------------------------------------------------------|
| `SYNAP_URL`               | Target Synap instance (default `http://localhost:7401`).    |
| `CORTEX_BOOTSTRAP_CHECKPOINT` | Override path for the resume checkpoint file.           |

`cortex-ops` and `cortex-relevance-eval` read the same backend URLs
exposed by `cortex-core::config`.

## Testing

```bash
cargo test -p cortex-cli
```

Integration tests cover walker resumption, doctor drift detection, and the
relevance JSON contract. SDK calls are stubbed via in-memory fakes so the
suite stays hermetic.

## Stability

Pre-1.0 — adding a new subcommand or binary is a minor change; renaming or
removing one is breaking for the operator scripts that call them.
