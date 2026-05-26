# Spec: relevance axes (recency, scope, model, session, outcome)

## ADDED Requirements

### Requirement: Five orthogonal axes shape RRF fusion

The query API SHALL apply five multiplicative axes to every RRF
score before fusion ranking. Each axis SHALL be configurable via
`crates/cortex-api/config/relevance.toml`; defaults SHALL preserve
current behaviour when an axis is unset.

#### Scenario: temporal recency decays older hits

Given two seeded Turns with identical content, one occurring 1 day ago, one 365 days ago
And `Scope.recency_decay` defaults for `pre_change_context` (λ=0.02)
When the caller fires `pre_change_context` with the matching query
Then the recent Turn MUST rank higher than the old Turn in the response
And the score difference MUST equal `exp(-λ * 364)` to within 1 % tolerance

#### Scenario: cross-repo boost preserves in-repo priority

Given seeded Turns in two repos `a` and `b`, all matching the query equally
And `Scope.repo="a"`, `Scope.cross_repo_boost=0.5`
When the caller fires `free_search`
Then the response MUST contain hits from BOTH repos
And every hit from repo `a` MUST rank higher than every hit from repo `b`

#### Scenario: model filter restricts hits

Given seeded Turns with `model` ∈ `{claude-opus-4-7, claude-haiku-4-5}`
And `Scope.models=["claude-opus-4-7"]`
When the caller fires `similar_problems`
Then every hit MUST have `model="claude-opus-4-7"`

#### Scenario: same-session boost surfaces current cohort

Given the caller is in session `S`
And there exist past Turns with `session_id=S` and other Turns from unrelated sessions
And `Scope.session_id="S"`
When the caller fires `pre_change_context`
Then the same-session Turns MUST rank higher than equivalent-content Turns from other sessions
And the multiplier applied MUST equal 2.0 to within 1 % tolerance

#### Scenario: outcome filter excludes errored turns

Given seeded Turns with `outcome` ∈ `{success, error, blocked_by_law}`
And `Scope.exclude_outcomes=["error", "blocked_by_law"]`
When the caller fires `similar_problems`
Then every hit MUST have `outcome="success"`

### Requirement: Settings v2 declares new filterable attributes

`crates/cortex-workers/src/fulltext/settings.rs` SHALL ship a v2
schema adding `model`, `tool`, `session_id`, and `outcome` to
Meilisearch's `filterableAttributes` array. Existing v1 indexes
SHALL be upgradeable in place via a bootstrap subcommand.

#### Scenario: --apply-settings-only upgrades v1 to v2

Given a Meilisearch instance running with v1 settings on `cortex-cortex-turns`
When the operator runs `cortex-bootstrap --apply-settings-only --repo cortex`
Then `cortex-cortex-turns` MUST be reconfigured with v2 `filterableAttributes`
And no document data SHALL be deleted or re-indexed by this operation
And subsequent queries MUST be able to filter on `model`, `tool`, `session_id`, `outcome`

### Requirement: Pre-thinking bundle exposes `Past sessions` and outcome glyphs

The pre-thinking renderer SHALL emit a `Past sessions` section
listing top-3 historically-similar sessions and SHALL prepend an
outcome glyph (`✓` / `✗` / `⚠`) to every turn / decision line.
Both additions SHALL respect the existing 32 KiB byte-budget
clipper.

#### Scenario: bundle stays under budget after additions

Given a query producing > 50 candidate hits across all sections
When the renderer produces the bundle
Then the rendered byte length MUST be ≤ 32 KiB by default
And the `clipped` field SHALL describe what was tail-dropped
And the `Past sessions` section MUST contain ≤ 3 entries

#### Scenario: outcome glyph reflects classifier label

Given a Turn with `outcome="error"`
When the renderer outputs the line
Then the line MUST start with `✗ `
And the equivalent for `outcome="success"` MUST start with `✓ `
And the equivalent for `outcome="blocked_by_law"` MUST start with `⚠ `

### Requirement: Relevance gold set gates CI

A hand-curated 30-question gold set SHALL live at
`crates/cortex-api/tests/fixtures/relevance-gold.json`. CI SHALL
run an integration test that computes `mrr@10` and `ndcg@10`
against the gold set; the test SHALL fail when `mrr@10 < 0.75`.

#### Scenario: relevance_eval_it asserts the gate

Given the gold set with 30 questions
When `cargo test -p cortex-api --test relevance_eval_it` runs with `CORTEX_RELEVANCE_IT=1`
Then the test MUST report `mrr@10` and `ndcg@10` to stdout
And the test MUST fail when `mrr@10 < 0.75`
And the test MUST pass when `mrr@10 >= 0.75`
