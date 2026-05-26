# Proposal: phase1_bootstrap-cli

## Why

Day-1 retrieval must already be useful. The live capture layer only sees *new* AI interactions, but our institutional knowledge is already in 17 existing HiveLLM repos. This task delivers the CLI that walks each repo, synthesizes envelope-compliant events for source code, docs, git history, ADRs, laws, and memories, and publishes them to `cortex.events.bootstrap` — driving the same live pipeline for the backfill.

## What Changes

- `cortex-bootstrap` crate (Rust + Clap) with per-repo config (`cortex.toml`) + global defaults.
- File walker (`ignore` crate; `.gitignore`-aware), git log walker, ADR / OpenSpec / law / memory recognizers.
- Synthetic event emitters (kinds: `artifact.code`, `artifact.doc`, `turn.historical`, `decision.imported`, `law.imported`, `memory.imported`).
- Redaction pass before publication (uses `cortex-core` redactor).
- Checkpoint file with atomic write-rename; `--resume` support.
- `--dry-run --estimate` mode printing sizing (files, chunks, bytes, est cost).
- Parallel repo walkers (`--parallelism N`).

## Impact

- **Affected specs:** [`docs/specs/09-bootstrap-cli.md`](../../../docs/specs/09-bootstrap-cli.md).
- **Affected code:** new `cortex-bootstrap/` crate with bin `cortex-bootstrap`.
- **Breaking change:** NO — greenfield.
- **User benefit:** the very first pre-thinking query in a bootstrapped repo returns meaningful context; incremental re-runs are free.

## Source

`docs/specs/09-bootstrap-cli.md` · depends on specs 04, 05, 06, 07, 08 · PRD FR-9.
