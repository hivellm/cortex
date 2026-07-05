# Quality Specification

## ADDED Requirements

### Requirement: Unknown-language chunking preserves source text
The chunker SHALL return the original raw source text, not an empty or placeholder value, when it falls back for an unrecognized language.

#### Scenario: Fallback chunk matches raw source
Given a source file in a language the chunker doesn't recognize
When the chunker processes it
Then the returned chunk's content MUST equal the original file's raw text

### Requirement: Operator doctor tooling runs correctly on native Windows
`cortex-ops doctor` SHALL report accurate pass/fail status for each backend health check when run on a native Windows host, not just inside Linux containers.

#### Scenario: Doctor reports true status on native Windows
Given cortex-api and its dependent services are healthy and reachable
When an operator runs `cortex-ops doctor` on a native Windows machine
Then every check MUST report its true status, not a false failure caused by a POSIX-only code path

### Requirement: Graph-worker health reporting reflects consume-loop staleness
The system SHALL treat a Nexus-consumer loop that has stalled beyond the configured freshness threshold as unhealthy and SHALL apply an automated recovery safeguard.

#### Scenario: Sustained stall triggers degraded status and recovery
Given the cortex-graph-worker's Nexus-consumer loop has made no progress for longer than the configured freshness-degraded threshold
When an operator or supervisor checks worker health
Then the health check MUST report a non-healthy status instead of "healthy", and the configured stall-recovery safeguard MUST engage

### Requirement: Workspace dependencies carry no known vulnerabilities
The workspace SHALL have zero unresolved `cargo audit` advisories for its dependency tree.

#### Scenario: Audit is clean after the quinn-proto and rmcp upgrades
Given quinn-proto and rmcp are upgraded to their patched versions
When `cargo audit` is run against the workspace
Then it MUST report zero vulnerabilities for quinn-proto and rmcp
