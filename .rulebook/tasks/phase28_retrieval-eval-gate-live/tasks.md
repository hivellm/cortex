## 1. Prerequisite and reproducible corpus

- [ ] 1.1 Confirm phase36_local-first-core is complete and record its accepted build/contract; leave later items unchecked until this prerequisite is satisfied.
- [ ] 1.2 Define versioned public/local fixtures with stable identities, source hashes, provenance, relevance judgments and held-out queries covering recall, readable historical turns/tool events, supersession, empty results and repository collisions; document coverage/review criteria.
- [ ] 1.3 Build isolated homes through production import/reindex; test reproducibility, source preservation and equivalent identities/provenance after index rebuild.

## 2. Answer quality and isolation

- [ ] 2.1 Implement offline Recall@5/MRR@10 and per-query identity/provenance reports; test wrong records, duplicate hits, missing evidence and empty-result expectations.
- [ ] 2.2 Add adversarial scope checks for results, links, caches, statistics and ranking inputs, alternating repositories with matching names/text; require zero cross-scope disclosure.
- [ ] 2.3 Evaluate readable historical evidence and graph answers against expected nodes, typed edges and provenance; test supported paths/bounds and invalid links without inventing unimplemented tools.
- [ ] 2.4 Add offline Jev typed-response/quality fixtures, unknown/transient cases and authority checks; provide separately opt-in live evaluation of reviewed public payloads with bounded time/cost, model/schema/usage reporting and no secret output. Distinguish mocked scores from observed provider quality.

## 3. Latency and enforcement

- [ ] 3.1 Measure end-to-end CLI, direct MCP and supported hook latency with bounded samples/concurrency, separate cold/warm runs, p50/p95, failures and build/corpus/hardware/timing details; distinguish prepared delivery from model use.
- [ ] 3.2 Record reproducible baselines and explicitly justified standalone quality/latency floors before enforcement; test regression failures and preserve failed reports. Never silently lower floors or inherit legacy backend thresholds.
- [ ] 3.3 Wire deterministic explicit and push/PR gates without Docker, running HTTP service, live secrets or external/OS schedules; opt-in live Jev evaluation remains separate.
- [ ] 3.4 Run installed read-only smoke in an isolated fixture home: compare binary identity, advertised/implemented/documented tool sets and equivalent CLI/MCP identities/provenance/scope; check supported hook recall separately. Do not capture into the real store or classify.

## 4. Tail

- [ ] 4.1 Update or create documentation covering the implementation: evaluation docs, fixture provenance, baseline policy, measured limitations and CHANGELOG; retain historical failed-gate evidence and distinguish retrieval from observed agent use.
- [ ] 4.2 Write tests covering the new behavior: scoring, malformed fixtures, scope/cache contamination, wrong graph answers, provider failures, latency accounting and installed-surface drift.
- [ ] 4.3 Run tests and confirm they pass, plus type-check, formatting, lint and full required suite; obtain independent review, reconcile failures, then commit and archive only after acceptance passes.
