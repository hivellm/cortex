# HiveHub.Cloud: Design Decisions & Rationale

**Status**: Key decisions finalized; documented as architectural constants.

---

## D1: Rust Backend (Not Node.js/Python)

**Decision**: Use Rust (Axum + Tokio) for the API backend.

**Why**:
- **Performance**: 10,000+ req/sec per instance vs 2,000–3,000 for Node.js
- **Memory Efficiency**: 50–70% less memory than NestJS equivalent
- **Type Safety**: Compile-time guarantees prevent runtime errors
- **Consistency**: Same language as Vectorizer, Nexus, Synap (reduces context switching)
- **Production-Ready**: Proven in high-throughput systems

**Trade-off**: Longer initial development; offset by reduced debugging time and type safety.

**Alternative Considered**: Python (Django/FastAPI). Rejected: Python unsuitable for 10k+ req/sec per instance without horizontal scaling.

---

## D2: Single Monorepo vs Microservices

**Decision**: Monorepo structure (apps/api + apps/dashboard); API is single service (not split into auth/projects/payments microservices).

**Why**:
- **Deployment Simplicity**: One Docker image, one database, one deployment unit
- **Data Consistency**: All state in one database (no distributed transaction complexity)
- **Development Speed**: Shared code, no API versioning friction between internal services
- **Cost**: No service mesh, container orchestration complexity, or cross-service communication overhead

**When to Reconsider**: If single-instance throughput exceeds 10,000 req/sec AND bottleneck is CPU/memory (not database), consider splitting into services.

**Current Status**: Single monolithic API; no plans to split.

---

## D3: PostgreSQL (Not MongoDB/DynamoDB)

**Decision**: PostgreSQL 16+ as primary database.

**Why**:
- **ACID Guarantees**: Critical for payment/subscription state
- **Relational Data**: Users → Subscriptions → Projects → Files is naturally relational
- **Query Power**: Complex queries for analytics/admin dashboard
- **Cost**: Open-source, proven, no vendor lock-in
- **Ecosystem**: Excellent Rust drivers (Diesel, SQLx), migration tools

**Trade-off**: Less flexible schema than NoSQL; requires planning schema changes in advance.

**Why Not**:
- **MongoDB**: Lack of ACID for financial transactions
- **DynamoDB**: Cost scaling (on-demand), lacks join capability for analytics
- **Elasticsearch**: Not a primary database (no transactions)

---

## D4: Diesel ORM (Not SQLx or raw SQL)

**Decision**: Diesel ORM for database abstraction.

**Why**:
- **Type Safety**: Compile-time query generation (catches SQL mistakes before runtime)
- **Migrations**: Diesel migrations are reversible and documented
- **Ecosystem**: Mature, widely used in Rust
- **Multi-Database**: Supports PostgreSQL, MySQL, SQLite (future flexibility)

**Alternative**: SQLx (also compile-time safe; more flexible SQL). Chosen Diesel for ergonomics.

**Current Status**: Transitioning from `config.toml` to environment variables for 12-factor compliance.

---

## D5: Local Filesystem for File Storage (Not S3 Immediately)

**Decision**: Start with `STORAGE_DIR` on local filesystem; S3 migration path planned.

**Why**:
- **Simplicity**: No cloud vendor dependencies during MVP phase
- **Cost**: Zero storage costs (just VM disk)
- **Speed**: Faster development iteration

**Migration Path**:
1. Implement S3 abstraction layer
2. Add pre-signed URLs for download
3. Migrate files to S3 with backward-compatibility redirect

**Timeline**: Post-MVP (not blocking v1.0 release).

---

## D6: JWT Tokens (Access + Refresh) Not Session Cookies

**Decision**: Stateless JWT tokens; no server-side sessions (except session invalidation metadata).

**Why**:
- **Scalability**: No session store shared across instances
- **API-Friendly**: Works with mobile clients, SPAs, MCP servers
- **Standard**: OAuth-compatible (Google, GitHub use JWT-like tokens)

**Tokens**:
- Access token: 1 hour (short-lived)
- Refresh token: 7 days (long-lived)
- Refresh token stored in DB for revocation/logout

**Trade-off**: Slight complexity in logout (must invalidate refresh token in DB).

---

## D7: Multi-Provider OAuth (Google + GitHub, Not Firebase Auth)

**Decision**: Native integration with Google OAuth and GitHub OAuth; Firebase for email/password only (future: direct Rust implementation).

**Why**:
- **User Preference**: Users expect familiar OAuth providers
- **Lower Friction**: Pre-existing Google/GitHub accounts
- **No Lock-in**: Could replace OAuth provider without app rewrite
- **Cost**: No Firebase licensing (eventual move to pure Rust)

**Future Plan**: Remove Firebase dependency; implement OAuth directly in Rust using standard OAuth 2.0 flow.

---

## D8: Stripe + PayPal (Not Single Payment Provider)

**Decision**: Support both Stripe and PayPal; user chooses at checkout.

**Why**:
- **User Choice**: Different users prefer different providers
- **Risk Distribution**: Not dependent on single payment processor downtime
- **Market Coverage**: PayPal popular with non-US users

**Implementation**: Separate webhook handlers; unified `payment_records` table; abstracted payment service layer.

---

## D9: Plan-Based Quotas (Not Usage-Based Pricing)

**Decision**: Fixed subscription tiers (Free/Pro/Enterprise) with hard limits; not pay-as-you-go.

**Why**:
- **Simplicity**: Predictable costs for users
- **Revenue Clarity**: Easy to forecast MRR
- **Implementation**: Simpler enforcement logic (hard limits vs metered usage)

**Future Flexibility**: Could add overage charges or usage-based upsell (e.g., "Pro + 10k extra vectors = $5/mo").

**Limits Are Shared Across Projects**: User pays once, can allocate across projects freely.

---

## D10: Soft Deletes for User/Project/File Records

**Decision**: Use `deleted_at` timestamp (soft delete) instead of hard delete.

**Why**:
- **Audit Trail**: Can recover deleted data if user requests
- **Referential Integrity**: Easier to maintain FK relationships
- **Analytics**: Historical data remains for reporting

**Exceptions**:
- Payment records: never deleted (legal requirement)
- Sessions: hard delete on logout/expiration (privacy)

**Compliance**: Hard delete on GDPR "right to be forgotten" request (future feature).

---

## D11: Service API Keys (Not OAuth) for Service-to-Service Auth

**Decision**: Services (Vectorizer, Nexus, Synap) authenticate to HiveHub.Cloud using static API keys in Bearer tokens; not OAuth.

**Why**:
- **Simplicity**: Static keys stored in env vars, no token refresh complexity
- **Service-to-Service**: OAuth designed for user-facing flows; overkill for internal APIs
- **Rate Limiting**: Keys can be scoped per-service and rate-limited independently

**Security**:
- Keys rotated quarterly
- Keys logged/audited for access
- Keys scoped to specific service (X-Service-Name header validation)

**Alternative Considered**: mTLS (mutual TLS). Deferred: adds operational complexity.

---

## D12: File Upload Pipeline (Transmutation → Vectorizer → Nexus)

**Decision**: Three-stage async pipeline for file processing.

**Why**:
- **Decoupling**: Each stage independent; can scale independently
- **Failure Isolation**: Vectorizer failure doesn't block Transmutation
- **Extensibility**: Easy to add new stages (e.g., graph extraction, summarization)

**Queue**: Synap (HiveHub.Cloud pushes jobs to user's queue; services pull)

**Status Tracking**: File.status field (pending → processing → indexed → failed)

---

## D13: Shared Quotas Across Projects

**Decision**: Storage and document limits apply **across all user projects**, not per-project.

**Why**:
- **User Flexibility**: Can organize projects without worrying about per-project limits
- **Simplicity**: Single quota check; no project-level accounting
- **Revenue Model**: Charge per user, not per project

**Example**: Free user with 50MB limit can split across 5 projects (10MB each) or use all 50MB in one project.

---

## D14: MCP Server Generation (Planned)

**Decision**: Users can generate authenticated MCP server endpoints for each Hive service.

**Why**:
- **Integration**: Claude, Cursor, and other MCP-capable tools can access Hive services
- **Access Control**: Per-project, per-service API keys (future: per-operation scoping)
- **One-Click Setup**: No manual credential management

**Implementation**: POST `/api/mcp-servers` returns endpoint URL + API key.

**Status**: Specification complete; implementation planned post-core-features.

---

## D15: Dashboard Frontend Not in MVP

**Decision**: v1.0 focuses on API; dashboard frontend (React) deferred.

**Why**:
- **MVP Scope**: API is sufficient for MCP server usage; CLI tools; scripts
- **Time to Market**: Skip UI development, validate API first
- **Flexibility**: Can build dashboard in React, CLI tools, or both

**Future Plan**: Dashboard v1 post-core API stabilization.

---

## D16: Diesel Migrations (Not Flyway/Alembic)

**Decision**: Use Diesel's native migration system.

**Why**:
- **Rust Native**: Integrated with Cargo/codebase
- **Type Safety**: Migrations are Rust, subject to same compile-time checks
- **Ecosystem**: Works with SQLx and other Rust tools

**Alternative**: Sqlx migrations (also viable). Chose Diesel for consistency with ORM.

---

## D17: No GraphQL (Yet)

**Decision**: REST API only for v1.0; GraphQL deferred.

**Why**:
- **Simplicity**: REST is standard, well-known
- **Caching**: HTTP caching works naturally with REST
- **Tooling**: OpenAPI/Swagger mature ecosystem
- **Learning Curve**: Reduces complexity for first API iteration

**GraphQL Path**: If demand exists, can add `/api/graphql` endpoint post-v1.

---

## D18: Centralized Error Handling (Not Per-Route)

**Decision**: Unified error response format + middleware error handler.

**Why**:
- **Consistency**: All errors formatted the same way
- **Observability**: Errors logged centrally with trace IDs
- **Client Experience**: Predictable error messages

**Error Response Format**:
```json
{
  "error": {
    "code": "QUOTA_EXCEEDED",
    "message": "Storage limit reached",
    "status": 429,
    "trace_id": "abc123..."
  }
}
```

---

## D19: Async/Await for All I/O (Not Blocking Threads)

**Decision**: All database, HTTP, and file I/O is async (Tokio runtime).

**Why**:
- **Concurrency**: Handle thousands of concurrent requests with few threads
- **Resource Efficiency**: Memory-efficient vs thread-per-request model

**Implementation**: `#[tokio::main]` runtime; all services are `async fn`.

---

## D20: 95%+ Code Coverage Requirement

**Decision**: All code must have ≥95% test coverage.

**Why**:
- **Reliability**: Most bugs caught before production
- **Refactoring Safety**: High confidence when changing code
- **Financial**: Payment/subscription code critical; must be bulletproof

**Enforcement**: CI pipeline blocks merge if coverage drops.

---

## Summary: Architectural Invariants

| Invariant | Value | Rationale |
|-----------|-------|-----------|
| **Language** | Rust | Performance + type safety |
| **Framework** | Axum + Tokio | High concurrency, low latency |
| **Database** | PostgreSQL | ACID + relational + cost |
| **Auth** | JWT + OAuth | Stateless + standard |
| **Storage** | Local FS → S3 | Simplicity then scalability |
| **Quotas** | Shared across projects | Flexibility + simplicity |
| **Coverage** | 95%+ minimum | Reliability |
| **Deployment** | Docker + 12-factor | Containerized + portability |
