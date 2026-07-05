# Governance

## ADDED Requirements

### Requirement: Blocking law violations are detected and recorded before the offending action completes
For any Law marked `severity: critical`, the governance engine SHALL evaluate its detector synchronously at the equivalent of `PreToolUse` time and, on violation, SHALL persist a `LawViolation` envelope through the normal ingestion path before the offending tool call is allowed to complete.

#### Scenario: Critical law blocks and records a matching tool call
Given a critical-severity Law with a detector that matches a specific tool-call pattern
When an agent attempts that exact tool call
Then a LawViolation envelope MUST be persisted and MUST be retrievable via cortex_law_violations immediately afterward

### Requirement: Observational law violations are recorded without blocking the tool call
For any Law not marked `severity: critical`, the governance engine SHALL evaluate its detector asynchronously against the enriched event stream and SHALL persist a `LawViolation` envelope on a match without delaying or rejecting the originating tool call.

#### Scenario: Non-critical law records a violation after the fact
Given a notable-severity Law with a detector that matches a specific tool-call pattern
When an agent completes that exact tool call
Then the tool call completes without being blocked
And a LawViolation envelope MUST be persisted and MUST be retrievable via cortex_law_violations within one observational evaluation cycle

### Requirement: A recorded violation enqueues a reminder for the offending session's next turn
When a `LawViolation` is persisted for a law configured at tier 2, the governance engine SHALL enqueue a reminder scoped to the offending `session_id`, and the pre-thinking bundle assembly path SHALL include that reminder the next time it assembles a bundle for that session, until the reminder expires or is consumed.

#### Scenario: Tier-2 violation surfaces a reminder on the next turn
Given a tier-2 LawViolation was just persisted for session S
When session S's next pre-thinking bundle is assembled
Then the bundle's Laws section MUST include a reminder referencing the violated law

### Requirement: Trust score is recomputed nightly per (model, repo)
The governance engine SHALL recompute a trust score for every `(model, repo)` pair observed in the trailing 30 days on a nightly schedule, derived from violation history and decision-following accuracy, and SHALL support an on-demand recompute scoped to a single `(model, repo)` pair.

#### Scenario: Nightly job updates a pair's trust score
Given a (model, repo) pair accumulated at least one new LawViolation since the last recompute
When the nightly trust-score job runs
Then the pair's stored trust score MUST reflect the new violation and MUST be retrievable with an updated last_computed_at

### Requirement: The dashboard Laws view reads from the live governance engine
The dashboard's Laws view and trust-score route SHALL read the active law catalogue and trust scores from the live governance engine's own stores rather than from a catalogue derived from historical violation envelopes.

#### Scenario: Dashboard reflects a newly loaded law with zero violations
Given a Law is loaded into the live registry but has never been violated
When the dashboard's Laws view is requested
Then the newly loaded law MUST appear in the active-law list even though no LawViolation envelope references it
