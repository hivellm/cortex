# Pre-Thinking Specification

## ADDED Requirements

### Requirement: Pre-Thinking Bundle Traceable From Query Id To Consuming Turn
The system SHALL persist, for every `cortex_pre_thinking` call, its `query_id`, a summary of the assembled bundle's content, and (where determinable) the downstream turn(s) that consumed it, durably and queryably beyond the bounded in-memory audit ring buffer's retention window.

#### Scenario: Trail entry survives eviction from the in-memory ring buffer
Given a `cortex_pre_thinking` call occurred more than 1024 queries ago, past the in-memory ring buffer's capacity
When an operator looks up that query_id in the durable trail
Then the bundle summary and any linked downstream turns MUST still be retrievable

### Requirement: Downstream Turns Linked To The Preceding Pre-Thinking Query
The system SHALL link, where determinable, each persisted `query_id` to the downstream tool call(s) or turn(s) that followed it within the same session, using a best-effort heuristic when an exact causal link cannot be established.

#### Scenario: Trail entry references at least one downstream tool call
Given a `cortex_pre_thinking` call occurs within an active session
When the agent issues at least one subsequent tool call in that same session
Then the durable trail entry for that query_id MUST reference at least one downstream tool-call or turn identifier

### Requirement: Operators Can Browse The Pre-Thinking Trail In The Dashboard
The system SHALL expose a dashboard view listing persisted pre-thinking trail entries, each showing its `query_id`, bundle summary, and any linked downstream turns.

#### Scenario: Operator opens the pre-thinking trail view
Given at least one durably-persisted pre-thinking trail entry exists
When an operator opens the pre-thinking audit trail dashboard view
Then that entry's bundle summary and any linked downstream turns MUST be visible

### Requirement: Bundle Utilization Is Computed And Surfaced As A Metric
The system SHALL compute, for each durable trail entry, whether any file, decision, or law cited in its bundle was referenced again in the linked downstream turns, and SHALL surface an aggregate utilization metric from these computations.

#### Scenario: Trail entry is labeled utilized or not utilized
Given a persisted pre-thinking trail entry whose bundle cited at least one decision or file
When the bundle-utilization heuristic runs over that entry's linked downstream turns
Then the entry MUST be labeled either "utilized" or "not utilized" and included in an aggregate utilization metric queryable by an operator
