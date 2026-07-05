# Scheduled long-lived-stack doctor suite and registered-surface freshness

## ADDED Requirements

### Requirement: Doctor suite runs on a schedule against a long-lived stack
The full operator doctor suite (health, versions, config, canary) SHALL
run on a recurring schedule against a stack that has been running for
longer than the CI-boot window, in addition to running at PR time.

#### Scenario: Scheduled run detects a worker stalled for days
Given a worker's consume loop silently stops processing after several days of uptime while its container HEALTHCHECK still reports healthy
When the next scheduled doctor run executes
Then it MUST detect and report the stalled worker via its freshness/activity check, not just its container-health check

### Requirement: Registered surfaces are checked for recent activity, not just registration
Every MCP tool and every worker's consume loop MUST be checked, during
the scheduled doctor run, for having been active within a defined recent
window; the run MUST fail when any registered surface has gone silent
within that window.

#### Scenario: A registered MCP tool that nobody calls is caught
Given an MCP tool is registered in `ToolRegistry::default_set()` but has not been invoked within the defined recent window
When the scheduled doctor run executes its registered-surface check
Then it MUST report that tool as silently unexercised instead of only confirming it is present in the registry
