## 1. Contract and canonical persistence

- [ ] 1.1 Confirm phase36_local-first-core is complete before implementation; define bounded audit records, exact scope, query/delivery IDs, memory revision/hash/source references, observed attribution, errors and latency.
- [ ] 1.2 Define evidence labels and proof criteria: prepared delivery, observed related action, explicitly evidenced use and unknown; temporal correlation never proves consumption.
- [ ] 1.3 Implement immutable canonical records, idempotent observation appends and rebuildable SQLite indexing using standalone locking/publication; test restart, concurrent writes and interrupted index updates.
- [ ] 1.4 Add reindex recovery and validation; test retained working index on invalid canonical input and preserved exact historical memory references.
- [ ] 1.5 Define explicit bounded retention/query controls with dry-run impact; test that no automatic operation deletes canonical audit or source data and no retention action cascades into memories/imports.

## 2. Capture and evidence linkage

- [ ] 2.1 Record actual standalone recall/hook preparation outcomes, bounded summaries, exact delivered revisions, measured latency and failures; never fabricate session/turn IDs or record credentials.
- [ ] 2.2 Link independently observed follow-up actions within exact scope, recording method, attribution and uncertainty; test cross-scope rejection, absent identifiers, retries and ambiguous temporal matches.
- [ ] 2.3 Implement evidence-class metrics with denominators and unknown/error counts; test that prepared receipts and adjacent actions cannot count as explicitly evidenced use.

## 3. Inspectability and acceptance

- [ ] 3.1 Expose bounded CLI and MCP audit inspection through the local engine; test lookup and metrics with HTTP server stopped and no external services.
- [ ] 3.2 Add embedded GUI audit inspection showing revisions, sources, actions, evidence labels, errors and latency; enforce scope and render imported text safely.
- [ ] 3.3 Validate a real local capture/recall and observed follow-up, then restart and reindex; report only evidence actually observed and verify original source hashes unchanged.

## 4. Mandatory tail

- [ ] 4.1 Update or create documentation covering the implementation: architecture, CLI/MCP/GUI, audit specification and CHANGELOG with evidence and retention limits.
- [ ] 4.2 Write tests covering the new behavior: persistence, reconstruction, scope, attribution, evidence classification, retention and standalone surfaces.
- [ ] 4.3 Run tests and confirm they pass, plus formatting, type-check, strict lint and required full suite; obtain independent review and record results before archival.
