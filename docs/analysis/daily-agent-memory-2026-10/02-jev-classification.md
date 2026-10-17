# Jev: advisory judgments on explicit memory candidates

Date: 2026-10-16. Status: design for §1.1 of
`phase34_daily-agent-memory-recovery`; implementation belongs to §1.5, after
compact storage (§1.3) and capture/read APIs (§1.4). No inference request was
made and no credential was inspected during this research.

## Selected scope and joint decision

Use native Rust TypeSafe calls only on low-volume explicit lesson/decision
candidates. Three base judgments describe durability, kind, and scope. Optionally
compare against at most three already-authorized active memories for contradiction
hints. Rust owns persistence, scope, provenance, lifecycle, and delivery.

Explicit writes always persist, including transient, uncertain, or unclassifiable
content. Jev supplies advisory metadata; it cannot broaden scope, delete or
supersede memories, promote rules, or generate summaries. A typed explicit user
transition remains necessary for supersession. Existing source text or a
deterministic excerpt supplies display text.

The earlier 49-question all-event classifier backend was **evaluated, not
selected**. Claude's [peer review](04-claude-coordination.md#55-bounded-jev-backend-f-011--f-020)
identified the mismatch between tool-call volume and useful memory capture.
The coordinator's joint Fable/Kimi/Grok decision selects compact durable memory
and candidate-only Jev. General event enrichment, injection-time inference,
graph extraction, a model gateway, and old-worker accounting redesign are deferred.

## Findings

### F-011 — Candidate classification belongs after durable capture

**Evidence:** The selected task orders compact storage, API/MCP exposure, and Jev
at `.rulebook/tasks/phase34_daily-agent-memory-recovery/tasks.md:4`, `:5`, and
`:6`. Current capture is exposed at
`crates/cortex-mcp-server/src/tools.rs:913`. Claude's placement review is
`docs/analysis/daily-agent-memory-2026-10/04-claude-coordination.md:151`.

**Impact:** §1.5 adds a small advisory service to the memory-capture path defined
by §1.3/§1.4, not a `Classifier` implementation consuming all events. Commit the
explicit memory first. Release database locks before network work. A bounded
post-commit classification can annotate that record; failure or caller
cancellation cannot roll back capture. Do not add a new durable queue merely to
make optional advice eventually arrive.

**Confidence:** High; selected task dependency order and peer review are explicit.

### F-012 — Verified HTTP contract supports a small native Rust client

**Evidence:** The live [HTTP API](https://docs.typesafe.ai/api) specifies
`POST https://api.typesafe.ai/v1/systemone`, bearer authorization, and JSON with
`state`, `model`, and a `questions` map. Responses contain resolved `model`, an
`answers` map keyed by submitted IDs, and request-level `usage.input_tokens` and
`usage.output_tokens`. Question IDs are not sent to the model; instructions must
state the full meaning of each question.

The API requires UTF-8 JSON without a BOM, limits bodies to 32 MiB, and limits
nesting to 64 levels. Application limits should be far smaller. HTTP `400`,
`401`, `413`, `415`, and `422` are terminal request/authentication failures;
`429` and `529` support delayed retries. Existing Rust workspace dependencies
include `reqwest` and `serde`; tests can use existing `wiremock` conventions.

**Impact:** Implement the required Noul/Choice transport with typed serde enums,
fixed endpoint, bounded body/deadline, and explicit error categories. No SDK,
subprocess, model gateway, or unused Score implementation is needed. Score was
reviewed: it returns a weighted position on ordered levels, and is not needed
for the selected candidate questions.

**Confidence:** High; current provider contract read directly. Markdown URLs
that failed through the web connector were read at their normal HTML URLs.

### F-013 — Pin stable Jev and record actual usage

**Evidence:** The inspected [models and aliases](https://docs.typesafe.ai/models)
list `jev-1.13.0` and `jev-1.14.0`, with the latter released October 14, 2026.
`jev-latest` currently resolves to stable `jev-1.13.0`; `jev-preview` resolves to
`jev-1.14.0`. Both list $0.042 per million input tokens and free output tokens.
Default limits are 64k tokens per request and 32k per question.

**Impact:** Pin `jev-1.13.0` initially. Store the resolved response model,
question version, actual input/output usage, and latency with advice. Evaluate
preview separately before migration. Do not enable long context. At current
prices, 1,000 input tokens cost $0.000042; output usage remains observable but
has zero monetary charge. Aliases must not silently change calibrated behavior.

**Confidence:** High for the inspected contract; aliases and prices can change.

### F-014 — Three base judgments describe candidates without controlling writes

**Evidence:** [Choice](https://docs.typesafe.ai/primitives/choice) returns one
selected option, its probability distribution, and confidence.
[Noul](https://docs.typesafe.ai/primitives/noul) returns the probability of yes,
with no separate confidence. The
[memory cookbook](https://docs.typesafe.ai/cookbooks/basic_memory) demonstrates
a narrow durability question; its example write gate is not the chosen Cortex
policy. The [RAG cookbook](https://docs.typesafe.ai/cookbooks/classifying_rag_passages)
keeps observations separate from code-owned routing decisions.

Use these three base questions together over one sanitized state:

| ID | Primitive | Meaning and criteria |
| --- | --- | --- |
| `durable` | Noul | Does the candidate state a reusable decision or lesson that should remain useful beyond the current transient operation? Define yes/no with durable and status-only examples. |
| `kind` | Choice | `decision`: an adopted choice with consequences; `lesson`: an observed correction or reusable finding; `transient`: status, speculation, or no supported durable claim. |
| `scope` | Choice | `repo`: supported only within the current repository; `global`: candidate explicitly supports general applicability; `unclear`: evidence does not establish scope. |

State includes candidate source text, declared kind/scope, and minimal trusted
repository context. Questions must distinguish a claimed result from a plan,
quoted content, or embedded instructions.

**Impact:** The response is typed advice, not a permission to persist. Preserve
explicit declared kind and scope. An uncertain or transient result remains a
stored explicit memory. Model `global` advice cannot make a repo-scoped record
visible elsewhere; any scope change uses the explicit API contract.

**Confidence:** High on primitive fit and authority boundary; model accuracy
requires English/Portuguese examples from this domain.

### F-015 — Thresholds and contradiction hints remain advisory

**Evidence:** The [confidence guidance](https://docs.typesafe.ai/confidence)
distinguishes distribution concentration from correctness. A Noul near 0.5 is
uncertain yes/no probability, not medium intensity. Claude's review at
`docs/analysis/daily-agent-memory-2026-10/04-claude-coordination.md:161` recommends
candidate-time contradiction judgments while deterministic code owns transitions.

Proposed initial display thresholds, not provider guarantees:

| Signal | Proposed treatment |
| --- | --- |
| `durable >= 0.80` | Display durable suggestion. |
| `durable <= 0.20` | Display transient suggestion; preserve explicit write. |
| Intermediate durability | Display uncertain; preserve explicit write. |
| Kind/scope Choice confidence `>= 0.80` | Display selected advisory label; preserve declared fields. |
| Lower kind/scope confidence | Display uncertain; preserve declared fields. |
| Contradiction Noul `>= 0.80` | Display possible conflict with cited active memory; no transition. |

If cheap local retrieval finds relevant active memories, include at most three
that the caller is already authorized to read in the exact effective repo/scope.
Ask one contradiction Noul per supplied pair: whether their factual claims are
incompatible, not whether one should replace the other. There are three base
questions and zero to three optional pair questions, at most six total. Candidate
coverage, existence, status, and authorization are checked in Rust. Model scope
advice cannot change which comparison memories may be loaded.

**Impact:** Hints help expose duplicates/conflicts without auto-supersession,
automatic law promotion, or cross-project search. Store probabilities separately
from display labels so threshold tuning requires no repeat inference. Similarity
alone never establishes contradiction or precedence.

**Confidence:** High on required invariants; medium on proposed thresholds until
calibrated against independently labeled candidates.

### F-016 — Sanitize the complete egress state, not only its main text

**Evidence:** The reusable redactor starts at
`crates/cortex-core/src/redact.rs:105`. Ingestion uses it at
`crates/cortex-workers/src/ingestion/router.rs:435`; bootstrap uses it at
`crates/cortex-cli/src/bootstrap/emitter.rs:109`. Its catalog has bearer and
assignment patterns, but no dedicated `ts_` pattern or general masking of
secret-bearing JSON values by field name. The sibling transport reference is
`E:/HiveLLM/Rulebook/src/core/typesafe/client.ts:1`.

**Impact:** Reuse core redaction on a clone, then cover literal configured key,
secret-bearing fields, TypeSafe tokens, and incomplete private-key blocks.
Sanitize candidate, context, and optional comparison text before checking size.
Omit event/session IDs, source file contents, hashes, and unrelated envelope
fields. Map comparison IDs to local indices so answers cannot fabricate targets.
If evidence is too large, skip advice rather than silently judging a prefix.

Resolve process environment first, with an explicit ignored local `.env`
fallback for only the named key when needed. Keep key outside derived `Debug`,
serialized config, state, logs, and fixtures. Bearer header alone carries it.
Log categories, model, usage, latency, and fallback reason; never raw state or
HTTP error bodies. This does not claim complete PII anonymization or a provider
retention policy.

**Confidence:** High; redaction implementation and credential boundary inspected.

### F-017 — Deterministic fallback means saved source with no model authority

**Evidence:** Existing event-classifier failure at
`crates/cortex-workers/src/classifier_worker/worker.rs:1066` calls the all-default
helper at `:1304`. The current HTTP classifier evaluation hardwires static
classification at `crates/cortex-workers/src/classifier/http.rs:61`. Neither path
is the selected durable-memory capture/advice contract.

**Impact:** The new service returns a typed unavailable/uncertain advice outcome
on missing key, timeout, malformed response, capacity exhaustion, or outage. The
saved memory's text, declared kind, scope, provenance, lifecycle, and revision
remain valid. Keep summary as original/source text or a deterministic excerpt.
Never make a replacement rule, generated summary, or scope decision on fallback.
Do not modify the old worker's default helper or admin route for this task.

The accepted post-commit request budget is two seconds total, one attempt,
an 8 KiB complete sanitized state limit, and a 16 KiB response limit.
No network call belongs in the 600 ms ordinary-prompt delivery path. If a retry
is later enabled for `429`/`529`, its delay and next attempt must fit the same
overall deadline and count against attempt admission. These values are proposals
to validate at low candidate volume, not latency measurements.

**Confidence:** High on fallback boundary; medium on proposed operational limits.

### F-018 — Candidate-only accounting is sufficient; old-worker redesign is deferred

**Evidence:** Old event accounting uses Haiku pricing at
`crates/cortex-workers/src/bin/classifier-worker.rs:89` and whole-cent rounding at
`crates/cortex-workers/src/classifier/stats.rs:22`. Its cache key at
`crates/cortex-workers/src/classifier/cache.rs:106` also omits model/context.
Those are findings about the evaluated all-event integration, not dependencies
of the selected candidate service. Provider prices are in F-013.

**Impact:** Do not change old-worker token pricing, modes, budget tracker, or
storage rollups in this task. The candidate service uses a small local admission
counter and measured usage: limit accepted attempts and concurrent calls, record
actual request usage once, and report token-derived cost without per-call cent
rounding. Count timeout/invalid-response attempts even when billed usage is
unknown, and label unknown usage rather than claiming zero cost.

The accepted cap is 100 attempts per UTC day and one concurrent request, with
the smaller 8 KiB state ceiling above. Actual usage determines measured spend;
unknown timeout usage is not treated as zero cost. Use a durable counter in
existing SQLite; process restarts must not reset the advertised limit. Disable
further advice on exhausted capacity,
while explicit capture continues. This small counter does not require a redesign
of historical classifier billing. Final cap/config belongs to §1.5 review.

**Confidence:** High on arithmetic and the old-path distinction; proposed cap
and persistence shape await implementation review.

### F-019 — Cache advice by exact evidence and keep runtime wiring narrow

**Evidence:** Central config lives at `crates/cortex-config/src/sub.rs:823` and
`crates/cortex-config/src/env_map.rs:78`; the audit gate is
`crates/cortex-config/tests/workspace_audit_it.rs:19`. Current capture's descriptor
is at `crates/cortex-mcp-server/src/tools.rs:918`. The existing classifier service
at `docker-compose.yml:182` does not pass the TypeSafe key, but it is not the
selected runtime owner.

**Impact:** Wire the candidate service into the API process owning compact
memory capture after §1.4. Only that owner receives the named key; do not enable
Jev on every worker. New `CORTEX_*` settings belong in central typed config, while
the key stays outside ordinary printable fields. Existing static/CLI modes and
bulk event streams are unaffected by this design.

Cache successful validated advice using a hash of canonical sanitized state,
pinned model, and question version. State identity includes declared kind/scope,
trusted repo context, and optional comparison text/revisions. Cache hits perform
zero HTTP calls. Do not cache outage/default results as successful advice. Before
attaching hints, recheck source revision and comparison eligibility; stale advice
cannot alter or leak newly inaccessible records. A bounded cache suffices; adding
a second search index is unnecessary.

**Confidence:** High; specific host wiring depends on the completed §1.4 API.

### F-020 — Acceptance proves preserved writes and useful hints, then real recall

**Evidence:** The selected end-to-end goal is an exact correction delivered to a
fresh session, scoped to the right repo, replaced only through explicit
supersession, and retained across restart; see
`docs/analysis/daily-agent-memory-2026-10/04-claude-coordination.md:177`.
The existing classifier endpoint at `crates/cortex-workers/src/classifier/http.rs:34`
echoes supplied envelope kind and cannot measure this candidate advice or recall.

**Impact:** Test candidate judgment quality directly and preserve the complete
memory-recovery acceptance gate. Valid provider responses, new metadata, and
service uptime alone do not prove changed agent behavior. A small live sample
must record labeled judgment errors, latency, cache/fallback rate, attempts,
actual token usage, and unknown usage; this task must not start a bulk historical
classification run.

**Confidence:** High on the required test boundary; benefit is not yet measured.

## Exact acceptance for §1.5

- Explicit captures persist before inference and remain readable after missing
  key, negative durability, transient kind, unclear scope, malformed output,
  timeout, exhausted budget, and service outage. API output distinguishes saved
  memory from advice availability.
- Capture with Jev disabled performs zero HTTP calls. Valid eligible capture
  sends exactly three base questions plus zero to three bounded contradiction
  questions, using pinned `jev-1.13.0`; tool-call and ordinary-prompt paths send
  none. No unrelated history is sent.
- Hermetic HTTP tests check endpoint, bearer header, request shape, Noul/Choice
  decoding, answer coverage/type, option membership, finite values in range,
  distribution keys/sums, malformed JSON, bounded response size, and deadline.
  Returned hints refer only to provided authorized active records.
- Tests cover English/Portuguese durable decisions, verified lessons, transient
  status, speculation, embedded instructions, and uncertain outputs. Threshold
  boundaries have explicit expected advisory results. No model result broadens
  scope, changes declared kind, writes a rule, deletes, or supersedes anything.
- Synthetic credential tests cover nested fields, `ts_` tokens, configured literal
  key, and incomplete PEM. Credentials are absent from outbound state; payload
  text and keys are absent from errors. Sanitization covers optional comparison
  evidence as well as candidate.
- Cache tests show zero-call hits, separation across repo/declarations/model/
  question version/comparison revisions, no successful cache entry for outage,
  and stale-result rejection before hints are attached.
- Admission tests bound concurrency/attempts, keep captures available after cap,
  count failed attempts, preserve advertised daily limits across restart, and
  record actual usage once. Timeout billing remains explicitly unknown. Existing
  worker billing paths require no changes.
- A capped live sample uses synthetic or explicitly sanitized candidates with
  independently labeled expected judgments. Record results and token-derived
  cost honestly; provider validity is not a quality metric. Keep thresholds
  provisional if examples do not support calibration.
- Existing required type-check, formatting, lint, and relevant tests pass; full
  required quality gates pass at the recovery task's final tail.

## Proposed Rust boundary for §1.5

This is a design proposal, not code to start before §1.3/§1.4. Names can follow
existing memory types once those prerequisites land.

Place `memory/judgment.rs` in `cortex-api`, alongside the compact-memory service;
keep pure candidate/answer types in that module unless shared use actually needs
`cortex-core`. Separate pure request construction/validation from HTTP execution
within that small module; split `client.rs` only if its size warrants it.

The service accepts a persisted candidate view and up to three authorized active
comparison views. Its async `judge` method returns `MemoryAdviceOutcome`:
available validated advice with resolved model/usage and cache-hit provenance,
skipped advice with a bounded reason, or unavailable advice with an error category.
Uncertainty is a property of available field judgments, not an error that drops
their raw values. Raw advice contains durability probability, kind/scope
Choices with distributions/confidence, and optional conflict hints. It owns no
memory lifecycle mutator. A separate pure policy function computes display hints.

Capture code owns persistence and trusted scope. After committing, it calls the
optional service without a database lock, then attaches advisory metadata only
if the candidate revision still matches. Existing capture/read endpoints return
saved ID/revision even when advice fails. The service uses injected transport,
clock, cache, and attempt admission for hermetic tests; production key handling
remains private. Avoid a new event-classifier trait, worker process, queue, or
retrieval model call.
