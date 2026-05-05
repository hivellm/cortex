# HiveHub.Cloud: Architecture & Modules

**Version**: 1.0.0  
**Design**: Monorepo (Rust API + React dashboard planned); stateless backend; PostgreSQL persistence

---

## Monorepo Structure

```
hivehub-cloud/
├── apps/
│   ├── api/                    # Rust backend (Axum/Tokio)
│   │   ├── src/
│   │   │   ├── routes/         # HTTP endpoint handlers
│   │   │   ├── services/       # Business logic (auth, projects, subscriptions, payments)
│   │   │   ├── models/         # Data types (User, Project, Subscription, etc.)
│   │   │   ├── middleware/     # Authentication, error handling
│   │   │   ├── integrations/   # Vectorizer, Nexus, Synap, LessTokens clients
│   │   │   └── db/             # Database models + SQLx/Diesel queries
│   │   ├── migrations/         # SQL migration files (Diesel/SQLx)
│   │   ├── tests/              # Integration + E2E tests
│   │   └── Cargo.toml          # Dependencies (Axum, Tokio, Diesel, etc.)
│   └── dashboard/              # React frontend (planned; not started)
├── docs/
│   ├── ARCHITECTURE.md         # This file
│   ├── DEPLOYMENT.md           # Deployment procedures
│   ├── specs/                  # Feature specifications
│   │   ├── AUTHENTICATION.md
│   │   ├── SUBSCRIPTION.md
│   │   ├── SERVICE_API.md      # Internal API for Hive services
│   │   ├── INTEGRATIONS.md
│   │   └── ...
│   └── api/
│       └── openapi.yaml        # REST API specification
└── rulebook/                   # Task management + specs
```

---

## Core Modules

### 1. Authentication (`src/services/auth`)

**Responsibility**: User registration, login, token management, OAuth integration.

**Key Components**:
- `register()` – Email/password registration with bcrypt hashing (cost 12)
- `login()` – Email/password login; JWT token generation
- `google_oauth_callback()` – Google OAuth flow; user creation/linking
- `github_oauth_callback()` – GitHub OAuth flow; user creation/linking
- `refresh_token()` – JWT refresh token validation and renewal
- `verify_jwt()` – Middleware JWT validation on protected routes

**Persists To**: `users` table (email, password_hash, google_id, github_id, created_at)

**Tokens**:
- Access token: 1 hour (15 min in code, 1 hour in spec)
- Refresh token: 7 days
- Both JWT-signed with `JWT_SECRET` env var

---

### 2. Subscriptions (`src/services/subscriptions`)

**Responsibility**: Plan management, usage tracking, quota enforcement.

**Key Components**:
- `create_subscription()` – Assign plan (Free/Pro/Enterprise) to user
- `check_quota()` – Validate document/storage/API request limits before operation
- `update_usage()` – Record usage metrics after operation
- `upgrade_downgrade()` – Plan changes (triggers payment if upgrade)
- `enforce_quota()` – Block operations when limits hit

**Plans**:
```rust
Free: 1_000 docs, 50 MB, 200 LessTokens req/mo
Pro: 10_000 docs, 500 MB, 2_000 LessTokens req/mo
Enterprise: 100_000 docs, 5 GB, 20_000 LessTokens req/mo
```

**Persists To**: `subscriptions`, `usage_metrics` tables

---

### 3. Projects (`src/services/projects`)

**Responsibility**: User project organization and resource allocation.

**Key Components**:
- `create_project()` – New project for user
- `list_projects()` – User's projects with quota summaries
- `get_project()` – Project details + collection/graph references
- `delete_project()` – Remove project and cascade collections/graphs

**Multi-Project Model**:
- User can have multiple projects
- Each project has its own Vectorizer collections + Nexus graphs
- Storage/document limits are **shared** across all projects
- Project metadata stored; actual data namespaced in downstream services

**Persists To**: `projects` table (user_id, name, created_at, deleted_at)

---

### 4. Payments (`src/services/payments`)

**Responsibility**: Stripe/PayPal integration, webhook handling, invoice tracking.

**Key Components**:
- `create_stripe_session()` – Checkout session for plan upgrade
- `handle_stripe_webhook()` – Listen for payment_intent.succeeded, invoice.payment_failed
- `handle_paypal_webhook()` – PayPal payment notifications
- `track_invoice()` – Record payment records with timestamps
- `estimate_renewal()` – Next billing date calculation

**Workflow**:
1. User selects plan upgrade in dashboard
2. API creates Stripe/PayPal checkout session
3. User completes payment on provider site
4. Provider webhooks back to `/api/webhooks/stripe` or `/api/webhooks/paypal`
5. API validates webhook signature
6. API updates subscription status in DB
7. User can now use new plan

**Recurring Billing**: Planned; not yet spec'd.

**Persists To**: `payment_records`, `invoices` tables

---

### 5. File Upload & Processing (`src/services/files`)

**Responsibility**: File storage, Transmutation conversion, Vectorizer indexing.

**Workflow**:
1. User uploads file (dashboard or `/api/upload`)
2. File stored in `STORAGE_DIR` (or S3 future)
3. Async job: send to Transmutation service
4. Transmutation returns Markdown
5. Markdown chunked (parameterized window size)
6. Chunks embedded via Vectorizer
7. Vectors stored in user's collection
8. File metadata (vectors, embeddings_count) stored in DB

**Persists To**: `files`, `file_chunks` tables + Vectorizer collections

---

### 6. Service API (`src/routes/internal/`)

**Responsibility**: Internal endpoints for Hive services to query user quotas and validate data.

**Endpoints**:
- `GET /api/internal/vectorizer/user/{userId}/collections` – List user's collections
- `GET /api/internal/vectorizer/collection/{collectionId}/validate` – Check ownership + quota
- `GET /api/internal/nexus/user/{userId}/databases` – List user's graphs
- `GET /api/internal/synap/user/{userId}/keys` – List user's KV namespaces
- `POST /api/internal/{service}/usage/update` – Record usage after operation

**Authentication**: Service API key (Bearer token + X-Service-Name header)

**Rate Limiting**: Per-service key rate limit (planned)

---

### 7. Admin Dashboard (`src/routes/admin/`)

**Responsibility**: Admin-only analytics, user management, system health.

**Endpoints** (planned):
- `GET /api/admin/users` – List all users with subscription status
- `GET /api/admin/revenue` – Sales/revenue reports
- `GET /api/admin/system/health` – Database, service endpoint status
- `POST /api/admin/users/{userId}/reset-quota` – Manual quota reset

---

## Database Schema (PostgreSQL)

**Core Tables**:
- `users` – (id, email, password_hash, google_id, github_id, created_at)
- `subscriptions` – (id, user_id, plan, started_at, renews_at, status)
- `projects` – (id, user_id, name, created_at)
- `usage_metrics` – (id, user_id, document_count, storage_bytes, api_requests, last_reset)
- `payment_records` – (id, user_id, provider, amount_cents, status, stripe_id/paypal_id)
- `files` – (id, user_id, project_id, name, content_type, storage_path, uploaded_at)
- `sessions` – (id, user_id, refresh_token_hash, expires_at)
- `audit_logs` – (id, user_id, action, resource_id, timestamp) [planned]

**No direct storage of vectors/graph/KV data**; those reside in Vectorizer/Nexus/Synap.

---

## External Service Integrations

| Service | Purpose | Communication |
|---------|---------|-----------------|
| **Vectorizer** | Vector embedding, semantic search, collection mgmt | REST API + internal SDK |
| **Nexus** | Knowledge graphs, nodes, relationships, classifications | REST API + internal SDK |
| **Synap** | Key-value store, queues, pub/sub | REST API + internal SDK |
| **LessTokens** | Token-optimized LLM requests | REST API via proxy |
| **Stripe** | Payment processing, subscriptions | REST API + webhooks |
| **PayPal** | Alternative payment provider | REST API + webhooks |
| **Firebase** | OAuth provider (planned; not yet integrated) | REST API + client SDK |

**Service Endpoints** (from env vars):
```toml
[services]
vectorizer_endpoint = "http://localhost:15002"
nexus_endpoint = "http://localhost:15003"
synap_endpoint = "http://localhost:15004"
lesstokens_endpoint = "http://localhost:15005"
```

---

## Performance Characteristics

**Expected Throughput**: 10,000+ req/sec per instance (Rust + Tokio)  
**Expected Latency**: <5ms p95 for most endpoints  
**Memory**: 50-100MB baseline  
**Concurrent Connections**: 10,000+ per instance  
**Database**: Connection pool (min 10, max 100 connections by default)

---

## Deployment Model

- **API**: Single Rust binary (Docker container)
- **Database**: Managed PostgreSQL (AWS RDS, Azure, self-hosted)
- **Configuration**: Environment variables (12-factor app)
- **Scaling**: Horizontal (stateless API + database connection pooling)
- **Monitoring**: Structured logging (tracing crate); metrics TBD
