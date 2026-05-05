# HiveHub.Cloud: Data & Storage Model

**Database**: PostgreSQL 16+ (Diesel ORM)  
**File Storage**: `STORAGE_DIR` (local filesystem or S3 future)  
**Distributed Data**: Vectors in Vectorizer, graphs in Nexus, KV in Synap

---

## PostgreSQL Schema

### Core Tables

#### `users`

Stores user accounts.

```sql
CREATE TABLE users (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  email VARCHAR(255) UNIQUE NOT NULL,
  password_hash VARCHAR(255),  -- bcrypt hash (cost 12); NULL if OAuth-only
  name VARCHAR(255),
  google_id VARCHAR(255),      -- OAuth identifier
  github_id VARCHAR(255),      -- OAuth identifier
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  deleted_at TIMESTAMP         -- Soft delete
);

CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_google_id ON users(google_id);
CREATE INDEX idx_users_github_id ON users(github_id);
```

**Key Invariants**:
- `email` is globally unique
- At least one of `password_hash`, `google_id`, `github_id` must be set
- Soft deletes are used (not hard delete)

---

#### `subscriptions`

Tracks user plan and billing status.

```sql
CREATE TABLE subscriptions (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id),
  plan VARCHAR(50) NOT NULL,  -- 'free', 'pro', 'enterprise'
  status VARCHAR(50) NOT NULL DEFAULT 'active',  -- 'active', 'pending', 'cancelled', 'expired'
  started_at TIMESTAMP NOT NULL,
  renews_at TIMESTAMP,        -- Next billing date (NULL for free plan)
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_subscriptions_user_id ON subscriptions(user_id);
CREATE INDEX idx_subscriptions_status ON subscriptions(status);
```

**Plan Configuration** (from code):
```rust
pub enum Plan {
  Free { documents: 1000, storage_mb: 50, collections: 1 },
  Pro { documents: 10000, storage_mb: 500, collections: 5 },
  Enterprise { documents: 100000, storage_mb: 5000, collections: 99999 }
}
```

**Rules**:
- One active subscription per user at a time
- Free plan: no `renews_at` (perpetual)
- Paid plans: `renews_at` set at purchase time
- Downgrade resets `renews_at` to new renewal date

---

#### `usage_metrics`

Tracks resource consumption against plan limits.

```sql
CREATE TABLE usage_metrics (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL UNIQUE REFERENCES users(id),
  document_count INT DEFAULT 0,
  storage_bytes BIGINT DEFAULT 0,
  api_requests INT DEFAULT 0,  -- LessTokens API calls (monthly)
  last_reset TIMESTAMP,         -- Monthly reset date for API request counter
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_usage_metrics_user_id ON usage_metrics(user_id);
```

**Rules**:
- One row per user
- `document_count` and `storage_bytes` accumulate (never reset)
- `api_requests` counter resets monthly (on `last_reset` date)
- When quota exceeded: operation is blocked (error 429 or 403)

---

#### `projects`

User projects (organizational unit for collections/graphs).

```sql
CREATE TABLE projects (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id),
  name VARCHAR(255) NOT NULL,
  description TEXT,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  deleted_at TIMESTAMP  -- Soft delete
);

CREATE INDEX idx_projects_user_id ON projects(user_id);
```

**Rules**:
- User can have multiple projects
- Quotas are shared across all projects (not per-project)
- Deleting a project cascades to `files` and external service collections/graphs

---

#### `files`

File upload metadata and processing status.

```sql
CREATE TABLE files (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id),
  project_id UUID NOT NULL REFERENCES projects(id),
  name VARCHAR(255) NOT NULL,
  content_type VARCHAR(100),
  storage_path VARCHAR(1024),        -- Path in STORAGE_DIR
  file_size_bytes BIGINT,
  status VARCHAR(50) DEFAULT 'pending',  -- 'pending', 'processing', 'indexed', 'failed'
  chunks_count INT,                  -- Number of chunks from Transmutation
  vectors_stored INT,                -- Number of vectors in Vectorizer collection
  vectorizer_collection_id UUID,     -- Reference to collection in Vectorizer
  error_message TEXT,                -- If status='failed'
  uploaded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  processed_at TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_files_user_id ON files(user_id);
CREATE INDEX idx_files_project_id ON files(project_id);
CREATE INDEX idx_files_status ON files(status);
```

**Processing Workflow**:
1. User uploads file → status='pending'
2. Async job sends to Transmutation → status='processing'
3. Markdown returned → chunks created
4. Chunks sent to Vectorizer → vectors stored → status='indexed'
5. If error → status='failed', error_message populated

---

#### `file_chunks`

Chunks extracted from Transmutation markdown.

```sql
CREATE TABLE file_chunks (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  file_id UUID NOT NULL REFERENCES files(id),
  chunk_index INT NOT NULL,
  content TEXT NOT NULL,
  vector_id UUID,  -- Reference to vector in Vectorizer
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_file_chunks_file_id ON file_chunks(file_id);
```

---

#### `sessions`

Active user sessions (for refresh token invalidation).

```sql
CREATE TABLE sessions (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id),
  refresh_token_hash VARCHAR(255) NOT NULL,
  expires_at TIMESTAMP NOT NULL,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_sessions_user_id ON sessions(user_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);
```

**Rules**:
- One session per login
- Refresh token is hashed before storing (not stored plain-text)
- Expired sessions can be garbage-collected
- Logout deletes the session

---

#### `payment_records`

Payment transaction history.

```sql
CREATE TABLE payment_records (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id),
  provider VARCHAR(50) NOT NULL,  -- 'stripe', 'paypal'
  provider_id VARCHAR(255),        -- charge ID, payment ID
  amount_cents INT NOT NULL,
  currency VARCHAR(3) DEFAULT 'USD',
  status VARCHAR(50) NOT NULL,    -- 'pending', 'succeeded', 'failed'
  plan VARCHAR(50),                -- Target plan
  billing_cycle_start TIMESTAMP,
  billing_cycle_end TIMESTAMP,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_payment_records_user_id ON payment_records(user_id);
CREATE INDEX idx_payment_records_provider_id ON payment_records(provider_id);
CREATE INDEX idx_payment_records_status ON payment_records(status);
```

---

#### `mcp_servers` (Planned)

Authenticated MCP server endpoints.

```sql
CREATE TABLE mcp_servers (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id),
  project_id UUID NOT NULL REFERENCES projects(id),
  service VARCHAR(50) NOT NULL,  -- 'vectorizer', 'nexus', 'synap'
  name VARCHAR(255),
  api_key_hash VARCHAR(255),     -- Hashed MCP-specific API key
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  last_used TIMESTAMP
);

CREATE INDEX idx_mcp_servers_user_id ON mcp_servers(user_id);
```

---

## Data Segregation Strategy

### At Application Level (PostgreSQL)

- **User isolation**: All queries filtered by `WHERE user_id = $1`
- **Project isolation**: File/collection access checks project ownership
- **Multi-tenancy**: Single database; no separate DB per tenant

### At Service Level (Vectorizer/Nexus/Synap)

- **Vectorizer**: Collections are namespaced by user ID (e.g., `user-{userId}-collection`)
- **Nexus**: Graphs are namespaced by user ID
- **Synap**: Key-value namespaces are user-scoped

**Cross-Service Validation**:
- Before creating collection in Vectorizer, check user quota in HiveHub.Cloud
- HiveHub.Cloud provides list of collections via Service API
- Services call back to validate quota/ownership before indexing

---

## File Storage

### Local Filesystem (Development)

```
STORAGE_DIR/
├── {user_id}/
│   ├── {project_id}/
│   │   ├── {file_id}.pdf
│   │   ├── {file_id}.docx
│   │   └── {file_id}.txt
```

**Configuration**: `STORAGE_DIR` env var (default: `/var/lib/hivehub/storage`)

### S3 Storage (Planned)

Future migration path:
- Upload to S3 bucket with user/project prefix
- Pre-signed URLs for download
- Lifecycle rules for cleanup

---

## Backup & Recovery

### Database Backups

- PostgreSQL continuous archiving (WAL)
- Daily snapshots (AWS RDS automated backups)
- 30-day retention

### File Backups

- S3 cross-region replication (when migrated)
- Or filesystem-level snapshots

---

## Data Retention & Deletion

### User Deletion

Hard delete on request:
1. Find all projects → cascade delete files
2. Find all collections in Vectorizer → delete via Service API
3. Find all graphs in Nexus → delete via Service API
4. Delete subscriptions, sessions, payment records
5. Soft-delete or hard-delete user record

**Compliance**: GDPR right-to-be-forgotten support (future)

### Expired Sessions

Automatic cleanup: delete from `sessions` where `expires_at < NOW()`

### File Cleanup

After file deletion:
1. Remove file from `STORAGE_DIR`
2. Remove vectors from Vectorizer (via API)
3. Mark file as deleted_at (soft delete in DB)

---

## Performance Considerations

### Indexes

- `users.email` – Authentication lookup
- `subscriptions.user_id`, `subscriptions.status` – Quota/status queries
- `files.user_id`, `files.status` – File list/status queries
- `sessions.expires_at` – Cleanup queries

### Connection Pooling

- Min connections: 10
- Max connections: 100 (configurable)
- Connection timeout: 30 seconds

### Query Patterns

- Single user profile: index on `users.id`
- Quota check: index on `subscriptions.user_id`
- File upload count: index on `files.user_id` + aggregation
- Payment history: index on `payment_records.user_id`

---

## Migration Strategy

**Initial**: SQLx migrations (Diesel in code)  
**Naming**: `YYYYMMDD-description.sql`  
**Execution**: `sqlx migrate run` (idempotent)  
**Rollback**: Manual (create revert migration)

**Never in production**: `sqlx migrate revert` (not supported for safety)
