# Ship-then-dead-wire: features land unit-tested but disconnected from the live path

**Category**: architecture
**Tags**: cortex, observability, analysis:cortex-platform-2026-07

## Description

Recurring failure mode in Cortex: a feature passes unit tests and merges, but is never actually wired into the live/prod path, and this goes undiscovered for weeks or months. Confirmed instances: the phantom-link verifier landed dead-wired and was only connected at boot in a later phase; pre-thinking cache counters were invisible cross-process; the adapter daemon was simply not running while everything else looked fine; and (found via live testing 2026-07-05) cortex-graph-worker's Nexus-consumer loop silently stopped processing on 2026-06-27 while Docker kept reporting the container "healthy" for the following 8 days because its HEALTHCHECK only probes that /healthz responds, not that the work loop progresses.

## When to Use

Whenever shipping a new worker, consumer, background daemon, or any feature with a "wire it up at boot/deploy" step separate from the code that implements it.

## When NOT to Use

Purely synchronous request/response code paths with no background loop or separate wiring step to silently skip.
