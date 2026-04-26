# cortex-graph

> Spec: [`docs/specs/07-graph-writer.md`](../../docs/specs/07-graph-writer.md)

The Cortex graph writer. Consumes enriched events from
`cortex.events.enriched`, maps them onto the Cortex graph schema
(architecture §4.2), coalesces redundant node and edge upserts within a
micro-batch, and writes them to **Nexus** through the official
`nexus-graph-sdk`.

The crate is a **client**. It never owns the graph and never re-implements
Cypher beyond the parameterized templates in `cypher::CypherTemplates`.

## Pieces

| Module          | Role                                                                                        |
|-----------------|---------------------------------------------------------------------------------------------|
| `mapper`        | Enriched event → Cortex graph patches (nodes + edges).                                      |
| `coalescer`     | Folds duplicate node/edge MERGEs in the current batch (`PatchCoalescer`).                   |
| `cypher`        | Parameterized Cypher templates (constraints, indexes, MERGE statements).                    |
| `schema`        | Label set, edge types, and required-property checks shared with `cortex-storage::graph`.    |
| `identity`      | Deterministic node IDs so MERGE is idempotent across re-runs.                               |
| `nexus_client`  | Thin async client around `nexus-graph-sdk` (`nexus_sdk` library name).                      |
| `writer`        | Glues mapper → coalescer → client; one entry point for tests.                               |
| `worker`        | Synap consumer loop, batching, retries, metrics.                                            |
| `patch`         | Patch types (node upsert, edge upsert) used between mapper and writer.                      |

## Transport

The architecture spec calls out **Bolt**. In practice Nexus exposes its
own RPC over `nexus://` URLs (selected via `ClientConfig.transport` or
`NEXUS_SDK_TRANSPORT`) plus an HTTP fallback. This crate treats those two
transports as the equivalents of "Bolt vs HTTP" in the spec; choose with
`GraphTransport` in `GraphConfig`.

## Worker

```bash
cargo run --release -p cortex-graph --bin cortex-graph-worker
```

The worker subscribes to the enriched stream, builds patch batches per
event, coalesces them, and applies them through the Nexus client with
retry. Prometheus metrics expose batch sizes, MERGE counts, and per-edge
latencies.

## Library

```toml
[dependencies]
cortex-graph = { path = "../cortex-graph" }
```

```rust
use cortex_graph::{GraphConfig, writer::GraphWriter};

let config = GraphConfig::from_env()?;
let writer = GraphWriter::connect(&config).await?;
writer.apply(enriched_event).await?;
```

The crate is `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Schema

Labels and edge types are declared in `schema` and bootstrapped through
`cortex-storage::graph` (constraints + indexes). Anything declared there
is what `cortex-ops plan --slice nexus` emits — the writer never invents
labels or constraints on the fly.

## EnrichedEvent reuse

`EnrichedEvent` is re-exported from `cortex-embedder` so both workers
agree on the post-enrichment payload of `cortex.events.enriched`. Changes
to that type require coordinated bumps across both crates.

## Configuration

| Variable                       | Default                       | Notes                                          |
|--------------------------------|-------------------------------|------------------------------------------------|
| `CORTEX_GRAPH_NEXUS_URL`       | `http://localhost:7474`       | Nexus base URL (HTTP or `nexus://...`).        |
| `CORTEX_GRAPH_SYNAP_URL`       | `http://localhost:18443`      |                                                |
| `CORTEX_GRAPH_CONSUMER_GROUP`  | `cortex-graph`                | Synap consumer group identity.                 |
| `CORTEX_GRAPH_BATCH`           | `128`                         | Max patches per Nexus round-trip.              |
| `NEXUS_SDK_TRANSPORT`          | _SDK default_                 | `nexus` or `http`.                             |

## Testing

```bash
cargo test -p cortex-graph
```

Unit tests cover the mapper and coalescer. Integration tests use
`wiremock` to stand up a fake Nexus and assert idempotency, retry, and
schema invariants.
