# Spec: topic-card living-synthesis tier

## ADDED Requirements

### Requirement: TopicCard envelope is a first-class Kind

The system SHALL expose a `Kind::TopicCard` variant carrying a
`TopicCardPayload` that summarises a topic via LLM-maintained
synthesis and MUST round-trip through the existing ingestion +
indexing pipeline without schema-shape changes outside the new
variant + payload.

#### Scenario: serde round-trip preserves every field

Given a `TopicCardPayload` with non-empty `evidence`, `contradictions`, and `open_questions`
When the envelope is serialised and deserialised via `serde_json`
Then every field MUST be preserved byte-for-byte
And `validate_event` MUST return `Ok` for the round-tripped envelope

#### Scenario: schema_stem maps to canonical filename

Given a `Kind::TopicCard` envelope
When `Kind::schema_stem()` is called
Then it MUST return `"topic_card"`

#### Scenario: derive_topic_card_id is deterministic per slug + scope

Given a `topic_slug` and `repo_scope`
When `derive_topic_card_id(slug, scope)` is called twice
Then both invocations MUST return the same `TopicCardId`
And the id MUST start with `topic-` followed by 24 hex chars

### Requirement: TopicCard payload validates cross-field rules

The validator SHALL enforce that `evidence` is non-empty and every
`contradictions[*].evidence_a` / `evidence_b` references an item in
`evidence`.

#### Scenario: empty evidence is rejected

Given a TopicCardPayload with `evidence: []`
When `validate_topic_card_payload` runs
Then validation MUST fail with reason `evidence_required`

#### Scenario: orphan contradiction reference is rejected

Given a TopicCardPayload where `contradictions[0].evidence_a` is not in `evidence`
When `validate_topic_card_payload` runs
Then validation MUST fail with reason `contradiction_references_unknown_evidence`

#### Scenario: synthesis below floor is rejected

Given a TopicCardPayload with `synthesis_markdown` of 199 bytes
When the JSON Schema validator runs
Then validation MUST fail with path `/payload/synthesis_markdown`

### Requirement: TopicCards route to a dedicated family

`Kind::TopicCard` SHALL route to the new `topic_cards` family on
every backend.

#### Scenario: fulltext routing emits to global + per-repo Meili indexes

Given a TopicCard envelope with `repos=["cortex"]`
When the fulltext worker indexes it
Then a document MUST appear in `cortex_topic_cards` (global index)
And a document MUST appear in `cortex-cortex-topic-cards` (per-repo index)

#### Scenario: embedder routing emits to topic_card collection

Given the same envelope
When the embedder worker indexes it
Then chunks MUST land in `cortex.topic_card.fp32` (hot tier)
And the per-repo Vectorizer collection `cortex-cortex-topic_cards` MUST receive the same chunks

#### Scenario: graph mapper emits :TopicCard with EVIDENCE_OF edges

Given a TopicCard with 3 evidence items (Decision, Law, Consolidation)
When the graph worker indexes it
Then Nexus MUST contain a `:TopicCard` node with `topic_card_id` as the primary key
And exactly 3 `EVIDENCE_OF` edges MUST connect that node to the typed evidence nodes
And every `related_topic_ids` entry MUST produce a bidirectional `RELATED_TO` edge

### Requirement: Reactive trigger rewrites cards on relevant evidence

`Trigger::evaluate` SHALL return `Rewrite` when any of three
conditions hold; otherwise it returns `Hold { reason }`.

#### Scenario: events-since-last-rev threshold fires rewrite

Given a TopicCard with `events_since_last_rev = 8`
And a new arbitrary evidence event
When `Trigger::evaluate` runs
Then it MUST return `Rewrite`

#### Scenario: low-distance high-impact event fires rewrite

Given a TopicCard whose synthesis is semantically close to a new `Kind::Decision` event (distance 0.20)
When `Trigger::evaluate` runs with that distance and event
Then it MUST return `Rewrite`

#### Scenario: stale card with new evidence fires rewrite

Given a TopicCard with `synthesis_age_d = 14`
And one new evidence event of any kind
When `Trigger::evaluate` runs
Then it MUST return `Rewrite`

#### Scenario: non-relevant low-impact event holds

Given a TopicCard with `events_since_last_rev = 2`
And a low-impact `Kind::Turn` event with embedding distance 0.80
When `Trigger::evaluate` runs
Then it MUST return `Hold { reason: NotRelevant }`

### Requirement: Contradictions surface explicitly

`ContradictionScanner::scan` SHALL surface three classes of
contradictions; the scanner MUST never block a rewrite.

#### Scenario: decision supersession surfaces a contradiction

Given evidence containing two `Kind::Decision` items where item A's `supersedes` field equals item B's id
When `ContradictionScanner::scan` runs
Then the result MUST contain a `Contradiction` of kind `DecisionSupersession` with status `Open`
And it MUST reference both decision ids

#### Scenario: law-violation version mismatch surfaces a contradiction

Given evidence containing a `Kind::LawViolation` citing law version 1.0 and a `Kind::Law` evidence with active version 1.2
When `ContradictionScanner::scan` runs
Then the result MUST contain a `Contradiction` of kind `LawViolationMismatch`

#### Scenario: outcome-divergence surfaces a contradiction

Given two `Kind::Consolidation` evidence items with overlapping `temporal_span` and conflicting `outcome_distribution` majorities (one mostly success, one mostly error)
When `ContradictionScanner::scan` runs
Then the result MUST contain a `Contradiction` of kind `OutcomeDivergence`

### Requirement: Five MCP tools expose topic cards to the model

The system SHALL expose five MCP tools: `cortex_topic_get`,
`cortex_topic_drill`, `cortex_topic_neighbors`, `cortex_topic_diff`,
`cortex_synthesize`. All tools MUST emit audit envelopes.

#### Scenario: cortex_topic_get with exact slug short-circuits hybrid search

Given a TopicCard with `topic_slug = "auth-middleware-rewrite"`
When `cortex_topic_get("auth-middleware-rewrite", scope)` is called
Then the tool MUST return that exact card without running hybrid search
And the audit envelope MUST record `path = "slug-exact"`

#### Scenario: cortex_topic_get with query runs hybrid search

Given a query string that does not match any slug regex
When `cortex_topic_get(query, scope)` is called
Then the tool MUST run hybrid search (fulltext + vector RRF) over `cortex_topic_cards`
And it MUST return `Some(card)` only when the top-1 match has confidence ≥ 0.6
And it MUST return `None` otherwise

#### Scenario: cortex_topic_drill returns evidence hydrated with title and timestamp

Given a TopicCard with 5 evidence items
When `cortex_topic_drill(id, Evidence)` is called
Then the response MUST contain 5 hydrated entries
And each entry MUST carry `title` and `occurred_at` resolved from the source envelope

#### Scenario: cortex_topic_neighbors clips at 64 nodes

Given a TopicCard with a transitive neighborhood of 200 nodes within depth 2
When `cortex_topic_neighbors(id, depth=2)` is called
Then the response MUST contain at most 64 nodes
And the response MUST flag `clipped = true`

#### Scenario: cortex_synthesize counts against the cost budget

Given the monthly cost budget has 50 cents remaining
And `cortex_synthesize` would cost ≥ 100 cents (Haiku estimate)
When the tool is called
Then it MUST return a `BudgetExhausted` error
And no synthesis call MUST reach Anthropic

### Requirement: Pre-thinking prefers topic cards over consolidations

The pre-thinking renderer SHALL place a `Topic card` section above
the `Consolidated context` section when ≥ 1 topic card matches the
query scope and the card is not stale.

#### Scenario: matched topic card replaces the consolidation block

Given a query with 1 matching TopicCard (confidence 0.85, age 5 d) and 3 matching consolidations
When the pre-thinking renderer produces the bundle
Then the bundle MUST contain a `## Topic card` section
And the topic card section MUST appear BEFORE the consolidation section
And the topic card section MUST be ≤ 1 400 bytes (`section_caps::TOPIC_CARDS`)

#### Scenario: stale topic card downgrades to fallback

Given a query with 1 matching TopicCard whose `confidence = 0.55`
When the pre-thinking renderer produces the bundle
Then the bundle MUST emit a `> stale-topic-card: low-confidence` advisory line
And the consolidation section MUST render BEFORE the topic card section

#### Scenario: zero topic cards falls back to phase11j ordering

Given a query with 0 matching topic cards and ≥ 1 matching consolidation
When the pre-thinking renderer produces the bundle
Then the bundle MUST NOT contain a `## Topic card` section
And the section ordering MUST match the phase11j contract (consolidations before similar turns before past sessions)

### Requirement: Cost telemetry tracks topic-card synthesis under a dedicated grain

`cortex-topic-cards` SHALL stamp every realised cost into the
`CostLedger` under the `topic_card` grain bucket so the
`/v1/health/coverage` endpoint surfaces the burn separately from
consolidations.

#### Scenario: realised cost lands in topic_card bucket

Given the orchestrator runs a Haiku rewrite that costs 80 cents
When the run completes
Then `CostLedger::get("topic_card")` MUST equal 80 cents
And `CostLedger::get("session")` MUST be unchanged

#### Scenario: budget cap blocks rewrite before API call

Given the monthly cap is 100 cents and the ledger holds 50 cents under any grain
When the orchestrator gates a Haiku run with a 100-cent estimate
Then the gate MUST return `CostCeiling`
And no Anthropic API call MUST be made
