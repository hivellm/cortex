# Docs As Graph

## ADDED Requirements

### Requirement: Deterministic document graph
The system SHALL parse the documentation corpus into `document`/`article` and `entity`
nodes, parse YAML frontmatter, and emit a `related` edge for every `[[wikilink]]`,
before any LLM annotation runs.

#### Scenario: Wikilink becomes a related edge
Given a markdown doc containing `[[other-note]]`
When the deterministic doc pass runs
Then a `related` edge from the doc to `other-note` is emitted

### Requirement: Conservative claim and contradiction extraction
The system SHALL surface `claim` nodes and implicit `builds_on`/`contradicts`/`cites`
edges only when there is explicit textual evidence, MUST store that evidence in the
edge `description`, and MUST NOT duplicate edges already produced from wikilinks.

#### Scenario: Contradiction requires evidence
Given two docs that are merely thematically similar with no contradicting statement
When the annotation pass runs
Then no `contradicts` edge is created between them

#### Scenario: Evidence is recorded
Given a doc whose text explicitly contradicts a claim in another doc
When a `contradicts` edge is created
Then its `description` contains the supporting evidence span

### Requirement: Doc-versus-decision drift alert
The system SHALL detect when a document contradicts a currently accepted decision while
citing a superseded one, and surface it through a fail-open pre-thinking band.

#### Scenario: Drift against an accepted ADR
Given an accepted decision that supersedes an older one
And a doc that contradicts the accepted decision while citing the superseded one
When pre-thinking assembles context
Then the drift is surfaced as a contradiction signal

#### Scenario: Band fails open
Given the contradiction band encounters an internal error
When pre-thinking assembles context
Then the band contributes empty content and the session is not broken
