# 05 — Classifier (Haiku via CLI / SDK)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 01, 04
>
> Library: [`crates/cortex-classifier/`](../../crates/cortex-classifier/). `Classifier` trait, `StaticClassifier` (pure-Rust rule table), `CachedClassifier`, `BudgetedClassifier` (threshold ladder → static fallback), `HaikuCliClassifier` (spawns `claude -p ... --output-format json`), prompt template v1 + topic vocabulary v1.
>
> Worker binary: [`crates/cortex-classifier-worker/`](../../crates/cortex-classifier-worker/). Standalone crate (separate to avoid a `classifier → embedder → classifier` cycle, since the worker publishes the embedder's `EnrichedEvent` shape). Drains `cortex.events.raw` + `cortex.events.bootstrap`, classifies through the configured stack (`static` default, `cli` opt-in via `CORTEX_CLASSIFIER_MODE=cli`), and publishes on `cortex.events.enriched`. Replay dedup is keyed on `event_id`; missing Synap rooms are auto-created on the first publish.

## Goal

Implement the classifier worker that consumes raw/bootstrap events, batches them, calls **Claude Haiku 4.5** (default: through the **Claude Code CLI** in headless mode; optional: through the **Anthropic SDK** directly), parses the structured JSON output, caches by content hash, tracks daily spend, and publishes enriched events. No local model. No training.

## Scope

**In:**
- Worker layout, batching, parallelism.
- Prompt template (versioned, tested).
- Two invocation backends (CLI default, SDK optimization) behind one trait.
- Output parser + validation against a fixed JSON Schema.
- Content-hash cache (Synap-backed).
- Budget tracker + degradation ladder.
- Static fallback classifier.
- Telemetry.

**Out:**
- Embedding (spec 06).
- Topic vocabulary curation (lives in `cortex-classifier/topics.yaml`, evolved over time, not part of this spec's freeze).
- Prompt evolution beyond v1 — handled as new `prompt_version` releases under same spec.

## Inputs / Outputs

### Trait

```rust
#[async_trait]
pub trait Classifier: Send + Sync {
    async fn classify_batch(&self, events: &[EnrichmentInput]) -> Result<Vec<ClassifierOutput>>;
}

pub struct EnrichmentInput {
    pub event_id: String,
    pub kind: Kind,
    pub content_hash: String,                 // pre-redaction; cache key
    pub redacted_payload: serde_json::Value,  // what we send to Haiku
    pub context_repo: Option<String>,
}

pub struct ClassifierOutput {
    pub event_id: String,
    pub kind_refinement: Option<String>,      // e.g., "git_push"
    pub topics: Vec<String>,                  // from controlled vocab
    pub severity: Severity,                   // info | notable | critical
    pub pii_risk: PiiRisk,                    // low | medium | high
    pub redaction_suggestions: Vec<RedactionSuggestion>,
    pub summary: Option<String>,              // ≤ 300 chars; mandatory if input >4 KB
    pub source: ClassifierSource,             // haiku_cli | haiku_sdk | cache | static_fallback
    pub prompt_version: String,
    pub model: String,                        // "claude-haiku-4-5"
    pub latency_ms: u32,
    pub tokens_in: u32,
    pub tokens_out: u32,
}
```

### Implementations

```rust
pub struct HaikuCliClassifier { claude_bin: PathBuf, model: String, prompt: PromptTemplate }
pub struct HaikuSdkClassifier { client: anthropic::Client, model: String, prompt: PromptTemplate }
pub struct CachedClassifier<C> { inner: C, cache: SynapKvCache }
pub struct BudgetedClassifier<C> { inner: C, budget: BudgetTracker, fallback: StaticClassifier }
pub struct StaticClassifier { rules: RulesTable }
```

Composition (default wiring):

```rust
let inner: Box<dyn Classifier> = match cfg.mode {
    ClassifierMode::Cli => Box::new(HaikuCliClassifier::new(...)),
    ClassifierMode::Sdk => Box::new(HaikuSdkClassifier::new(...)),
};
let cached = CachedClassifier::new(inner, synap.clone());
let final_ = BudgetedClassifier::new(cached, budget, StaticClassifier::default());
```

### Prompt template (v1)

`cortex-workers/prompts/classifier.v1.txt`:

```
You are an event classifier for the Cortex system. You will receive a JSON
array of events. For each event, output one classification record. Output a
single JSON object: {"events":[<record>, ...]} in the same order. No commentary.

Topic vocabulary (use only these; multi-label allowed; lowercase, snake_case):
{{TOPIC_VOCAB}}

Severity:
  info     — routine, low signal
  notable  — worth surfacing in dashboard timeline (decisions, errors, refactors)
  critical — must alert (security, broken contract, law violation, data loss)

PII risk:
  low      — no personal data, no secrets
  medium   — usernames, emails, internal paths, repo/branch names
  high     — credentials, tokens, keys, financial data, customer PII

Summary rule:
  If event.payload exceeds 4096 chars (excluding whitespace), produce a 1–2
  sentence summary capturing the WHAT and the OUTCOME.
  Otherwise, omit the summary field.

Redaction suggestions:
  If you see a likely secret the static redactor missed, add an entry to
  redaction_suggestions: [{ "pattern_class": "...", "json_pointer": "...",
  "rationale": "..." }]

Output schema (one record per input event):
{
  "event_id": "...",
  "kind_refinement": "...|null",
  "topics": ["..."],
  "severity": "info|notable|critical",
  "pii_risk": "low|medium|high",
  "redaction_suggestions": [],
  "summary": "...|null"
}

Events:
{{EVENTS_JSON}}
```

The vocabulary `{{TOPIC_VOCAB}}` is interpolated from `cortex-classifier/topics.yaml` at startup. Hot-reloadable on SIGHUP.

### CLI invocation

```rust
async fn classify_batch_cli(&self, events: &[EnrichmentInput]) -> Result<Vec<ClassifierOutput>> {
    let prompt = self.prompt.render(events)?;            // ~one input → ~one prompt file
    let mut cmd = Command::new(&self.claude_bin);
    cmd.args([
        "-p", &prompt,
        "--model", &self.model,
        "--output-format", "json",
        "--max-tokens", "4096",
    ]);
    let output = cmd.output().await?;
    let text = String::from_utf8(output.stdout)?;
    let parsed: ClaudeJsonResponse = serde_json::from_str(&text)?;
    let inner: ClassifierOutputBatch = serde_json::from_str(&parsed.text)?;
    self.validate_and_match(events, inner)
}
```

If the prompt exceeds the CLI's argv limit on the platform, the worker writes it to a tempfile and uses `--prompt-file`.

### SDK invocation

```rust
async fn classify_batch_sdk(&self, events: &[EnrichmentInput]) -> Result<Vec<ClassifierOutput>> {
    let prompt = self.prompt.render(events)?;
    let resp = self.client.messages()
        .create(MessagesRequest::new(self.model.clone(), 4096)
            .add_user_message(prompt)
            .response_format_json())
        .await?;
    let inner: ClassifierOutputBatch = serde_json::from_str(&resp.content_text())?;
    self.validate_and_match(events, inner)
}
```

Reuses the same `ClassifierOutputBatch` type — backend swap is transparent to the rest of the worker.

### Output validation

`validate_and_match` enforces:

- The output array length equals input length.
- `event_id` values match input order (set membership; if mismatched, falls back to map by id).
- All `topics[]` values are in the vocabulary; unknown topics trigger a warning and are dropped (not rejected — Haiku occasionally invents).
- `severity` and `pii_risk` are valid enums.
- `summary` is present when input payload exceeds 4 KB.

If validation fails for the whole batch, the worker retries once with a stricter prompt; second failure routes the batch to `cortex.events.invalid` with cause = `classifier_output_unparseable`.

### Cache

Key: `cache:classify:<content_hash>` (Synap KV, TTL 24 h, spec 02).
Value: serialized `ClassifierOutput` minus latency/tokens (cache hit reports `latency_ms=0`, `source=cache`).

Cache is checked **per event**, not per batch — a batch of 32 may have 20 cache hits and only 12 forwarded to Haiku. If all 32 hit, no API call is made.

### Budget tracker

```rust
struct BudgetTracker {
    daily_limit_usd_cents: u32,
    spend_today: AtomicU32,             // resets at UTC midnight
    pricing: HaikuPricing,              // tokens_in_usd_per_1k, tokens_out_usd_per_1k
    threshold_warn: f32,                // 0.8
    threshold_degrade: f32,             // 0.9
    threshold_halt: f32,                // 1.0
}
```

Behavior at thresholds:

| Spend ÷ limit | Action                                                                 |
|---------------|------------------------------------------------------------------------|
| < 0.8         | Normal operation                                                        |
| ≥ 0.8         | Log warning, increment `cortex.classifier.budget.warn`                  |
| ≥ 0.9         | Drop `redaction_suggestions` and `summary` from prompt; raise batch to 64 |
| ≥ 1.0         | Switch to `StaticClassifier` for non-`critical` events; queue critical |

Spend persisted in SQLite metadata (`classifier_spend` table, spec 02) on every API call.

### Static fallback classifier

Pure-Rust rules; no API calls. Produces correct `kind`, lossy `topics`, conservative `severity` and `pii_risk`.

```rust
pub struct StaticClassifier { rules: RulesTable }

// Examples:
//   tool_call.tool_name == "Bash" && payload.input.command starts with "git push"
//     -> kind_refinement="git_push", topics=["git","deployment"], severity=notable
//   tool_call.tool_name == "Edit" -> topics=["code","edit"]
//   tool_call.tool_name == "Read" -> topics=["read"], severity=info
//   tool_call.input matches /password|secret|token/i -> pii_risk=high
//   default -> topics=[], severity=info, pii_risk=low
```

Rules live in `cortex-classifier/static-rules.yaml`. Easy to extend; not a substitute for Haiku — it's a graceful degradation layer.

### Summary contract

`ClassifierOutput.summary: Option<String>` is `None` only on the Haiku/SDK path when the payload is ≤ 4 KB (per the prompt rule above), or on a cache replay of a stored `None`. The **static fallback path never returns `None`**: since `phase26c §1.2` it always stamps a deterministic `"{kind} in {location}: {snippet}"` summary, so the fulltext worker and the vector embedder keep a readable body candidate even when Haiku is unreachable or budget-halted. The old `"static summary: <N> chars"` placeholder (2026-04-27 incident, below) must never come back.

As of `phase26f §2.1`, `{snippet}` is a per-kind NL extraction (`nl_summary_snippet` in [`crates/cortex-workers/src/classifier/statics.rs`](../../crates/cortex-workers/src/classifier/statics.rs)) — Turn → user/assistant messages, ToolCall → `tool: k=v` pairs, Decision → title/status/body, Knowledge/Learning → title/body, etc., falling back to a generic `body`/`content`/`text` field with whitespace collapsed to one line. Previously the snippet was the first 120 chars of the raw-flattened JSON payload (`{"text":...}`-style noise in the Meili summary + keyword lane); the embedder's separate `nl_projection` hot path (spec 06) is unrelated and was intentionally left untouched.

Downstream readers (`cortex-fulltext-worker` body selector, `cortex-embedder` chunker) still treat `summary == None` as the cue to fall back on the source `text` / redacted payload — see spec-08 §Body selection rule 2 and spec-06 §Chunk inputs — which in practice now only fires on the Haiku/SDK path, since static never emits `None`.

**Why this matters (2026-04-27 incident):** the previous implementation stamped `summary = "static summary: <N> chars"` for any payload over 4 KB. The full-text worker copied that placeholder into Meilisearch's `body` field, and search broke for ~96 % of indexed artifacts (no real tokens to match against). The fix, scoped under `phase2_static_classifier_summary_preserves_text`, dropped the field (`summary = None`) as the immediate mitigation; `phase26c §1.2` replaced it with the deterministic template above, and `phase26f §2.1` cleaned the template's snippet so it reads as prose instead of raw JSON. Reindex via `cortex-bootstrap` recovers documents indexed before either fix.

## Design

### Worker concurrency

```
Synap consumer ──┐
                 ├─► batcher ──► classifier (Haiku) ──► publisher (cortex.events.enriched)
                 │                     │
                 │                     ├─► cache write
                 │                     └─► budget update
                 │
              (multiple workers in same consumer group; Synap rebalances)
```

**Batching:** 32 events per batch; flush on N=32 OR 200 ms timeout, whichever first. Per spec architecture §5.2.1, a single worker handles ~20 eps; pool of 25 covers 500 eps.

**Concurrency knobs (env):**
- `CORTEX_CLASSIFIER_WORKERS=8` (default; scale up for live traffic spikes)
- `CORTEX_CLASSIFIER_BATCH=32`
- `CORTEX_CLASSIFIER_FLUSH_MS=200`
- `CORTEX_CLASSIFIER_MAX_CONCURRENT_BATCHES_PER_WORKER=4`

### CLI vs SDK selection

Per worker process. CLI is the default (uses Claude Code subscription quota); SDK is opt-in via `CORTEX_CLASSIFIER_MODE=sdk` and requires `ANTHROPIC_API_KEY`. Mixed deployments allowed: e.g., 2 SDK workers for the bootstrap stream, 6 CLI workers for live.

### Prompt versioning

Each prompt template lives in `cortex-workers/prompts/classifier.v<N>.txt`. The active version is set via `CORTEX_CLASSIFIER_PROMPT_VERSION` (default: latest). `ClassifierOutput.prompt_version` records which version produced each result so we can re-classify selectively when prompts change.

### Failure modes

| Failure                              | Handling                                                               |
|--------------------------------------|------------------------------------------------------------------------|
| CLI process timeout (>30s)           | Kill, retry once, then mark batch as `static_fallback` and continue    |
| SDK 429 / 5xx                        | Exponential backoff (1s, 2s, 4s), max 3 attempts, then static fallback |
| JSON parse error from Haiku          | Retry once with stricter prompt; then dead-letter                      |
| Schema validation error per event    | Per-event fallback to static for that one event; rest of batch OK      |
| Synap consumer lag                   | Backpressure already handled by router (spec 04); no extra logic       |
| Budget halted                        | Static fallback for non-critical; critical events queued for next day  |

### Observability

Every classification emits a span with attributes: `events.count`, `cache.hits`, `cache.misses`, `classifier.source`, `classifier.latency_ms`, `tokens.in`, `tokens.out`, `cost.usd_cents`. Aggregated counters on `cortex.metrics`:

```
cortex.classifier.requests.total       counter, labels: source, status
cortex.classifier.batch_size           histogram
cortex.classifier.latency_ms           histogram, labels: source
cortex.classifier.cache.{hit,miss}     counter
cortex.classifier.tokens.{in,out}      counter
cortex.classifier.cost.usd_cents       counter (daily reset visible via labels)
cortex.classifier.budget.state         gauge: 0 normal, 1 warn, 2 degrade, 3 halt
cortex.classifier.fallback.total       counter, labels: reason
```

## Acceptance criteria

- [ ] CLI mode: a smoke test sends 32 mixed events through the worker, all 32 receive valid `ClassifierOutput`, latency P95 < 3 s on a developer machine.
- [ ] SDK mode: same test succeeds with `CORTEX_CLASSIFIER_MODE=sdk`.
- [ ] Cache: re-running the smoke test with identical events returns 32 cache hits, zero API calls (verified via `tokens.in==0` counter delta).
- [ ] Vocabulary enforcement: a synthetic event whose Haiku output invents `topics: ["unknown_topic"]` triggers a warning, the unknown topic is dropped, the rest of the record is kept.
- [ ] Budget: synthetic spend 1.0× limit triggers static-fallback path; verified by `source=static_fallback` on subsequent non-critical events; critical events queued, not silently dropped.
- [ ] Static fallback alone passes all required output fields validation on a 1k synthetic-event corpus (no Haiku reachable).
- [ ] CLI process timeout: simulated by `claude_bin` pointing to a `sleep 60` script — worker kills at 30 s, retries, then falls back, no batch lost.
- [ ] Prompt version recorded on every output; switching `CORTEX_CLASSIFIER_PROMPT_VERSION` mid-run produces records with the new version.
- [ ] Telemetry counters non-zero after the soak.

## Decisions

1. **CLI default, SDK opt-in.** Honors user preference (uses Claude Code quota); SDK reserved for high-throughput bootstrap.
2. **Batching is mandatory** — N=32, 200 ms flush. No single-event Haiku calls allowed in production code path.
3. **Cache key = pre-redaction `content_hash`.** Best dedup. Cache invalidation on `prompt_version` change handled by including version in key: `cache:classify:v1:<hash>`.
4. **Static fallback is built-in, not optional.** It's the safety net for budget exhaustion AND offline dev (no network).
5. **Topic vocabulary lives in YAML, not in prompt.** Hot-reloadable; out-of-vocab outputs are dropped, not rejected.
6. **No fine-tuning, no LoRA, no Expert in v1.** Re-evaluate at Phase 4 review using thresholds in architecture §12 OQ #1.

## Open questions

*(none — defaults locked)*

## References

- Architecture §5.2.1 (Haiku rationale + batching).
- Spec 01 — Event schema (`content_hash` is the cache key).
- Spec 02 — Storage layout (Synap KV namespace `cache:classify:*`, SQLite `classifier_spend`).
- Spec 04 — Cortex Core (consumes `cortex.events.raw` & `bootstrap`; emits to `cortex.events.enriched`).
- Spec 06 — Embedder (consumes the `summary` field for >4 KB payloads).
- Anthropic SDK docs; Claude Code CLI `--output-format json` reference.
