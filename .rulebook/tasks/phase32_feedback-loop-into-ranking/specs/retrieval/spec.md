# Retrieval

## ADDED Requirements

### Requirement: Recorded feedback signals influence subsequent ranking for similar queries
Helpful/unhelpful feedback recorded via `cortex_feedback_record` SHALL measurably adjust the RRF lane weighting applied to subsequent queries of the same intent, within a bounded adjustment range.

#### Scenario: Repeatedly unhelpful lane is down-weighted within bounds
Given a lane (e.g. graph) is repeatedly marked unhelpful for a specific intent
When subsequent queries of that same intent are ranked
Then that lane's contribution to the fused ranking MUST be measurably down-weighted relative to its default
And MUST remain within the configured bounds (not permanently zeroed by a small number of signals)

### Requirement: Bundle-level feedback is attributed to the lane(s) responsible before it adjusts ranking
The system SHALL record, per `query_id`, which lane contributed each hit in the bundle a feedback signal refers to, so that a bundle-level `helpful`/`unhelpful` verdict MUST be attributable to the specific lane(s) whose hits were present in that bundle before any lane-weight adjustment is derived from it.

#### Scenario: Unhelpful feedback is not misattributed to an uninvolved lane
Given a bundle whose hits came only from the vector and keyword lanes for query Q
When feedback recorded against Q's query_id is helpful: false
Then the derived weight adjustment MUST apply only to the vector and/or keyword lanes
And MUST NOT apply to the graph lane, which contributed no hits to that bundle

### Requirement: Feedback-driven ranking adjustment is disabled by default
The feedback-driven lane-weight adjustment SHALL be gated behind a configuration flag that defaults to disabled, so ranking behavior is unchanged from the current static-weight baseline until the flag is explicitly enabled.

#### Scenario: Ranking is unaffected when the feature flag is off
Given the feedback-loop feature flag is at its default disabled value
When a query is ranked
Then the fused ranking MUST be identical to the ranking produced before this feature existed, regardless of any recorded feedback signals
