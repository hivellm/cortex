# Local Feedback and Ranking

## ADDED Requirements

### Requirement: Feedback retains explicit local attribution
The system SHALL persist feedback in canonical local records with exact repository,
query/delivery identity, provenance and explicit judgment. Record-specific feedback
MUST identify a record ID and revision in that delivery. Unknown or legacy
attribution MUST remain visible and ineligible for ranking changes.

#### Scenario: Cross-repository attribution is rejected
Given a delivery belongs to repository A
When feedback claims that delivery or its records belong to repository B
Then the system MUST reject that attribution without changing either repository's ranking

#### Scenario: Bundle feedback does not assign record blame
Given a delivery contains several records and receives a negative bundle verdict
When no specific record is identified by explicit feedback
Then the verdict MUST remain bundle-level
And missing citations MUST NOT be treated as negative record or lane feedback

#### Scenario: Legacy feedback lacks a matching delivery
Given imported legacy feedback has no verifiable repository and delivery association
When it is inspected or the index is rebuilt
Then its source and unknown attribution MUST remain available
And it MUST NOT contribute to ranking adjustment

### Requirement: Ranking adjustment is optional, bounded and reproducible
The system SHALL disable feedback adjustment by default. When explicitly enabled,
it SHALL apply a documented deterministic rule with minimum evidence threshold,
decay and bounds. Duplicate feedback MUST NOT amplify the signal. Feedback MUST
NOT change memory authority, source content, repository scope or supersession.

#### Scenario: Disabled mode preserves the baseline
Given feedback exists and adjustment is disabled
When the same query is executed against the same corpus and configuration
Then result identity, ordering and scores MUST equal the baseline without feedback adjustment

#### Scenario: Replay and reindex preserve bounded adjustments
Given enough eligible feedback exists for an enabled adjustment
When the same feedback is retried and the derived index is rebuilt
Then the effective signal MUST remain unchanged by duplicate replay
And the adjustment MUST remain within its documented bounds and repository scope

### Requirement: Operators can inspect and evaluate ranking effects
The system SHALL expose feedback, eligibility and applied adjustments through
local CLI/MCP and embedded GUI inspection. Evaluation SHALL compare enabled and
disabled ranking on the locked local corpus, with per-query quality and latency
results. No improvement SHALL be claimed from feedback counts alone.

#### Scenario: Adjustment makes retrieval worse
Given enabled-versus-disabled evaluation reports a quality regression
When the report is produced
Then the regression MUST remain visible and the default MUST remain disabled
And disabling adjustment MUST restore baseline behavior without deleting feedback or source memories

#### Scenario: Temporal correlation is the only evidence
Given a later action occurs after a prepared memory delivery without explicit use evidence
When feedback eligibility is evaluated
Then the correlation MUST NOT become an explicit helpful or unhelpful judgment
