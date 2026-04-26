# cortex-core

> Spec: [`docs/specs/01-event-schema.md`](../../docs/specs/01-event-schema.md), [`docs/specs/04-cortex-core.md`](../../docs/specs/04-cortex-core.md)

The foundation crate for every Cortex component. Defines the on-the-wire event
contract, the canonical encoding used to compute stable identities, and the
validator that every adapter and worker uses before publishing.

## Scope

- **Event schema** (`events`) — Rust mirrors of the JSON Schemas under
  [`schemas/`](schemas/). The schemas are the source of truth; the Rust types
  are tested against them so neither side can drift.
- **Canonical JSON** (`canonical_json`) — RFC 8785-style canonicalization used
  for content hashing.
- **Content hash** (`content_hash`) — SHA-256 of canonical JSON; the basis for
  the CAS store and for de-duplication.
- **Identifiers** (`ids`) — ULID-based `EventId` / `SessionId` with strict
  parsing.
- **Redaction** (`redact`) — pattern catalog v1 (PII / secrets) used by the
  ingestion service before any payload touches disk.
- **Validator** (`validate`) — single entry point for envelope and payload
  validation against the bundled JSON Schemas.
- **Vocab** (`vocab`) — frozen enum vocabularies (kinds, severities,
  PII-risk levels) shared with the classifier.

## Library

```toml
[dependencies]
cortex-core = { path = "../cortex-core" }
```

```rust
use cortex_core::{validate_envelope, content_hash, event_id};

let envelope = serde_json::from_str(raw)?;
validate_envelope(&envelope)?;
let id = event_id();
let hash = content_hash(&envelope)?;
```

The crate is `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Binary

`cortex-core` ships a thin CLI used by tests, hooks, and the bootstrap script:

```bash
cortex-core validate path/to/event.json
cortex-core hash    path/to/event.json
cortex-core redact  path/to/event.json
```

See `src/bin/cli.rs` for the full subcommand list.

## Schemas

JSON Schemas under [`schemas/`](schemas/) are the public wire contract:

- `envelope.schema.json` — the outer envelope every event carries.
- One schema per payload kind (turn, tool-call, agent-call, decision,
  artifact, memory, analysis, law-violation).

Adapters in other languages should pull these schemas directly rather than
re-deriving them.

## Testing

```bash
cargo test -p cortex-core
```

The test suite cross-checks that every Rust type round-trips through its JSON
Schema and that every redaction pattern in `PATTERN_CATALOG_V1` has a positive
and negative case.

## Stability

Pre-1.0 — schemas are still allowed to evolve. Once Cortex hits its first
public release the envelope and payload schemas become the ABI for every
adapter, worker, and dashboard.
