# Post-Backend-Update Validation

## ADDED Requirements

### Requirement: Dense lane contributes after the Vectorizer update
Once the deployed Vectorizer serves a dense provider (vectorizer#306), the
system SHALL re-index at dim 768 and a paraphrase/semantic `cortex_query`
MUST return at least one hit with `source: vector`, and
`cortex-eval --suite retrieval` MUST report MRR@10 ≥ 0.60.

#### Scenario: Semantic query surfaces a vector hit
Given the Vectorizer serves a dense provider at dim 768 and the corpus is re-indexed
When a paraphrase query (no lexical overlap with the target) runs through `cortex_query`
Then the fused results include a `source: vector` hit
And the top-K is topically relevant rather than the prior generic-file failure mode.

### Requirement: Inline-literal Cypher is removed after the Nexus fix
Once Nexus binds `$param` (nexus#3) and resolves property corruption
(nexus#4), the system SHALL replace every inline-literal +
`sanitize_literal` Cypher call site (graph writer, operator CLIs, HTTP
routes, lane templates) with parameterized queries, and the workaround
helpers MUST be deleted (not left dead). A property write→read round-trip
MUST persist every property.

#### Scenario: Parameterized Cypher binds
Given a Nexus that binds parameters
When `RETURN $name` runs with `{name:"x"}`
Then it returns `"x"` (not null)
And the graph writer's parameterized MERGE/SET persists node properties verified by read-back.

### Requirement: The blocked phase18 eval gates close on labelled data
The system SHALL author the labelled time-sensitive + cross-project query
corpus and run the phase18 §3.8 (temporal MRR +10%) and §5.4 (cross-project
positive delta) gates. Both gates MUST be recorded with measured numbers and
phase18 tasks.md MUST be flipped from blocked to done.

#### Scenario: Temporal gate meets the +10% floor
Given a labelled time-sensitive subset in tests/golden/retrieval.csv
When the temporal classifier is measured ON vs OFF
Then the MRR delta is ≥ +10%
And phase18 §3.8 is marked done with the measured delta.

### Requirement: Full hybrid acceptance is gated and CI is restored
The system SHALL prove a single `cortex_query` returns hits from all three
lanes (keyword + vector + graph), the full `cortex-eval` battery meets every
phase14c floor, and the CI workflows disabled during the degraded window are
re-enabled and pass.

#### Scenario: Hybrid is whole again
Given the dense + graph lanes are restored
When a `cortex_query` runs
Then the fused results contain hits with `source` in {keyword, vector, graph}
And re-enabled CI workflows (Doctor consistency gate, eval, Relevance harness gate) pass on the recovered stack.

### Requirement: Gated phases fail closed, not green, against a degraded backend
A phase whose upstream fix has not shipped MUST remain `blocked` (LAW-CORTEX-001
exemption 2) rather than being marked done. A gated validation item MUST NOT
report green while its precondition probe fails.

#### Scenario: Unshipped dense provider keeps P1 blocked
Given the deployed Vectorizer still reports provider `bm25` at dim 512
When P1 preconditions are checked
Then P1 stays blocked with a one-line reason
And no P1 item is marked done.
