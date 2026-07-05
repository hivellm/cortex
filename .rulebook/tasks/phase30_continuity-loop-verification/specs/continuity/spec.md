# Continuity Specification

## ADDED Requirements

### Requirement: Session Consolidation Retrievable In The Next Session
The system SHALL make a session's consolidation (session, topic, or decision-trace grain) retrievable via `cortex_pre_thinking` from the immediately following session scoped to the same repository, for a query related to that consolidation's content.

#### Scenario: Consolidation from session A surfaces in session B's pre-thinking bundle
Given session A produces a consolidation about a specific technical decision
When session B, a fresh session scoped to the same repo, asks a `cortex_pre_thinking` question related to that decision
Then the consolidation from session A MUST appear in session B's returned bundle

### Requirement: Active In-Flight Work Surfaces Automatically At Session Start
The system SHALL surface a summary of active in-flight Rulebook tasks (equivalent to `cortex_active_work`'s output) automatically when a new session starts for a repository, without requiring the agent to explicitly invoke a retrieval tool first.

#### Scenario: New session surfaces in-flight tasks without an explicit query
Given a repository has one or more in-flight Rulebook tasks tracked by `cortex_active_work`
When a new session starts and the `SessionStart` hook fires for that repository
Then the session's initial context MUST include a summary of those in-flight tasks before the agent issues any explicit query
