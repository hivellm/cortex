# Spec: Knowledge + learnings walker

## ADDED Requirements

### Requirement: knowledge + learning kinds

The canonical envelope schema MUST accept `kind="knowledge"` and
`kind="learning"`. `knowledge` carries `payload.category ∈
{pattern, anti_pattern}`. Both kinds MUST flow through redaction
unchanged (they are operator-curated text, no PII expected).

#### Scenario: walker emits one envelope per file
Given `.rulebook/knowledge/` contains 20 markdown files and
  `.rulebook/learnings/` contains 40
When the bootstrap walker runs against the repo
Then exactly 60 new envelopes MUST be emitted
And 20 of them MUST carry `kind=knowledge`
And 40 of them MUST carry `kind=learning`.

### Requirement: dedicated single-tier collections + indexes

The storage layout MUST declare `cortex.knowledge.fp32`,
`cortex.learning.fp32` (single-tier, no PQ rollover — the corpus
is small and dense), `cortex_knowledge`, `cortex_learnings`,
`:Knowledge`, `:Learning`.

#### Scenario: bootstrap creates the new namespaces
Given a fresh stack
When `cortex-ops plan --slice all` is invoked
Then the JSON output MUST include the four collection /index /
  label entries listed above.

### Requirement: pre-thinking pulls the new corpora

For `pre_change_context` and `decision_lookup` intents, the
pre-thinking pipeline MUST query the knowledge + learning lanes
in addition to the existing snippet lane. The bundle MUST surface
at least one knowledge or learning row when one is relevant.

#### Scenario: pattern surfaces on a related prompt
Given a knowledge entry titled "RRF fusion blend tuning" exists
When the agent prompts "tune the rrf fusion blend"
Then the pre-thinking bundle MUST include that knowledge row
And the row MUST be tagged `kind=knowledge`.
