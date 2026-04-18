# 01 — Event Schema (wire format)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** —
>
> Implementation: [`crates/cortex-core/`](../../crates/cortex-core/). Schemas under `crates/cortex-core/schemas/`; validator at `crates/cortex-core/src/validate.rs`; fixtures at `crates/cortex-core/tests/fixtures/`.

## Goal

Define the **single, versioned wire format** for every artifact that flows through Cortex — from capture adapters, across the Synap event bus, into processing workers, and out as enriched records. All adapters, workers, the query API, the dashboard, and the bootstrap CLI must speak this schema. Locking it first unblocks every other spec.

## Scope

**In:**
- Top-level event envelope (fields, types, validation).
- Per-`kind` payload schemas (Turn, ToolCall, AgentCall, Memory, Decision, Analysis, LawViolation, Artifact).
- Identity rules (event IDs, content hashes, deduplication keys).
- Schema versioning and forward/backward compatibility policy.
- Reference JSON Schema files and Rust type definitions.

**Out:**
- How events are *captured* (→ spec 10 for Claude Code, spec 17 for others).
- How events are *processed* (→ spec 04+).
- How events are *queried* (→ spec 11).
- Storage representation (→ spec 02).

## Inputs / Outputs

### Top-level envelope

Every event, regardless of `kind`, has the same envelope:

```jsonc
{
  "event_id":      "01HXYZABCDEF0123456789ABCD",   // ULID, 26 chars; lex-sortable by time
  "schema_version":"1",                              // semver-major; changes only on breaking
  "occurred_at":   "2026-04-17T12:34:56.789Z",       // RFC 3339, UTC, ms precision
  "ingested_at":   "2026-04-17T12:34:57.012Z",       // set by ingestion router, not adapter
  "session_id":    "01HXYZ...",                      // ULID, stable per AI session
  "stream":        "live",                           // "live" | "bootstrap"
  "tool":          "claude-code",                    // adapter id (controlled vocab, see below)
  "model":         "claude-opus-4-7",                // model id (free-form string; null if N/A)
  "kind":          "tool_call",                      // discriminator (see below)
  "context":       { /* see Context */ },
  "payload":       { /* shape determined by kind */ },
  "redactions":    ["secret:.env:line=12", "..." ],  // applied before publication
  "content_hash":  "sha256:abc123...",               // sha256 of canonical(payload), pre-redaction
  "parent_event_id": null                            // for nested events (e.g., subagent in agent_call)
}
```

**Field rules:**

| Field             | Required | Notes                                                              |
|-------------------|:--------:|--------------------------------------------------------------------|
| `event_id`        |    ✓     | ULID; client-generated; conflicts on insert are an error           |
| `schema_version`  |    ✓     | Currently `"1"`; bump only on breaking change                      |
| `occurred_at`     |    ✓     | When the event happened in the source system                       |
| `ingested_at`     |          | Set by the ingestion router; never trust adapter-supplied value    |
| `session_id`      |    ✓     | One ULID per AI session; adapter responsible for stability         |
| `stream`          |    ✓     | Routes to `cortex.events.raw` or `cortex.events.bootstrap`         |
| `tool`            |    ✓     | Adapter id; see §"Controlled vocabularies" below                   |
| `model`           |          | `null` for non-LLM events (filesystem walk, git commit historical) |
| `kind`            |    ✓     | Discriminator for `payload` shape; see §"Kinds" below              |
| `context`         |    ✓     | Capture-time metadata; see §"Context"                              |
| `payload`         |    ✓     | Kind-specific; validated against per-kind JSON Schema              |
| `redactions`      |          | List of opaque tokens describing what was scrubbed                 |
| `content_hash`    |    ✓     | sha256 over canonical-JSON of `payload` *before* redaction         |
| `parent_event_id` |          | For nested/derived events; null at top level                       |

### Context block

```jsonc
{
  "repo":    "e:/HiveLLM/Cortex",        // absolute path, normalized (forward slashes)
  "branch":  "main",                     // null if not in a git repo
  "commit":  "abc123def456",             // null if not in a git repo
  "cwd":     "e:/HiveLLM/Cortex/docs",   // working directory when event occurred
  "user":    "andre@hivellm",            // local OS user @ org tag
  "platform":"win32",                    // win32 | darwin | linux
  "ide":     "vscode-claude-code",       // optional, free-form
  "extras":  { /* adapter-specific bag */ }
}
```

### Kinds and payloads

Eight `kind` values. Each has its own JSON Schema in `cortex-core/schemas/kinds/{kind}.schema.json`.

| `kind`           | Purpose                                                             |
|------------------|---------------------------------------------------------------------|
| `turn`           | One user↔assistant exchange (prompt + response text)                |
| `tool_call`      | Invocation of a tool (Bash, Edit, Read, MCP tool, etc.)             |
| `agent_call`     | Invocation of a sub-agent (Task tool, code-reviewer, etc.)          |
| `memory`         | Persisted memory write/update/delete                                |
| `decision`       | Formalized decision record (ADR-style)                              |
| `analysis`       | Deep-analysis report (often the parent of many turn events)         |
| `law_violation`  | Detector fired; emitted by governance engine (spec 14)              |
| `artifact`       | Stand-alone artifact reference (file, diff, snippet, URL)           |

**Example — `tool_call` payload:**

```jsonc
{
  "tool_name":  "Bash",                          // controlled per-adapter vocab
  "input":      { "command": "git status", "description": "..." },
  "output":     { "stdout": "...", "stderr": "", "exit_code": 0, "truncated": false },
  "duration_ms": 142,
  "touched":    [                                 // resolved post-call by adapter
    { "kind": "file_read",  "path": "e:/HiveLLM/Cortex/docs/architecture.md" },
    { "kind": "file_write", "path": "e:/HiveLLM/Cortex/docs/specs/01-event-schema.md" }
  ],
  "outcome":    "success"                         // success | error | blocked_by_law:LAW-007
}
```

**Example — `turn` payload:**

```jsonc
{
  "user_message": "refactor the HNSW configurator",
  "assistant_message": "I'll start by reading...",
  "tokens": { "in": 1234, "out": 567, "cache_read": 8000, "cache_write": 1000 },
  "tool_call_event_ids": ["01HXYZ...", "01HXYZ..."]    // back-references to children
}
```

**Example — `memory` payload:**

```jsonc
{
  "op":     "write",                              // write | update | delete
  "memory_type": "feedback",                      // user | feedback | project | reference
  "name":   "Cortex — early decisions",
  "body":   "...",
  "memory_path": "C:/Users/Bolado/.claude/.../memory/project_cortex_decisions.md"
}
```

Other payloads (`agent_call`, `decision`, `analysis`, `law_violation`, `artifact`) are documented inline in the JSON Schema files; this spec freezes only the *shape policy*, not every field. Each kind's schema may evolve under §"Schema evolution" rules without bumping `schema_version`, as long as changes are additive.

### Controlled vocabularies

Two fields draw from controlled vocabularies, versioned alongside the schema:

- `tool` — `{ "claude-code", "cursor", "codex", "gemini", "copilot", "windsurf", "cortex-cli", "cortex-bootstrap", "git-hook", "fs-watcher" }`
- `kind` — exactly the 8 values listed above; adding a new kind is a breaking change.

The `topics[]` field produced later by the classifier (spec 05) is a separate vocabulary, not part of this schema.

### Identity & deduplication

- `event_id` is the **primary key**; ingestion is idempotent on it. Re-publishing an event with the same `event_id` is a no-op.
- `content_hash` (sha256 of canonical-JSON `payload`, *pre*-redaction) is a **secondary key** used by the classifier cache (spec 05) and the embedder dedup (spec 06). Identical payloads from different sources share the hash.
- Canonical JSON: keys sorted lexicographically, no insignificant whitespace, UTF-8, numbers in shortest-roundtrip form. Reference impl: `serde_json::to_string` with a sorted-map wrapper.

## Design

### Schema artifacts

The schema lives in three forms, all generated from one source:

```
cortex-core/
├─ schemas/
│  ├─ envelope.schema.json           # JSON Schema, draft 2020-12
│  ├─ context.schema.json
│  └─ kinds/
│     ├─ turn.schema.json
│     ├─ tool_call.schema.json
│     ├─ agent_call.schema.json
│     ├─ memory.schema.json
│     ├─ decision.schema.json
│     ├─ analysis.schema.json
│     ├─ law_violation.schema.json
│     └─ artifact.schema.json
├─ src/
│  └─ events.rs                       # Rust types, derive(Serialize, Deserialize)
└─ build.rs                           # generates events.rs from schemas at build time
```

JSON Schemas are the source of truth. Rust types and TypeScript types (for the dashboard) are generated. Adapters in other languages can pull the schemas at runtime or vendor them.

### Validation

Every event published to Synap must validate against `envelope.schema.json` AND the appropriate per-kind schema. Validation happens **twice**:

1. **At adapter** — fail closed; reject malformed events before they reach the bus.
2. **At ingestion router** — defense in depth; malformed events go to a dead-letter Synap stream `cortex.events.invalid` for inspection.

A malformed event is never silently dropped.

### Schema evolution

We use a strict additive-only policy on `schema_version="1"`:

| Change                                              | Allowed without version bump? |
|-----------------------------------------------------|:-----------------------------:|
| Adding a new optional field to envelope or payload  |              ✓                |
| Adding a new value to a controlled vocabulary       |              ✓                |
| Adding a new `kind` (with new schema file)          |              ✓                |
| Renaming a field                                    |              ✗                |
| Changing a field's type                             |              ✗                |
| Removing a field                                    |              ✗                |
| Making an optional field required                   |              ✗                |
| Removing a value from a controlled vocabulary       |              ✗                |

Breaking changes require a new `schema_version` and a migration plan written into the spec that introduces them.

### Redaction representation

`redactions` is a list of opaque, machine-readable tokens describing **what was removed and where**, never the secret itself:

```
secret:env_var:OPENAI_API_KEY
secret:file:.env:line=12
secret:pattern:bearer_token:tool_call.input.command:offset=42:length=64
```

Tokens follow the form `secret:<class>:<locator>` and are produced by the redactor (spec 04). Locators reference paths into the *redacted* payload so reviewers can find the redaction site without seeing the secret.

### Examples (round-trip test fixtures)

`cortex-core/tests/fixtures/events/` will hold one sample per `kind` plus a few edge cases (max-size payload, deeply nested agent call, tool call with truncated output, redacted tool call). These fixtures double as documentation and as the basis for adapter conformance tests.

## Acceptance criteria

- [ ] `envelope.schema.json` and all 8 `kinds/*.schema.json` files exist and validate against JSON Schema 2020-12.
- [ ] `cortex-core/src/events.rs` is generated from the schemas at build time; no hand-edited divergence allowed.
- [ ] Round-trip test: every fixture deserializes from JSON → Rust struct → JSON and the result is byte-identical (after canonicalization).
- [ ] Validation test: a deliberately malformed event of each `kind` is rejected by the validator with a useful error path (e.g., `payload.touched[1].path: required`).
- [ ] Canonical JSON helper produces stable `content_hash` values across platforms (verified on win32, darwin, linux).
- [ ] All controlled vocabularies are exposed as `pub const` arrays in Rust and as `as const` arrays in the generated TypeScript module.
- [ ] Adapter conformance suite: a tiny harness reads the fixtures and verifies an adapter implementation can produce all of them. Used by every adapter spec from #10 onward.

## Decisions (resolved during drafting)

1. **Max payload size:** **1 MB hard cap on the envelope.** Anything larger is rejected by the validator. Producers must summarize and offload the long tail to CAS (spec 02) before publishing.
2. **Inline-vs-CAS threshold for blob fields** (`tool_call.output.stdout`, large `decision.body`, etc.): **inline up to 16 KB; above that, store in CAS and replace inline value with `{ "cas_ref": "sha256:...", "size": N, "truncated": true }`**. Adapter responsibility.
3. **`model` field:** **free-form string** with a soft-validated registry (`cortex-core/registries/models.json`). Unknown models log a warning, never reject.
4. **Tracing IDs:** **W3C `traceparent`** carried in `context.extras.traceparent` when present. Optional; we don't synthesize one if the adapter didn't.
5. **Streaming turns:** emit **one final `turn` event** when the assistant message completes; partial tokens are not events. Add `tokens.streamed_chunks` (int) for analytics.

## Open questions

*(none — all founding decisions resolved during drafting; reopen by superseding spec)*

## References

- Architecture §4.3 (envelope), §4.1 (entity types), §5.1 (capture), §8 (privacy).
- Spec 02 — Storage layout (will define how envelope fields map to Vectorizer collections, Nexus labels, Meilisearch indexes, Parquet partitions).
- Spec 04 — Cortex Core (consumes and validates events).
- Spec 05 — Classifier (extends events with classifier output).
- Spec 10 — Claude Code adapter (first concrete producer).
- External: [JSON Schema 2020-12](https://json-schema.org/draft/2020-12/schema), [ULID spec](https://github.com/ulid/spec), [RFC 3339](https://datatracker.ietf.org/doc/html/rfc3339).
