# cortex-classifier-worker

The bridge between **raw / bootstrap** event streams and the **enriched**
stream the embedder, graph writer, and full-text indexer all consume.

```
cortex.events.raw  ──┐
                     ├──> cortex-classifier-worker ──> cortex.events.enriched
cortex.events.bootstrap ─┘
```

Spec 05 (`docs/specs/05-classifier.md`) shipped the classifier library —
`StaticClassifier`, `CachedClassifier`, `BudgetedClassifier`,
`HaikuCliClassifier`, the prompt template, and the budget tracker — but
explicitly deferred the worker binary to a follow-up pass. This crate is
that follow-up: the binary that actually drains the input streams,
classifies each envelope through a configurable stack, and publishes the
result on `cortex.events.enriched`.

## Why a separate crate

`cortex-embedder` already depends on `cortex-classifier` for
`ClassifierOutput`. The worker needs to publish the same `EnrichedEvent`
struct the embedder consumes, so it depends on `cortex-embedder`. Putting
the worker inside `cortex-classifier` would create a `classifier ->
embedder -> classifier` cycle. A standalone crate keeps both crates pure
libraries and the worker dedicated to the wire bridge.

## Modes

`CORTEX_CLASSIFIER_MODE=static` (default)
> Pure-rust deterministic fallback. No network, no API key, free.
> Output uses the topic vocab and severity rules baked into
> `cortex-classifier::StaticClassifier`. Suitable for local development,
> CI, and any environment where the LLM cost is unacceptable.

`CORTEX_CLASSIFIER_MODE=cli`
> Spawns `claude -p ... --output-format json` per batch via
> `HaikuCliClassifier`. Requires the Claude Code CLI on `PATH`
> (configurable via `CLAUDE_CODE_BIN`). The cache + budget tracker
> automatically degrade to `StaticClassifier` once daily spend crosses
> `CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS`.

## Configuration

| Variable | Default | Notes |
|---|---|---|
| `CORTEX_CLASSIFIER_SYNAP_URL` | `http://127.0.0.1:17003` | Synap base URL. Falls back to `SYNAP_URL`. |
| `CORTEX_CLASSIFIER_MODE` | `static` | `static` or `cli`. |
| `CORTEX_CLASSIFIER_WORKERS` | `2` | Concurrent pull tasks. |
| `CORTEX_CLASSIFIER_BATCH` | `32` | Max messages per pull. |
| `CORTEX_CLASSIFIER_DAILY_LIMIT_CENTS` | `2000` | Halts to static fallback above this. |
| `CORTEX_CLASSIFIER_PROMPT_VERSION` | `static-v1` | Stamped on every output. |
| `CORTEX_CLASSIFIER_MODEL` | `claude-haiku-4-5` | Used in `cli` mode. |
| `CLAUDE_CODE_BIN` | `claude` | Path to the CLI binary. |

## Run

Local stack must be up (`bin/cortex-up`) so Synap is reachable.

```bash
# offline mode (no LLM cost, deterministic)
CORTEX_CLASSIFIER_MODE=static \
  cargo run --release -p cortex-classifier-worker

# Haiku-via-CLI mode (requires `claude` on PATH)
CORTEX_CLASSIFIER_MODE=cli \
  cargo run --release -p cortex-classifier-worker
```

Once the worker is up, publish into the input streams the usual way:
- `cortex-bootstrap <repo>` for backfill
- `cortex-ingestion` HTTP API for live capture

The worker auto-creates Synap rooms it needs to publish to (lazy
`stream.create` on the first "Room not found"), so the order of startup
between bootstrap, classifier worker, and downstream workers does not
matter.

## Behaviour

For each consumed envelope:

1. **Normalise** — bootstrap envelopes (kind strings like `artifact.code`)
   and canonical envelopes (`cortex_core::events::Envelope`) both get
   collapsed to a common shape with `Kind`, `event_id`, `content_hash`,
   `redacted_payload`, `context_repo`, `context_path`,
   `parent_event_id`.
2. **Dedup** — `event_id` is checked against an in-memory set. Replays
   within the worker lifetime are acked without re-publishing.
3. **Classify** — the input goes through the configured
   `ClassifierStack` (`Budgeted ← Cached ← StaticClassifier` or
   `Budgeted ← Cached ← HaikuCliClassifier`). Backend errors fall back
   to a deterministic static record so the pipeline never stalls on
   transient classifier failures.
4. **Publish** — an `EnrichedEvent` (matching
   `cortex_embedder::EnrichedEvent`) is written to
   `cortex.events.enriched`. The source message is acked only after the
   publish succeeds, giving at-least-once delivery downstream.

## Stream contract

- **Inputs:** `cortex.events.raw`, `cortex.events.bootstrap`
- **Output:** `cortex.events.enriched`
- **Envelope shape on enriched:** `cortex_embedder::EnrichedEvent`
  (re-exported by both `cortex-embedder` and the graph + fulltext crates)

## Tests

Integration tests in `tests/worker.rs` drive the worker with the
in-memory consumer/publisher and cover every requirement in the spec:

- bootstrap envelope -> enriched envelope
- canonical envelope -> enriched envelope
- replay dedup
- budget halt forces static fallback

Unit tests in `src/kinds.rs` cover the bootstrap-kind -> canonical-Kind
mapping for every bootstrap kind string.

```bash
cargo test -p cortex-classifier-worker
```
