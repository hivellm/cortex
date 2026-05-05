# HiveHub.Cloud: Open Questions & Gaps

**Status**: Unresolved items blocking full clarity on design/implementation.

---

## Unresolved Design Decisions

### Q1: Dashboard Technology & Timeline

**Question**: Should dashboard be React + TypeScript + Vite (planned) or something else? When should it ship?

**Current Status**:
- CLAUDE.md specifies React + TypeScript + Vite
- No implementation started
- Not blocking API v1.0 release

**Impact**: Affects hiring (need React dev), feature prioritization, GTM timeline.

**Next Step**: Product decision required; API can function without dashboard.

---

### Q2: GraphQL API

**Question**: Should HiveHub.Cloud expose a GraphQL endpoint in addition to REST?

**Current Status**:
- REST API fully specified and implemented
- No GraphQL mentioned in specifications
- Some admin dashboard features might benefit from GraphQL (complex nested queries)

**Arguments For**:
- Better for frontend pagination/nested data
- IDE integration tools (Relay, Apollo) reduce boilerplate

**Arguments Against**:
- REST is sufficient for current API surface
- Adds implementation complexity
- N+1 query potential if not careful

**Next Step**: Defer to post-v1.0; gather user feedback first.

---

### Q3: Rate Limiting Implementation

**Question**: Should API have per-user rate limits? If so, what are the thresholds?

**Current Status**:
- Architecture mentions rate limiting (planned)
- No specification exists
- No code implemented

**Proposal**:
```
Free plan: 100 req/min
Pro plan: 1000 req/min
Enterprise: 10,000 req/min
```

**Next Step**: Implement rate limiting middleware post-core-features.

---

### Q4: Recurring Billing Logic

**Question**: How should recurring billing work? When is next invoice sent? How are downgrades handled mid-cycle?

**Current Status**:
- Stripe/PayPal integration exists
- Payment webhooks implemented
- Recurring billing specification mentioned but not detailed

**Scenarios**:
1. **Free → Pro upgrade**: Start billing immediately? Prorate?
2. **Pro → Free downgrade**: Refund unused days?
3. **Subscription renewal**: Auto-charge on `renews_at` date?
4. **Successful charge failure**: How many retries? When to downgrade plan?

**Next Step**: Product specification required; post-MVP.

---

### Q5: File Retention & Auto-Deletion

**Question**: Should files be auto-deleted after a certain period? Who pays for storage of abandoned files?

**Current Status**:
- File deletion exists (manual via DELETE endpoint)
- No auto-deletion policy specified
- No storage cleanup scheduled

**Options**:
1. **Permanent retention**: User pays indefinitely (current behavior)
2. **Grace period**: Delete after 90 days of no activity
3. **Archive tier**: Move old files to cheaper S3 storage

**Next Step**: Product decision; may affect compliance (GDPR data minimization).

---

### Q6: Multi-Region Deployment

**Question**: Should HiveHub.Cloud support multi-region deployment? Data residency requirements?

**Current Status**:
- Single-region PostgreSQL assumed
- No multi-region specification
- Some enterprise customers may require EU data residency

**Challenges**:
- Database replication complexity
- Webhook handling across regions
- Payment processor integrations per region

**Next Step**: Defer to Enterprise feature; gather requirements from customers.

---

## Implementation Gaps

### G1: Dashboard Frontend

**Status**: Not started.

**Required**:
1. React component library selection (Material-UI? Ant Design? Custom?)
2. State management (Context API? Zustand?)
3. Authentication middleware (JWT token refresh)
4. Page structure (login, projects, subscriptions, settings, admin)
5. File upload UI
6. Subscription upgrade flow

**Estimated Effort**: 4–6 weeks

**Blocking**: No; API functional without dashboard.

---

### G2: Admin Dashboard Analytics

**Status**: Endpoints specified in routes/admin, logic not implemented.

**Required**:
1. User list + subscription status
2. Revenue reports (MRR, ARR, churn)
3. System health (service endpoints, database performance)
4. Abuse detection (quota violations, suspicious activity)

**Estimated Effort**: 2–3 weeks

**Blocking**: No; can manually query database in interim.

---

### G3: Entity Extraction & Knowledge Graph Building

**Status**: File → Vectorizer integration done; File → Nexus integration sketched but not implemented.

**Required**:
1. After vectors indexed, async job to extract entities
2. Call LessTokens API for entity recognition
3. Create Nexus nodes for entities
4. Create relationships between entities

**Estimated Effort**: 3–4 weeks

**Blocking**: No; users can manually create graphs.

---

### G4: MCP Server Endpoint Implementation

**Status**: Specification complete; code not written.

**Required**:
1. `POST /api/mcp-servers` endpoint
2. Generate per-project MCP API key
3. `GET /mcp/servers/{id}` proxy endpoint
4. Forward requests to underlying Hive service
5. Apply user quota limits to MCP requests

**Estimated Effort**: 1–2 weeks

**Blocking**: No; users can use dashboard/API directly.

---

### G5: Audit Logging

**Status**: Database table schema sketched; no implementation.

**Required**:
1. Log all user actions (login, file upload, subscription change, etc.)
2. Log admin actions (user deletion, quota reset, etc.)
3. Audit log query endpoint (`GET /api/audit-logs`)
4. Data retention policy (how long to keep logs?)

**Estimated Effort**: 1–2 weeks

**Blocking**: No; can add later.

---

### G6: GDPR Data Subject Access Request (DSAR)

**Status**: Not implemented.

**Required**:
1. `POST /api/users/dsar` endpoint to initiate data export
2. Collect all user data (profile, projects, files, payment history)
3. Generate ZIP file with user data
4. Email to user

**Estimated Effort**: 1 week

**Blocking**: No; handle manually for early customers.

---

## Technical Debt & Known Issues

### Issue 1: Firebase Auth Dependency

**Status**: Current code uses Firebase for OAuth; should be replaced with native Rust.

**Reason**: Firebase adds vendor lock-in; native OAuth simpler.

**Effort**: 1–2 weeks

**Priority**: Medium (post-v1.0)

---

### Issue 2: Migration from Config File to Environment Variables

**Status**: Code supports both `config.toml` and env vars; should remove toml support.

**Reason**: 12-factor app best practice; env vars simpler in Docker.

**Effort**: 1 day

**Priority**: Low (post-v1.0)

---

### Issue 3: S3 Migration Path

**Status**: Local filesystem storage assumed; S3 migration deferred.

**Reason**: Simplicity during MVP; S3 required for production multi-region.

**Effort**: 1–2 weeks

**Priority**: Medium (pre-production)

---

### Issue 4: Error Message Localization

**Status**: All error messages in English; no i18n support.

**Reason**: Early stage; not enough users to justify complexity.

**Effort**: 1 week per language

**Priority**: Low (post-v1.0)

---

## Specification Gaps

### S1: Exact Quota Limits for Pro/Enterprise Plans

**Current Status**:
```
Free: 1,000 docs, 50MB storage, 200 API calls/mo, 1 collection
Pro: 10,000 docs, 500MB storage, 2,000 API calls/mo, 5 collections
Enterprise: 100,000 docs, 5GB storage, 20,000 API calls/mo, unlimited
```

**Questions**:
- Are these numbers based on customer feedback or guesses?
- Should they be adjustable per-customer (Enterprise)?
- Should usage-based overages be supported?

**Next Step**: Validate with early customers.

---

### S2: Collection Naming & Metadata

**Current Status**:
- Collections created with auto-generated names (e.g., `user-{userId}-project-{projectId}-collection`)
- No metadata (description, tags, public/private flag)

**Questions**:
- Should collections be user-customizable?
- Should collections be shareable across projects?
- Should multiple users share a collection?

**Next Step**: Product requirements gathering.

---

### S3: File Chunk Size & Overlap

**Current Status**:
- Transmutation service chunks files into Markdown
- HiveHub.Cloud doesn't control chunk size or overlap strategy

**Questions**:
- What is optimal chunk size for semantic search? (1024 tokens? 2048?)
- Should chunks overlap? (sliding window strategy?)
- Should chunk strategy be tunable per-plan?

**Next Step**: Experiment with Transmutation; measure search quality vs latency.

---

### S4: Search Quality & Ranking

**Current Status**:
- Vectorizer performs semantic search (cosine similarity)
- No ranking customization

**Questions**:
- Should search results be re-ranked by relevance score?
- Should filtering by metadata be supported?
- Should full-text search be combined with semantic search?

**Next Step**: User feedback required.

---

### S5: Data Export & Portability

**Current Status**:
- No bulk export endpoint

**Questions**:
- Should users be able to export all vectors as JSON?
- Should they be able to export Nexus graphs as JSON/GraphML?
- Should exported data be compatible with other vector DBs?

**Next Step**: Plan post-v1.0; may be strategic advantage (reduce lock-in perception).

---

## Infrastructure & Operations

### O1: Disaster Recovery Plan

**Status**: Not documented.

**Questions**:
- What is RTO (Recovery Time Objective)? (1 hour? 24 hours?)
- What is RPO (Recovery Point Objective)? (1 hour? 1 day?)
- How often are backups tested?
- What is the runbook for database corruption?

**Next Step**: Write formal DR plan pre-production.

---

### O2: Performance Benchmarks

**Status**: Expected performance documented; no actual benchmarks.

**Questions**:
- What is p95 latency for typical operations?
- What is throughput per instance under load?
- What is database connection pool exhaustion point?

**Next Step**: Run load test (k6, Apache JMeter) post-MVP.

---

### O3: Cost Estimation

**Status**: No cost model published.

**Questions**:
- What is cost per user (storage, compute, Hive service APIs)?
- What is margin target per plan?
- Are there loss-leader plans (Free)?

**Next Step**: Financial model required; informs pricing.

---

## Security & Compliance

### C1: OAuth Token Security

**Status**: Google/GitHub OAuth tokens stored in database?

**Questions**:
- Are tokens encrypted at rest?
- What is token refresh strategy?
- Can tokens be revoked if user account hacked?

**Next Step**: Security audit required.

---

### C2: Payment Security (PCI-DSS)

**Status**: Stripe handles payment processing; HiveHub.Cloud doesn't store cards.

**Questions**:
- Is Stripe 3DS (3D Secure) required for compliance?
- Are payment webhooks properly validated?
- Are payment records encrypted?

**Next Step**: Security audit + PCI compliance review.

---

### C3: Data Privacy (GDPR/CCPA)

**Status**: Not fully compliant.

**Required**:
- GDPR: Data subject access requests, right to be forgotten
- CCPA: California privacy rights

**Next Step**: Legal review required.

---

## Suggested Next Steps (Priority Order)

1. **Validate quota limits** with early customers; adjust if needed.
2. **Implement rate limiting** (protect against abuse).
3. **Write disaster recovery plan** (pre-production requirement).
4. **Add audit logging** (compliance + debugging).
5. **Build admin dashboard** (operational visibility).
6. **Implement GDPR DSAR** (legal requirement for EU users).
7. **Optimize database queries** (performance tuning based on load tests).
8. **Start dashboard frontend** (if go-to-market requires UI).
9. **Implement recurring billing** (if subscriptions will auto-renew).
10. **Migrate to S3** (if multi-region/scale required).
