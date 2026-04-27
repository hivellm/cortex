# Classifier worker — wire Synap consumer/publisher

## ADDED Requirements

### Requirement: Classifier worker binary
The system SHALL provide a `cortex-classifier-worker` binary inside the
`cortex-classifier` crate that bridges raw/bootstrap event streams to
the enriched stream.

#### Scenario: Bootstrap event flows end-to-end
Given Synap is reachable at `http://127.0.0.1:15003`
And `cortex.events.bootstrap` contains one `artifact.code` envelope
When `cortex-classifier-worker` runs one iteration
Then exactly one `EnrichedEvent` envelope is published on `cortex.events.enriched`
And the source message offset is acked
And the published envelope has `classifier.source = "static_fallback"` (default mode)

#### Scenario: Live raw event flows end-to-end
Given `cortex.events.raw` contains one canonical `cortex_core::events::Envelope`
When `cortex-classifier-worker` runs one iteration
Then one `EnrichedEvent` is published on `cortex.events.enriched`
And the envelope's `kind`, `event_id`, `content_hash`, `redacted_payload` are preserved

#### Scenario: Replay is deduped within a single worker lifetime
Given the worker has already classified an event with a given `event_id`
When the same `event_id` is re-delivered by Synap
Then the worker acks the message without re-publishing on `cortex.events.enriched`

### Requirement: Kind mapping
The worker MUST map bootstrap event-kind strings onto canonical
`cortex_core::events::Kind` values.

#### Scenario: All bootstrap kinds map correctly
Given bootstrap events of kinds `artifact.code`, `artifact.doc`, `turn.historical`,
`decision.imported`, `law.imported`, `memory.imported`
When the worker derives the canonical `Kind`
Then `artifact.code` and `artifact.doc` map to `Kind::Artifact`
And `turn.historical` maps to `Kind::Turn`
And `decision.imported` maps to `Kind::Decision`
And `law.imported` maps to `Kind::LawViolation`
And `memory.imported` maps to `Kind::Memory`

### Requirement: Configurable backend
The worker MUST select its classifier backend via
`CORTEX_CLASSIFIER_MODE`, defaulting to the offline static fallback.

#### Scenario: Default mode is static fallback
Given `CORTEX_CLASSIFIER_MODE` is unset
When the worker classifies an event
Then the published `ClassifierOutput.source` is `"static_fallback"`
And no Claude CLI process is spawned

#### Scenario: CLI mode opts into Haiku
Given `CORTEX_CLASSIFIER_MODE=cli`
And `CLAUDE_CODE_BIN` resolves to a runnable executable
When the worker classifies an event
Then the worker invokes `HaikuCliClassifier`
And the budget tracker records the spend

### Requirement: Replay-safe dedup
The worker MUST track already-classified `event_id` values in-memory so
duplicate deliveries within a single worker lifetime do not double-publish.
