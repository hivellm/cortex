# Standalone Dashboard Continuity

## ADDED Requirements

### Requirement: Existing dashboard visual structure remains authoritative
The standalone web interface SHALL reuse the existing gui renderer's shell,
design tokens and component patterns. Functional adaptations MUST preserve the
established layout and navigation structure rather than introduce another design.

#### Scenario: Opening the standalone browser interface
Given the user starts cortex serve
When the authenticated startup URL is opened
Then the existing Cortex dashboard header, sidebar and visual language MUST appear
And supported views MUST operate on the local store without required services

### Requirement: Local mode keeps data and credentials isolated
The renderer MUST retain exact repository scope and same-origin session-token
authentication. It MUST NOT persist the transient token in legacy connections,
send it in API URLs or issue unsupported legacy service requests.

#### Scenario: Switching exact repository scope
Given records exist only in repository Cortex
When the user selects cortex
Then cached records, editors, audit selections and feedback targets MUST reset
And data from Cortex MUST NOT appear as data from cortex

### Requirement: Embedded assets are reproducible and self-contained at runtime
The build SHALL embed the real dashboard renderer and serve manifest-listed local
assets. The installed executable MUST NOT require Node, Electron or Docker.

#### Scenario: Missing asset or unauthenticated local request
Given the embedded server is running
When an unknown asset or unauthenticated API operation is requested
Then it MUST return an explicit failure without leaking local data or credentials
