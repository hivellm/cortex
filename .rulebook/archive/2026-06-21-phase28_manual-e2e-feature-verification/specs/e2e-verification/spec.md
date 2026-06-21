# E2E manual verification

## ADDED Requirements

### Requirement: Deployed stack matches committed code before verification
The verification process SHALL rebuild and recreate any service image
that lags `HEAD` before its features are probed, so a probe never passes
or fails against a stale binary.

#### Scenario: Stale container is refreshed before probing
Given a committed change to a service that is not yet in its running container
When the verification run reaches that service's probes
Then the image is rebuilt and the container recreated before the probe runs

### Requirement: Every feature probe has an explicit expected result
Each checklist item SHALL state a concrete probe (command or MCP tool)
and an expected result, and MUST be marked passing only when the actual
result matches the expected result.

#### Scenario: A failing probe is not silently skipped
Given a probe whose actual result does not match the expected result
When the operator records the outcome
Then the item is marked failing with the actual output and a follow-up fix task is created
