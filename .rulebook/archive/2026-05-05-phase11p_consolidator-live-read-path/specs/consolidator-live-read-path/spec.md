# Spec: consolidator live read path

## ADDED Requirements

### Requirement: Live source modules feed every producer

The system SHALL expose three `Source` types — `LiveSessionSource`, `LiveTopicSource`, `LiveDecisionTraceSource` — that materialise `SessionInput`, `Vec<TopicCluster>`, and `DecisionTraceInput` from live Synap + Vectorizer + Nexus reads.

#### Scenario: session source returns ordered envelopes

Given a session_id with 30 envelopes recorded out of order in Synap
When `LiveSessionSource::fetch(session_id)` runs
Then the returned `SessionInput.envelopes` MUST be sorted by `occurred_at` ascending
And the `repo` field MUST be derived from the majority repo across the envelope set

#### Scenario: empty session is a typed error

Given a session_id with zero envelopes in Synap
When `LiveSessionSource::fetch(session_id)` runs
Then it MUST return `SourceError::EmptyResult`
And it MUST NOT call the Anthropic Messages API

#### Scenario: topic source clusters via HDBSCAN with min_cluster_size = 3

Given 12 turn-digest embeddings inside a `[since_ts, until_ts]` window for repo `cortex`
And the embeddings split into one 7-session cluster, one 5-session cluster, and one outlier
When `LiveTopicSource::fetch("cortex", since_ts, until_ts)` runs
Then the result MUST contain exactly 2 `TopicCluster` rows
And the outlier (label = -1) MUST NOT appear in any cluster
And the cluster sizes MUST be 7 and 5

#### Scenario: decision trace stops at MAX_HOPS

Given a decision envelope whose `parent_event_id` chain extends 20 hops deep
When `LiveDecisionTraceSource::fetch(decision_id)` runs
Then the returned `DecisionTraceInput.chain` MUST contain at most `MAX_HOPS = 16` envelopes (root-first)
And the original decision envelope MUST be carried in `DecisionTraceInput.decision`

#### Scenario: decision trace detects parent cycles

Given a decision envelope whose `parent_event_id` chain forms a cycle
When `LiveDecisionTraceSource::fetch(decision_id)` runs
Then the function MUST return `SourceError::Storage`
And the error message MUST name the cycling pair `(parent_event_id, child_event_id)`

### Requirement: cortex-consolidator binary dispatches against the live sources

The `cortex-consolidator` binary SHALL replace the `pending §3 routing wiring` stubs in every operational subcommand with calls into the §1 live sources.

#### Scenario: run-session emits a real consolidation row

Given a Synap session with ≥ 1 envelope
And `ANTHROPIC_API_KEY` is set
When `cortex-consolidator run-session <session_id>` runs
Then exactly one row MUST land in the `cortex_consolidations` Meili index keyed by the deterministic `consolidation_id`
And the binary MUST exit 0
And the binary MUST print the `consolidation_id`, `cost_cents`, and `source_event_count`

#### Scenario: nightly cursor is durable across runs

Given a previous nightly run wrote `<home>/.cortex/consolidator-cursor.json` with `last_run_ts = T0`
When `cortex-consolidator nightly` runs at T1
Then it MUST enumerate sessions, topics, and decisions in `(T0, T1]`
And it MUST atomically rewrite the cursor with `last_run_ts = T1` only after every producer dispatch returns

### Requirement: Cron schedule closes the consolidate → demote → purge loop

The system SHALL ship a cron seed that runs the live consolidator nightly before the pruner sweep.

#### Scenario: consolidator nightly seeds at 02:00, before the 03:00 pruner

Given a fresh `cortex-storage` metadata DB
When `seed_defaults(&store, now)` runs
Then a seed named `retention.consolidator_nightly` MUST exist with `schedule = "0 2 * * *"` and `enabled = true`
And the seed `retention.consolidation_prune` (03:00) MUST also be enabled
And `retention.memory_consolidate` MUST be enabled (flipped from the prior phase11j default)

## MODIFIED Requirements

### Requirement: cortex-consolidator subcommand status surface

The four operational subcommands (`run-session`, `run-topic`, `run-decision`, `nightly --dry-run=false`) MUST NOT print `status: pending §3 routing wiring`. They MUST surface real producer dispatch results.
