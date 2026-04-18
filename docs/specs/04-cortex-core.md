# 04 — Cortex Core (types, redactor, ingestion router)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 01, 02
>
> Implementation split across two crates: [`crates/cortex-core/`](../../crates/cortex-core/) (types + schema + validator + redactor, no runtime deps) and [`crates/cortex-ingestion/`](../../crates/cortex-ingestion/) (Axum router, Synap publisher via `synap-sdk`, Zstd-compressed NDJSON archive, Prometheus metrics). Separation keeps library consumers dep-light.

## Goal

Build the foundational Rust crate every other Cortex component links against: typed events, the redactor, and the ingestion router that accepts events and publishes them to Synap. This crate has zero knowledge of classifier/embedder/storage internals — it only validates, redacts, and routes.

## Scope

**In:**
- Rust workspace layout for `cortex-core`.
- Generated Rust types from spec 01 JSON Schemas.
- Validator (envelope + per-kind).
- Static-pattern redactor with versioned pattern catalog.
- Ingestion router: HTTP endpoint, validation, routing to `cortex.events.raw` or `cortex.events.bootstrap`, durable Parquet write, ack.
- Canonical-JSON helper for `content_hash`.
- ULID generator.
- Telemetry hooks (counters, latency histograms) emitted to `cortex.metrics`.

**Out:**
- Classifier (spec 05).
- Embedder (spec 06).
- Graph/full-text writers (specs 07, 08).
- Adapters (specs 10, 17).
- Query API (spec 11) — built as a separate crate that *uses* `cortex-core`.

## Inputs / Outputs

### Crate layout

```
cortex/
├─ Cargo.toml                  (workspace)
├─ cortex-core/
│  ├─ Cargo.toml
│  ├─ build.rs                 # generates events.rs from schemas
│  ├─ schemas/                 # source-of-truth JSON Schemas (spec 01)
│  ├─ src/
│  │  ├─ lib.rs
│  │  ├─ events.rs             # generated; do not edit
│  │  ├─ context.rs            # generated
│  │  ├─ canonical_json.rs
│  │  ├─ ulid.rs
│  │  ├─ validate.rs
│  │  ├─ redact/
│  │  │  ├─ mod.rs
│  │  │  ├─ patterns.rs        # versioned pattern catalog
│  │  │  └─ engine.rs
│  │  ├─ router/
│  │  │  ├─ mod.rs
│  │  │  ├─ http.rs            # axum routes
│  │  │  ├─ synap.rs           # publish
│  │  │  └─ archive.rs         # Parquet writer
│  │  ├─ metrics.rs
│  │  └─ error.rs
│  └─ tests/
│     └─ fixtures/             # event fixtures from spec 01
└─ cortex-api/                 # depends on cortex-core; spec 11 lives here
└─ cortex-workers/             # depends on cortex-core; specs 05–08 live here
```

### Public API

```rust
// re-exported types (generated)
pub use events::{Envelope, Context, Kind, ...};

// validation
pub fn validate(env: &Envelope) -> Result<(), ValidationError>;

// canonical JSON
pub fn canonical_json(value: &serde_json::Value) -> String;
pub fn content_hash(payload: &serde_json::Value) -> String;  // "sha256:..."

// ULID
pub fn new_ulid() -> String;

// redactor
pub struct Redactor { /* loaded patterns */ }
impl Redactor {
    pub fn load(path: &Path) -> Result<Self>;
    pub fn redact(&self, env: &mut Envelope) -> Vec<RedactionToken>;
}

// router (consumed by cortex-api)
pub struct IngestionRouter { /* synap, archive, metrics handles */ }
impl IngestionRouter {
    pub async fn accept(&self, env: Envelope) -> Result<AckResponse>;
}
```

### HTTP ingestion endpoint (cortex-api re-exports)

```
POST /api/v1/events
Content-Type: application/json
Authorization: Bearer <api-key>          # optional in localhost dev

Request body:
  Envelope (single) OR { "events": [Envelope, ...] } (batch up to 256)

Responses:
  200 OK
    { "accepted": [event_id, ...], "ingested_at": "..." }
  207 Multi-Status (partial batch failure)
    { "accepted": [...], "rejected": [{ event_id, reason }] }
  400 Bad Request    — schema violation
  413 Payload Too Large — > 1 MB envelope (spec 01 §Decisions #1)
  429 Too Many Requests — local backpressure (Synap stream lag exceeds threshold)
  500 Internal Server Error — Parquet write failed (events not acked)
```

Endpoint is the **single entry point**; adapters never write to Synap directly. This keeps redaction, validation, and archive guarantees centralized.

### Pattern catalog format

`cortex-core/redact-patterns/v1.yaml`:

```yaml
version: 1
patterns:
  - id: aws_access_key
    class: secret.cloud.aws
    regex: 'AKIA[0-9A-Z]{16}'
    case_sensitive: true
  - id: openai_api_key
    class: secret.api.openai
    regex: 'sk-[A-Za-z0-9]{32,}'
  - id: anthropic_api_key
    class: secret.api.anthropic
    regex: 'sk-ant-[A-Za-z0-9_-]{40,}'
  - id: github_pat
    class: secret.api.github
    regex: '(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}'
  - id: env_assignment
    class: secret.env
    regex: '(?m)^(?:export\s+)?([A-Z][A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD|PWD))\s*=\s*([^\s\n]+)'
    replace_group: 2
  - id: bearer_token
    class: secret.bearer
    regex: '(?i)Bearer\s+[A-Za-z0-9._\-+/=]{20,}'
  - id: pem_private_key
    class: secret.pem
    multiline: true
    regex: '-----BEGIN (RSA |EC |OPENSSH |DSA |)PRIVATE KEY-----[\s\S]*?-----END \1PRIVATE KEY-----'
  - id: jwt
    class: secret.jwt
    regex: 'eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+'
  - id: credit_card
    class: pii.cc
    regex: '\b(?:\d[ -]*?){13,19}\b'
    luhn_check: true
```

Catalog is **versioned**; bumping `version` requires re-running redaction over the Parquet archive. The current catalog version is recorded in every `RedactionToken` produced.

## Design

### Validation

Two-stage: serde deserialization (struct-shape) + JSON Schema validation (constraints, regex, enums) using `jsonschema` crate. Schema validation runs against the *original* JSON, not the deserialized struct, so regex constraints on string formats fire correctly.

```rust
pub fn validate(env_json: &Value) -> Result<Envelope, ValidationError> {
    static ENVELOPE_SCHEMA: Lazy<JSONSchema> = Lazy::new(|| compile(...));
    static KIND_SCHEMAS: Lazy<HashMap<Kind, JSONSchema>> = Lazy::new(|| ...);

    ENVELOPE_SCHEMA.validate(env_json)?;
    let env: Envelope = serde_json::from_value(env_json.clone())?;
    KIND_SCHEMAS[&env.kind].validate(&env_json["payload"])?;
    Ok(env)
}
```

Validation errors carry a JSON Pointer to the offending field (e.g., `/payload/touched/1/path`).

### Redaction algorithm

```
for each pattern in catalog (compiled once):
    for each string-typed leaf in envelope.payload:
        find all matches
        for each match:
            if pattern.replace_group is set:
                replace only that capture group with "<REDACTED:{pattern.id}>"
            else:
                replace the full match
            emit RedactionToken {
                class:    pattern.class,
                pattern:  pattern.id,
                locator:  json_pointer + offset + length
            }
```

Leaf walker uses `serde_json::Value` recursion; arrays/objects iterated; non-string leaves skipped. Regexes precompiled with `regex::RegexSet` for one-pass scanning per leaf.

**Order of operations:** redaction happens **after** schema validation but **before** `content_hash` finalization on the *redacted* payload. `content_hash` is then derived from the redacted payload — meaning identical pre-redaction inputs with different secrets still hash differently if and only if the secrets affected non-secret fields. (Most of the time, identical code/text → identical hash.)

> Note: spec 01 says `content_hash` is computed pre-redaction. **Resolved here:** we compute and ship two hashes — `content_hash` (pre-redaction, used for classifier cache) and `content_hash_redacted` (post-redaction, useful for "what's in storage" dedup). Spec 01 to be amended via PR alongside this spec.

### Ingestion router pipeline

For each event in the request:

```
1. Extract envelope JSON.
2. validate(env_json)?            -> 400 on failure
3. Enforce 1 MB cap                -> 413 on failure
4. Set ingested_at = now() (UTC).
5. Redactor.redact(&mut env)       -> populates env.redactions
6. Compute content_hash and content_hash_redacted.
7. Acquire archive write slot (batched, 250 ms windows or 64 events).
8. Write to Parquet (sync within batch).            // durability gate
9. Publish to Synap stream (raw or bootstrap).      // best-effort; replay from Parquet on loss
10. Increment metrics counters.
11. Append event_id to response 'accepted' list.
```

If step 8 fails, the entire batch is rejected with 500 — no partial durability.

If step 9 fails after 8 succeeded, the event is durably archived but won't reach workers immediately. A reconciliation job (`cortex-reconcile`) replays Parquet rows whose `event_id` is missing in Synap-confirmed sets every minute.

### Backpressure

The router subscribes to `cortex.metrics` for `synap.stream.<name>.lag`. When lag for `cortex.events.raw` exceeds **5 000 events**, the router returns 429 with a `Retry-After: 5` header. Adapters back off; the bootstrap stream is paused entirely (a Synap pub/sub `cortex.control.bootstrap.pause` flag is set).

### Metrics

Emitted to `cortex.metrics` every 10 s, scraped by the dashboard:

```
cortex.ingest.requests.total      counter, labels: stream, kind, status
cortex.ingest.duration_ms         histogram, labels: stream
cortex.ingest.batch_size          histogram
cortex.ingest.payload_bytes       histogram, labels: kind
cortex.redactor.matches.total     counter, labels: pattern_id, class
cortex.archive.write_duration_ms  histogram
cortex.synap.publish_duration_ms  histogram, labels: stream
cortex.errors.total               counter, labels: kind, code
```

### Errors

```rust
pub enum CoreError {
    Validation(JsonPointer, String),
    PayloadTooLarge { actual: usize, limit: usize },
    ArchiveWrite(io::Error),
    SynapPublish(SynapError),
    Backpressure { stream: String, lag: u64 },
    Internal(anyhow::Error),
}
```

All errors carry enough context to be actionable in logs without exposing payloads.

## Acceptance criteria

- [ ] `cargo build --workspace` succeeds; `events.rs` is generated from schemas.
- [ ] `cargo test -p cortex-core` passes:
  - validate accepts every fixture; rejects each malformed fixture with the right JSON Pointer.
  - canonical JSON is byte-stable across platforms.
  - redactor scrubs each pattern from a synthetic payload and emits a correct `RedactionToken`.
  - router round-trips a batch of 10 events into Parquet + Synap and acks each.
- [ ] HTTP endpoint passes a Postman/REST collection in CI: 200, 207, 400, 413, 429, 500 all reachable on demand.
- [ ] Backpressure trip: a stress test publishing 10k events with workers paused returns 429 within expected window.
- [ ] Reconciliation: kill Synap mid-batch; archived events appear in stream after `cortex-reconcile` runs.
- [ ] `cortex.metrics` shows non-zero counters after a 1k-event soak.
- [ ] Redactor pattern catalog v1 has unit tests for *every* pattern (positive + negative cases).

## Decisions

1. **Two hashes (`content_hash`, `content_hash_redacted`).** Pre-redaction enables classifier cache hits across redaction-catalog changes; post-redaction is what ends up in storage. Spec 01 amended.
2. **Schemas validated twice (serde + JSON Schema).** Belt-and-suspenders — serde catches structural issues with great error messages, JSON Schema catches enums/regex/range constraints serde doesn't enforce.
3. **Parquet write is the durability gate, not Synap publish.** Streams are best-effort; the archive is authoritative.
4. **Pattern catalog is YAML, not Rust.** Operators (and the Haiku classifier!) can add patterns without recompiling. Each catalog version is hashed and recorded in `RedactionToken`.
5. **Router is HTTP-only at v1.** No gRPC, no MCP; adapters POST JSON. Simplest possible interface; we revisit if throughput demands binary.

## Open questions

*(none — defaults locked)*

## References

- Spec 01 — Event schema (consumes; amends pre-/post-redaction hash policy).
- Spec 02 — Storage layout (Synap streams, Parquet layout, metadata DB).
- Spec 03 — Local stack (deploys this binary as `cortex-api`).
- Spec 05 — Classifier (consumes events from `cortex.events.raw`).
- Architecture §5.1 (capture), §5.2 (processing), §9 (privacy), §11 (resource budget).
