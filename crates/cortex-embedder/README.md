# cortex-embedder

> Spec: [`docs/specs/06-embedder.md`](../../docs/specs/06-embedder.md)

The Cortex embedding worker. Consumes enriched events from
`cortex.events.enriched`, splits payloads into chunks (Tree-sitter for
code, section parser for docs, sliding window fallback), and upserts
vectors into the **Vectorizer** service via `vectorizer-sdk`.

The crate ships:

- A library with the chunkers, routing rules, and a `VectorizerClient`
  abstraction.
- A binary, `cortex-embedder-worker`, that runs the consumer loop.

## Design

- The Vectorizer service owns the model and the index. This crate is a
  **client**; it never embeds locally.
- Each chunk carries a deterministic `dedup_key = sha256(canonical)` so
  re-runs are idempotent. The Vectorizer assigns its own primary UUID.
- Routing decides which collection a chunk lands in based on
  `(kind, language, source)`; the rules live in `routing::collection_for`
  and the catalog itself comes from `cortex-storage`.

```
Synap (cortex.events.enriched)
        │
        ▼
   Chunker (code | doc | fallback)
        │
        ▼
   VectorizerClient.upsert(...)
        │
        ▼
   Vectorizer collection (per cortex-storage)
```

## Chunkers

| Chunker          | Selection criteria                                     |
|------------------|--------------------------------------------------------|
| `CodeChunker`    | `kind=artifact` and language in the Tree-sitter set.   |
| `DocChunker`     | Markdown / RST / plaintext docs and rationales.        |
| `FallbackChunker`| Anything else — overlapping sliding window.            |

Tree-sitter languages bundled today: Rust, TypeScript, JavaScript,
Python, Go, Java, C, C++, Markdown, JSON, YAML, TOML.

## Worker

```bash
cargo run --release -p cortex-embedder --bin cortex-embedder-worker
```

The worker subscribes to the enriched stream, fans events out to the
right chunker, retries upserts with exponential backoff
(`vectorizer_client::with_retry`), and emits Prometheus metrics.

## Library

```toml
[dependencies]
cortex-embedder = { path = "../cortex-embedder" }
```

```rust
use cortex_embedder::{VectorizerEmbedder, EmbedderConfig};

let config   = EmbedderConfig::from_env()?;
let embedder = VectorizerEmbedder::connect(&config).await?;
let report   = embedder.embed(enriched_event).await?;
println!("upserted={} skipped={}", report.upserted, report.skipped);
```

The crate is `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Configuration

| Variable                          | Default                       | Notes                                  |
|-----------------------------------|-------------------------------|----------------------------------------|
| `CORTEX_EMBEDDER_VECTORIZER_URL`  | `http://localhost:15001`      | Vectorizer base URL.                   |
| `CORTEX_EMBEDDER_SYNAP_URL`       | `http://localhost:18443`      |                                        |
| `CORTEX_EMBEDDER_CONSUMER_GROUP`  | `cortex-embedder`             | Synap consumer group identity.         |
| `CORTEX_EMBEDDER_BATCH`           | `64`                          | Max chunks per upsert call.            |
| `CORTEX_EMBEDDER_RETRY_MAX`       | `5`                           | Upsert retry budget per chunk.         |

## Testing

```bash
cargo test -p cortex-embedder
```

Unit tests cover each chunker. Integration tests use `wiremock` to stand
up a fake Vectorizer and assert idempotency and retry semantics.
