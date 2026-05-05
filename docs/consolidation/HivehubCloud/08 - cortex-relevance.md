# HiveHub.Cloud: Cortex Ingestion Priorities

**Purpose**: Guide Cortex knowledge consolidation; prioritize what to index, analyze, and track.

---

## High Priority (Ingest First)

### 1. User/Subscription Context

**Why**: Cortex operates in multi-tenant environment; must understand user quotas and plan limits.

**What to Index**:
- User ID → Subscription plan mapping
- Plan quotas (storage, documents, collections)
- Current usage per user
- Active/inactive subscription status

**Source**: `subscriptions`, `usage_metrics` tables

**Update Frequency**: Real-time (on user activity)

**Use Case**: When Cortex processes data for a user, query HiveHub.Cloud to enforce quotas before indexing.

### 2. File Upload Pipeline State

**Why**: Cortex ingests data *from* file upload pipeline; must track processing status and errors.

**What to Index**:
- File ID → processing status (pending, processing, indexed, failed)
- File chunks and chunk count
- Vector count and storage size
- Error messages (if failed)

**Source**: `files`, `file_chunks` tables

**Update Frequency**: Real-time (on pipeline stage completion)

**Use Case**: Cortex can retry failed file processing; optimize chunking strategy based on success/failure rates.

### 3. Project Organization

**Why**: Users organize data into projects; Cortex should track project-to-collection mappings.

**What to Index**:
- Project ID → user ID
- Project metadata (name, creation date)
- Collections created per project (via Service API)
- File count per project

**Source**: `projects` table + Service API calls

**Update Frequency**: On project creation/deletion

**Use Case**: Cortex can answer "all files for project X" queries; provide project-scoped analytics.

---

## Medium Priority (Ingest Second)

### 4. Payment & Subscription Events

**Why**: Plan changes affect available quotas; Cortex should track events for audit/analytics.

**What to Index**:
- Payment success/failure events
- Plan upgrades/downgrades
- Quota changes (e.g., Free 50MB → Pro 500MB)
- Refunds or cancellations

**Source**: `payment_records`, `subscriptions` (history)

**Update Frequency**: On payment webhook received

**Use Case**: Analytics dashboard; identify churn patterns; reconcile usage with plan changes.

### 5. Service API Call Patterns

**Why**: Track which services (Vectorizer, Nexus, Synap) are called most; identify performance issues.

**What to Index**:
- Service endpoint call count per user
- Response times per service
- Error rates per service
- API key usage (which service doing what)

**Source**: Application logs (structured JSON)

**Update Frequency**: Log aggregation (hourly/daily rollup)

**Use Case**: Performance optimization; identify slow services; quota enforcement tuning.

### 6. Authentication & Session Events

**Why**: Security audit; track login patterns, OAuth provider usage.

**What to Index**:
- Login events (email/password vs Google vs GitHub)
- Failed authentication attempts
- Session creation/invalidation
- Password changes

**Source**: Application logs + `sessions` table

**Update Frequency**: Real-time (on auth event)

**Use Case**: Security incident investigation; user behavior analytics.

---

## Low Priority (Ingest Third, Optional)

### 7. Admin Metrics & System Health

**Why**: Operational insights; identify bottlenecks.

**What to Index**:
- Database connection pool usage
- API response time distribution
- Error rate by endpoint
- Storage utilization trends

**Source**: Application logs + Prometheus/Datadog metrics

**Update Frequency**: Periodic (hourly/daily)

**Use Case**: Capacity planning; SLA monitoring; cost optimization.

### 8. Webhook Events

**Why**: Track external integrations (Stripe, PayPal, OAuth providers).

**What to Index**:
- Webhook received/processed events
- Webhook failure/retry events
- Provider-specific event types (payment_intent.succeeded, etc.)

**Source**: Application logs

**Update Frequency**: Real-time

**Use Case**: Integration health monitoring; audit trail for payments.

---

## Not Relevant to Cortex (Skip)

- **Internal API Responses**: Do not need to index every internal API call response (too noisy)
- **JWT Token Details**: Do not index token content (privacy); only login events
- **Environment Variables**: Do not index secrets (security)
- **Source Code**: Do not index code (out of scope for Cortex)

---

## Recommended Cortex Ingestion Flow

```
1. Initialize:
   └─ Load User/Subscription mappings
      └─ Load Project IDs per user
      └─ Load current usage_metrics

2. On File Upload:
   └─ Create file record
   └─ Link to project
   └─ Track progress through pipeline

3. On Pipeline Stage:
   ├─ Transmutation complete → log chunk count
   ├─ Vectorization complete → log vector count + storage size
   └─ Graph extraction (if enabled) → log node/relationship count

4. On Quota Trigger:
   └─ Check usage_metrics vs plan limits
   └─ Block operation if quota exceeded
   └─ Log quota enforcement event

5. Analytics (periodic):
   └─ Aggregate file processing times
   └─ Summarize per-user quota usage
   └─ Calculate cost/plan ratio (cost vs service usage)
```

---

## Data Schema for Cortex Integration

### Recommended Cortex Collections

#### `hivehub_users`

```json
{
  "id": "uuid",
  "email": "user@example.com",
  "subscription_plan": "free|pro|enterprise",
  "subscription_status": "active|pending|cancelled",
  "created_at": "2025-12-04T...",
  "updated_at": "2025-12-04T...",
  "quota": {
    "storage_bytes_limit": 52428800,
    "storage_bytes_used": 10485760,
    "document_count_limit": 1000,
    "document_count": 150,
    "api_requests_limit": 200,
    "api_requests_used": 45
  }
}
```

#### `hivehub_projects`

```json
{
  "id": "uuid",
  "user_id": "uuid",
  "name": "My Project",
  "created_at": "2025-12-04T...",
  "file_count": 42,
  "collection_id": "vectorizer-collection-uuid",
  "graph_id": "nexus-graph-uuid"
}
```

#### `hivehub_files`

```json
{
  "id": "uuid",
  "user_id": "uuid",
  "project_id": "uuid",
  "name": "document.pdf",
  "status": "indexed",
  "chunk_count": 42,
  "vector_count": 42,
  "storage_bytes": 1048576,
  "uploaded_at": "2025-12-04T...",
  "processed_at": "2025-12-04T...",
  "error": null
}
```

#### `hivehub_events`

```json
{
  "id": "uuid",
  "type": "file_uploaded|quota_enforced|plan_upgraded|payment_received",
  "user_id": "uuid",
  "data": {
    "file_id": "uuid",
    "plan": "pro",
    "amount_cents": 2000,
    "error": "quota_exceeded"
  },
  "timestamp": "2025-12-04T..."
}
```

---

## Integration Points

### HiveHub.Cloud → Cortex (Push)

1. **On File Upload**:
   ```
   POST /cortex/api/files
   {
     "id": "...",
     "user_id": "...",
     "project_id": "...",
     "status": "processing"
   }
   ```

2. **On Pipeline Stage Complete**:
   ```
   PATCH /cortex/api/files/{id}
   {
     "status": "indexed",
     "vector_count": 42,
     "storage_bytes": 1048576
   }
   ```

3. **On Subscription Change**:
   ```
   PATCH /cortex/api/users/{id}/subscription
   {
     "plan": "pro",
     "quota": { ... }
   }
   ```

### Cortex → HiveHub.Cloud (Pull)

1. **Query User Quota**:
   ```
   GET /api/subscriptions/current
   Authorization: Bearer <user-token>
   ```

2. **Query File Status**:
   ```
   GET /api/files/{fileId}
   Authorization: Bearer <user-token>
   ```

3. **Query Project Info**:
   ```
   GET /api/projects/{projectId}
   Authorization: Bearer <user-token>
   ```

---

## Analytics Queries for Cortex

### 1. User Cohort Analysis

```
Find all users with plan='free' AND storage_used > 40MB
→ Identify upsell candidates
```

### 2. Pipeline Performance

```
Average time from file upload to indexed across all users
→ Identify processing bottlenecks
```

### 3. Quota Enforcement

```
Count enforcement events by plan
→ Verify quota limits are appropriate
```

### 4. Service Integration Health

```
Error rate by Hive service (Vectorizer, Nexus, Synap, LessTokens)
→ Identify integration issues
```

### 5. Revenue Correlation

```
Correlate plan upgrades with file processing volume
→ Justify pricing; identify features driving upgrades
```

---

## Cortex Analyzer Focus Areas

### For HiveHub.Cloud Product

1. **Quota Efficiency**: Are quotas aligned with real user needs?
2. **Feature Adoption**: Which Hive services (Vectorizer, Nexus, Synap) do users consume most?
3. **Processing Performance**: Median/p95/p99 file processing times
4. **Churn Indicators**: What predicts plan cancellation?

### For Service Integration Quality

1. **Vectorizer**: Vector quality, chunking strategy effectiveness
2. **Nexus**: Node/relationship creation patterns, classification accuracy
3. **Synap**: Queue job completion rates, latency
4. **LessTokens**: Token budget consumption, cost per operation

### For Cortex Itself

1. **Indexing Completeness**: What % of HiveHub.Cloud data is indexed in Cortex?
2. **Query Coverage**: Can Cortex answer key questions about HiveHub.Cloud?
3. **Data Freshness**: How recent is Cortex data vs live HiveHub.Cloud state?
4. **Cross-Project Insights**: Can Cortex correlate patterns across all Hive projects?

---

## Timeline

**Phase 1 (MVP)**: User + Subscription + Project collections only.

**Phase 2**: Add File + Event collections; implement push integration.

**Phase 3**: Add Service API call pattern tracking; analytics dashboards.

**Phase 4**: Add payment event tracking; revenue correlation analysis.

---

## Success Metrics

1. **Ingestion Completeness**: 100% of active users indexed
2. **Data Freshness**: <5 minute delay from event to index
3. **Query Latency**: <500ms for typical quota/file queries
4. **Analyst Productivity**: Can answer "X" question in <1 minute without code changes
