# TmlDocs — Open Questions and Gaps

## Scope Clarifications

### Q1: TML Compiler Source Code Documentation
**Status**: Unclear

**Question**: Should TmlDocs index the TML compiler's **implementation** docs (C++ source, AST design, codegen), or only the **user-facing** stdlib/language reference?

**Impact on Cortex**: 
- If implementation docs included: much larger surface for indexing
- If users-only: simpler, focused integration

**Recommendation**: Start users-only (stdlib + lang ref + tutorials); add implementation docs later if needed by HiveLLM infra team

---

### Q2: Scoped Packages Support
**Status**: Under-specified

**Question**: Design decision mentions scoped packages (`@user/package`) as "worth supporting from day one" but tml.toml spec doesn't define syntax. Should we support them at launch or in Phase 2?

**Impact**: 
- If launch: more design work (namespace schema, display name in registry)
- If Phase 2: faster initial launch

**Recommendation**: Defer to Phase 2. Start with flat namespace. Add scopes once publish volume justifies complexity.

---

### Q3: Advisory Database Source
**Status**: Unspecified

**Question**: Which upstream advisory feeds should registry pull from?
- NVD (National Vulnerability Database)
- GitHub Security Advisory Database
- RustSec database (closest model)
- Custom TML-specific advisories

**Impact**: 
- Scope of background audit worker
- Dependencies (API clients to NVD, GitHub)
- Data freshness SLA

**Recommendation**: Start with GitHub Security Advisory API (free, documented, good coverage for supply-chain attacks). Add NVD later if needed.

---

### Q4: Monorepo Metadata Completeness
**Status**: Partially specified

**Question**: When publishing from a monorepo (path_in_repo = "lib/postgresql"), should registry:
- Fetch parent tml.toml (workspace metadata)?
- Index all sibling packages?
- Track workspace relationships?

**Impact**: 
- Database schema (workspace_id, member relationships)
- Publish validation logic
- UI (show "part of X workspace" on package page)

**Recommendation**: Start simple: treat each package independently. Add workspace introspection in Phase 2 if ecosystem demands it.

---

### Q5: CLI Integration Scope
**Status**: Out-of-scope but uncertain

**Question**: Phase 7 (CLI integration) is in the TML compiler repo, not TmlDocs. Should TmlDocs define a **Registry API spec** first, then TML compiler implements against it?

**Impact**: 
- Chicken-and-egg: API design blocked on CLI requirements
- Stability: CLI team may discover missing endpoints mid-implementation

**Recommendation**: Parallel workstreams — TmlDocs ships API in Phase 2, TML compiler team implements CLI concurrently, feedback loop via integration tests.

---

### Q6: Search Ranking Algorithm
**Status**: Unspecified

**Question**: How should package search rank results?
- Relevance (BM25 / Postgres FTS)
- Popularity (install_count)
- Recency (updated_at)
- Maintainer reputation (GitHub stars of repo)
- Audit score (vulnerability count)

**Impact**: 
- User experience (discoverability of quality packages)
- Database queries (indexes on rank fields)

**Recommendation**: Start with BM25 (Postgres native) + install_count secondary. Tune ranking based on user feedback.

---

### Q7: Multi-Version Installation
**Status**: Unspecified

**Question**: If a developer's project depends on `postgresql@0.1.0` AND `postgresql@0.2.0` (via transitive deps), how does the resolver handle it?
- Force upgrade to 0.2.0?
- Compile both versions (like Rust)?
- Error out?

**Impact**: 
- Dependency resolution algorithm (semver matching)
- CLI logic
- Registry schema (no version conflict tracking yet)

**Recommendation**: Document resolver behavior in Phase 7. Suggest "use latest matching" for now; upgrade path TBD.

---

### Q8: Audit Result Retention
**Status**: Unspecified

**Question**: How long should we keep historical audit results?
- Forever (audit history of every version)
- 30 days (compliance audits)
- Only latest per version

**Impact**: 
- Database disk growth (audit_results table)
- Privacy (can deleted packages' audit data be recovered)
- Compliance requirements

**Recommendation**: Keep all (immutable audit history). Archive to separate DB/bucket if disk becomes concern.

---

### Q9: Rate Limiting Strategy
**Status**: Unspecified

**Question**: How should registry rate-limit Cortex, CLI, and web client?
- Per-IP
- Per-user (token-based)
- Per-endpoint
- Tiered (generous for CLI auth, strict for anonymous web)

**Impact**: 
- DDoS resistance
- Fair usage (prevent one bad actor blocking everyone)
- Cortex crawler design (batching, backoff logic)

**Recommendation**: Start simple: 100 req/sec per IP (web), unlimited per authenticated token (CLI/Cortex). Add sophisticated rate limiting if abuse detected.

---

### Q10: Dependency Graph Caching in Cortex
**Status**: Unspecified

**Question**: Should Cortex build and maintain a **persistent** dependency graph of TML packages, or just index snapshots at publish time?

**Impact**: 
- Cortex storage (graph DB vs. document store)
- Query capabilities (transitive dependency queries, etc.)
- Sync frequency (every publish vs. daily batch)

**Recommendation**: Start with snapshot indexing. Build dependency graph in Cortex as a follow-up if data science team needs it for ecosystem analysis.

---

## Known Gaps (Not Blockers)

1. **Monorepo workspace root detection**: tml.toml spec mentions workspace-root flag but version-publish flow doesn't fully specify workspace root discoverability.

2. **License compatibility checking**: Registry could warn users of incompatible license combinations (GPL + MIT → issues). Not yet designed.

3. **Performance optimization guide**: Docs mention benchmarks but no "writing performant TML" guide yet.

4. **Package deprecation workflow**: Design missing for graceful package retirement (DEPRECATED badge, forward-to replacement).

5. **Audit result remediation**: Audit finds vulnerability, but who fixes it? No process for "vulnerability resolved in 0.2.1" yet.
