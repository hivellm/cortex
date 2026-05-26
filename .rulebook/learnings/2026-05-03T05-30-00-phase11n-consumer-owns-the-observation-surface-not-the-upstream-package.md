The original phase11n premise was "open an issue against `hivellm/rulebook` and add `synap-sdk` (JS) to its handlers so every memory write publishes a Cortex-shaped envelope". That premise pollutes a generic third-party MCP package with consumer-specific stream names + envelope shapes (`cortex.events.dashboard`, the spec 21 envelope). It also adds a release-cadence dependency: every Cortex schema bump becomes a pending Rulebook PR.

Rejected. The right boundary: Rulebook writes to its own files / DB; Cortex (the consumer that cares about dashboard push) owns the read-side observation. Both surfaces in the revised proposal — FS watcher extension to `.rulebook/learnings/**` and a SQLite tail loop on `.rulebook/memory/memory.db` — live entirely inside `cortex-api`.

Pattern: when adding observability for an upstream service's writes, the observation surface lives in the OBSERVING service, not the OBSERVED one. Reasons:

1. Coupling — the observed service inherits the consumer's stream contract. Schema bumps in the consumer require coordinated releases of the observed package.
2. Generic packages stay generic — `@hivehub/rulebook` ships to projects that aren't Cortex; they would not benefit from the Synap publisher.
3. The observation strategy (FS watcher cadence, SQLite read-only tail, `last_rowid` seeding to avoid history replay) is consumer-specific. Encoding it in the observed service forces every other consumer to either inherit it or carry override flags.

Specific to phase11n: SQLite's WAL mode means `notify` watching the `.db` file fires on every WAL flush whether or not a row was committed, and even when it does fire it cannot tell the GUI which row appeared. A polling tail with `last_rowid` cursor is the only way to convert "DB changed somehow" into "this specific entity_id appeared". That logic is wrong to bake into Rulebook; it belongs in cortex-api where the dashboard contract lives.

Generalisation: cross-service observability should be implemented as a read-only adapter in the consumer, not as a publisher hook in the producer.
