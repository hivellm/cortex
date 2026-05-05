# HiveHub.Cloud: Service Integrations

**Role**: HiveHub.Cloud is the control plane; Vectorizer, Nexus, Synap are backend services orchestrated via REST API + internal Rust SDK.

---

## Overview Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                    HiveHub.Cloud                             │
│  (Users, Subscriptions, Projects, Quotas, File Upload)      │
└────────────────────┬────────────────────────────────────────┘
                     │
     ┌───────────────┼───────────────┬──────────────┐
     │               │               │              │
     ▼               ▼               ▼              ▼
┌─────────┐    ┌─────────┐    ┌────────┐    ┌──────────────┐
│Vectorizer│   │  Nexus  │    │ Synap  │    │ LessTokens   │
│ (Search) │   │ (Graph) │    │(KV/Q)  │    │  (Token Mgmt)│
└─────────┘    └─────────┘    └────────┘    └──────────────┘
```

---

## Vectorizer Integration

**Purpose**: Semantic search and vector storage.

**Capabilities**:
- Collection management (create, list, delete)
- Vector insertion and search
- Text embedding (via chunking)
- Semantic similarity queries

### REST API

**Endpoints** (service-specific config):
```
POST /collections                      Create collection
GET /collections/{collectionId}        Get collection info
DELETE /collections/{collectionId}     Delete collection
POST /collections/{collectionId}/vectors  Insert vectors
POST /collections/{collectionId}/search   Search by embedding
```

### Integration Flow: File Upload → Vectorizer

1. **User uploads file** via `/api/files/upload`
2. **HiveHub.Cloud checks quota** (storage + document limit)
3. **File stored locally** in `STORAGE_DIR`
4. **Async job launched**: send to Transmutation service
5. **Transmutation returns Markdown** (chunked by service)
6. **Markdown sent to Vectorizer**:
   - Each chunk embedded (text → embedding vector)
   - Vectors inserted into user's collection
   - Vector count tracked in `files.vectors_stored`
7. **HiveHub.Cloud records usage**: increment `usage_metrics.document_count`, `storage_bytes`

### Quota Enforcement

Before Vectorizer indexes vectors, it calls HiveHub.Cloud Service API:

```
GET /api/internal/vectorizer/collection/{collectionId}/validate
  ?userId={userId}
Authorization: Bearer <vectorizer_service_key>
```

**Response**:
```json
{
  "valid": true,
  "quota": {
    "max_vectors": 10000,
    "max_collections": 1,
    "vectors_used": 5000,
    "collections_used": 1
  }
}
```

**Rules**:
- Free plan: 1 collection, 10,000 vectors
- Pro plan: 5 collections, 100,000 vectors per collection
- Enterprise: unlimited

If quota exceeded → Vectorizer returns 429 Quota Exceeded

### Internal Rust SDK

Services use the Rust SDK to abstract API calls:

```rust
use hivehub_cloud_internal_sdk::HiveHubCloudClient;

let client = HiveHubCloudClient::new(
  "svc_vectorizer_abc123...",
  "https://api.hivehub.cloud"
)?;

// Validate collection before insert
let validation = client.vectorizer()
  .validate_collection(&collection_id, &user_id)
  .await?;

if validation.valid {
  // Insert vectors
}

// Update usage after insert
client.vectorizer()
  .update_usage(&user_id, &UsageUpdate {
    vectors_added: 42,
    ...
  })
  .await?;
```

---

## Nexus Integration

**Purpose**: Knowledge graphs, node relationships, LLM-based classification.

**Capabilities**:
- Node creation and management
- Relationship creation
- Cypher query execution
- Classification (LLM-based, consumes credits)

### REST API

**Endpoints**:
```
POST /graphs/{graphId}/nodes          Create node
GET /graphs/{graphId}/nodes/{nodeId}  Get node
POST /graphs/{graphId}/relationships  Create relationship
POST /graphs/{graphId}/query          Execute Cypher query
POST /graphs/{graphId}/classify       Classify node (LLM)
```

### Integration Flow: File → Knowledge Graph

1. **File indexed in Vectorizer** (chunks → vectors)
2. **Async job**: Extract entities from vectors
   - Use LessTokens for entity extraction
   - Create nodes in Nexus (one per entity)
   - Create relationships between entities
3. **HiveHub.Cloud tracks usage**:
   - Nodes created: increment `usage_metrics`
   - API calls: increment LessTokens counter

### Quota Enforcement

Before Nexus creates nodes/relationships, it validates:

```
GET /api/internal/nexus/user/{userId}/databases
Authorization: Bearer <nexus_service_key>
```

**Response**:
```json
{
  "user_id": "uuid",
  "quota": {
    "max_nodes": 100000,
    "max_relationships": 500000,
    "max_graph_storage_mb": 5000,
    "nodes_used": 5000,
    "relationships_used": 10000,
    "storage_used_mb": 50
  }
}
```

**Rules**:
- Free plan: 10,000 nodes, 50,000 relationships
- Pro plan: 100,000 nodes, 500,000 relationships
- Enterprise: unlimited

---

## Synap Integration

**Purpose**: Key-value storage, queues, pub/sub messaging.

**Capabilities**:
- Set/get key-value pairs
- Queue operations (push, pop)
- Pub/sub channels

### REST API

**Endpoints**:
```
POST /kv/{namespace}/keys/{key}    Set key-value
GET /kv/{namespace}/keys/{key}     Get key-value
DELETE /kv/{namespace}/keys/{key}  Delete key
POST /queues/{queueId}/items       Push to queue
GET /queues/{queueId}/items        Pop from queue
```

### Integration Flow: Async Job Queues

1. **File upload initiated** → HiveHub.Cloud queues job in Synap
2. **Transmutation worker** pulls job from queue
3. **After Transmutation** → queue next job (vectorization)
4. **After Vectorization** → queue next job (graph extraction, optional)

**Namespace per User**:
- KV namespace: `user-{userId}-kv`
- Queue namespace: `user-{userId}-jobs`

### Quota Enforcement

```
GET /api/internal/synap/user/{userId}/keys
Authorization: Bearer <synap_service_key>
```

**Response**:
```json
{
  "user_id": "uuid",
  "quota": {
    "max_keys": 10000,
    "max_queue_items": 100000,
    "keys_used": 500,
    "queue_items_used": 10
  }
}
```

---

## LessTokens Integration

**Purpose**: Token-optimized LLM requests (entity extraction, summarization, etc.).

**Capabilities**:
- Prompt compression (reduce token usage)
- Model requests with automatic fallback
- Token counting and budgeting

### REST API

**Endpoints**:
```
POST /requests              Send compressed request
GET /usage                  Get monthly usage + remaining budget
POST /batch                 Batch multiple requests
```

### Integration Flow: Entity Extraction

1. **File chunks indexed** in Vectorizer
2. **Entity extraction job** launched:
   - Call LessTokens with chunk → entities JSON
   - LessTokens compresses prompt, calls LLM
   - Returns extracted entities
3. **Entities ingested** into Nexus as nodes
4. **HiveHub.Cloud tracks usage**: increment monthly API request count

### Quota & Credits

**Plan-based budgets**:
- Free: 200 requests/month
- Pro: 2,000 requests/month
- Enterprise: 20,000 requests/month

**Enforcement**: When user exhausts monthly budget, LessTokens returns 429 Quota Exceeded

---

## Payment Integrations

### Stripe

**Integration Points**:
- Plan upgrade/downgrade → create Stripe checkout session
- Webhook `payment_intent.succeeded` → update subscription status
- Webhook `invoice.payment_failed` → notify user, downgrade plan

**Configuration**:
```env
STRIPE_API_KEY=sk_live_...
STRIPE_WEBHOOK_SECRET=whsec_...
```

### PayPal

**Integration Points**:
- Plan upgrade → create PayPal checkout
- Webhook payment notification → update subscription
- Webhook payment denied → mark invoice as failed

**Configuration**:
```env
PAYPAL_CLIENT_ID=...
PAYPAL_SECRET=...
PAYPAL_MODE=live
```

---

## Service API (Internal)

**Endpoints for Services Only** (not user-facing).

### Authentication

Service-specific API key in Bearer token + `X-Service-Name` header:

```
Authorization: Bearer svc_vectorizer_abc123...
X-Service-Name: vectorizer
```

### Vectorizer Service API

```
GET /api/internal/vectorizer/user/{userId}/collections
  → List user's collections for data segregation

GET /api/internal/vectorizer/collection/{collectionId}/validate
  → Verify collection ownership + check quota

POST /api/internal/vectorizer/usage/update
  → Record vectors indexed, storage used

GET /api/internal/vectorizer/user/{userId}/quota
  → Get user's Vectorizer-specific quota
```

### Nexus Service API

```
GET /api/internal/nexus/user/{userId}/databases
  → List user's graphs

POST /api/internal/nexus/usage/update
  → Record nodes/relationships created

GET /api/internal/nexus/user/{userId}/quota
  → Get Nexus-specific quota
```

### Synap Service API

```
GET /api/internal/synap/user/{userId}/keys
  → List KV namespace info

POST /api/internal/synap/usage/update
  → Record key operations

GET /api/internal/synap/user/{userId}/quota
  → Get Synap-specific quota
```

### LessTokens Service API

```
GET /api/internal/lesstokens/user/{userId}/usage
  → Get monthly API request count + remaining

POST /api/internal/lesstokens/usage/update
  → Increment API request counter

GET /api/internal/lesstokens/user/{userId}/quota
  → Get monthly quota
```

---

## MCP Server Generation (Planned)

Users can request MCP server endpoints for each service:

```
POST /api/mcp-servers
{
  "service": "vectorizer",
  "project_id": "uuid"
}
```

**Response**:
```json
{
  "id": "uuid",
  "endpoint": "https://api.hivehub.cloud/mcp/servers/{id}",
  "api_key": "mcp_sk_...",
  "service": "vectorizer"
}
```

**Behavior**:
- User can register this endpoint as an MCP server in Claude/IDE
- Requests to endpoint are authenticated with the MCP API key
- Requests forwarded to underlying Hive service
- Quota enforcement happens at both layers (HiveHub.Cloud + service)

---

## Error Handling & Resilience

### Retry Logic

- Transient failures (5xx, timeout) → exponential backoff (3 attempts)
- Permanent failures (4xx) → immediate error, no retry

### Circuit Breaker

- If service unavailable for >5 seconds → return 503 Service Unavailable
- After service recovers → gradual traffic increase (half-open state)

### Quota Exhaustion

- Vectorizer: 429 Quota Exceeded (vectors or collection limit)
- Nexus: 429 Quota Exceeded (nodes or relationships limit)
- LessTokens: 429 Quota Exceeded (monthly request limit)
- Storage: 403 Forbidden (storage limit reached)

### Timeout

- Service API calls: 10 second timeout
- File upload processing: 5 minute timeout
- Transmutation: configurable (default 2 minutes)

---

## Monitoring & Observability

### Metrics

- Service API response times (per service)
- Request success/error rates
- Quota exhaustion events
- File processing duration (Transmutation → Vectorizer → Nexus)

### Logs

- Request/response at INFO level
- Errors at ERROR level with context
- Structured logging: `{timestamp, service, user_id, error, trace_id}`

### Alerts

- Service endpoint down (5 consecutive failures)
- Quota enforcement triggered (logging only, no alert)
- Payment webhook processing failure
