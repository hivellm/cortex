# HiveHub.Cloud: Operational Guide

**Deployment**: Docker (recommended) or native Rust binary  
**Database**: PostgreSQL 16+  
**Configuration**: Environment variables (12-factor app)

---

## Prerequisites

| Component | Version | Notes |
|-----------|---------|-------|
| **Rust** | 1.92+ | 2024 edition |
| **PostgreSQL** | 16+ | Managed or self-hosted |
| **Docker** | 20.10+ | Optional; for containerized deployment |
| **Node.js** | 20.x+ | For dashboard development (future) |

---

## Environment Configuration

### Required Variables

Create `.env` file at repository root:

```bash
# ===== DATABASE =====
DATABASE_URL=postgresql://hivehub_user:password@localhost:5432/hivehub_cloud
DATABASE_MAX_CONNECTIONS=100
DATABASE_MIN_CONNECTIONS=10

# ===== SERVER =====
HIVEHUB_SERVER_HOST=0.0.0.0
HIVEHUB_SERVER_PORT=3000
HIVEHUB_SERVER_WORKERS=4         # CPU cores recommended

# ===== JWT =====
JWT_SECRET=your-secret-key-min-32-chars-change-this-please
JWT_ACCESS_EXPIRATION=3600        # 1 hour (seconds)
JWT_REFRESH_EXPIRATION=604800     # 7 days (seconds)

# ===== FIREBASE (OAuth) =====
FIREBASE_PROJECT_ID=your-project-id
FIREBASE_CLIENT_EMAIL=your-service-account@...iam.gserviceaccount.com
FIREBASE_PRIVATE_KEY="-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----\n"

# ===== STRIPE =====
STRIPE_API_KEY=sk_live_xxxxx
STRIPE_PUBLISHABLE_KEY=pk_live_xxxxx
STRIPE_WEBHOOK_SECRET=whsec_xxxxx

# ===== PAYPAL =====
PAYPAL_CLIENT_ID=your-paypal-client-id
PAYPAL_SECRET=your-paypal-secret
PAYPAL_MODE=sandbox              # 'sandbox' or 'live'

# ===== SERVICE API KEYS (for internal services) =====
VECTORIZER_API_KEY=svc_vectorizer_xxxxx
VECTORIZER_ENDPOINT=http://localhost:15002
NEXUS_API_KEY=svc_nexus_xxxxx
NEXUS_ENDPOINT=http://localhost:15003
SYNAP_API_KEY=svc_synap_xxxxx
SYNAP_ENDPOINT=http://localhost:15004
LESSTOKENS_API_KEY=svc_lesstokens_xxxxx
LESSTOKENS_ENDPOINT=http://localhost:15005

# ===== STORAGE =====
STORAGE_DIR=/var/lib/hivehub/storage

# ===== LOGGING =====
RUST_LOG=info,hivehub_cloud_api=debug
RUST_LOG_STYLE=auto              # 'always', 'never', 'auto'
```

### Environment-Specific Overrides

**Development** (`.env.development`):
```bash
RUST_LOG=debug,hivehub_cloud_api=trace
DATABASE_URL=postgresql://postgres:postgres@localhost/hivehub_dev
STRIPE_API_KEY=sk_test_xxxxx
PAYPAL_MODE=sandbox
```

**Staging** (`.env.staging`):
```bash
RUST_LOG=info
DATABASE_URL=postgresql://hivehub_user:password@staging-db.example.com/hivehub_staging
STRIPE_API_KEY=sk_live_xxxxx
PAYPAL_MODE=live
```

**Production** (`.env.prod` — never commit):
```bash
RUST_LOG=warn,hivehub_cloud_api=info
DATABASE_URL=postgresql://hivehub_user:password@prod-db.example.com/hivehub_cloud
STRIPE_API_KEY=sk_live_xxxxx
JWT_SECRET=<use secrets manager, not .env file>
```

---

## Database Setup

### 1. Create Database & User

```bash
# Connect as PostgreSQL admin
psql -U postgres -h localhost

# Create database
CREATE DATABASE hivehub_cloud OWNER hivehub_user;

# Create user (if not exists)
CREATE USER hivehub_user WITH PASSWORD 'secure_password_here';

# Grant privileges
GRANT ALL PRIVILEGES ON DATABASE hivehub_cloud TO hivehub_user;
```

### 2. Run Migrations

```bash
cd apps/api

# Install SQLx CLI (if not already installed)
cargo install sqlx-cli --features postgres

# Set DATABASE_URL
export DATABASE_URL=postgresql://hivehub_user:password@localhost/hivehub_cloud

# Run migrations
sqlx migrate run

# Verify
psql -U hivehub_user -d hivehub_cloud -c "\dt"
```

### 3. Seed Initial Data (Optional)

```bash
# Create seed script (future feature)
# Currently: manual admin setup required
```

---

## Running the API

### Local Development

```bash
cd apps/api

# Build
cargo build

# Run with hot-reload (requires cargo-watch)
cargo install cargo-watch
cargo watch -x run

# Or run once
cargo run

# Server starts at http://localhost:3000
```

### Health Check

```bash
curl http://localhost:3000/health
# Expected: { "status": "ok", "database": "connected", "services": {...} }
```

### API Documentation

```
Swagger UI: http://localhost:3000/api/docs
OpenAPI Spec: http://localhost:3000/api/openapi.yaml
```

---

## Docker Deployment

### Build Image

```bash
docker build -t hivehub-cloud-api:latest -f Dockerfile .
```

### Docker Compose (Development)

Create `docker-compose.yml`:

```yaml
version: '3.8'
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: hivehub_user
      POSTGRES_PASSWORD: postgres
      POSTGRES_DB: hivehub_cloud
    ports:
      - "5432:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data

  api:
    build: .
    environment:
      DATABASE_URL: postgresql://hivehub_user:postgres@postgres:5432/hivehub_cloud
      HIVEHUB_SERVER_HOST: 0.0.0.0
      HIVEHUB_SERVER_PORT: 3000
      JWT_SECRET: dev-secret-key-change-in-prod
      RUST_LOG: debug
    ports:
      - "3000:3000"
    depends_on:
      - postgres
    command: sh -c "sqlx migrate run && cargo run"

volumes:
  postgres_data:
```

**Run**:
```bash
docker-compose up -d
```

### Production Docker Deployment

```bash
# Build optimized image
docker build --target production -t hivehub-cloud-api:v1.0.0 .

# Push to registry
docker push your-registry/hivehub-cloud-api:v1.0.0

# Run with secrets from environment
docker run \
  -e DATABASE_URL=postgresql://... \
  -e JWT_SECRET=<from-secrets-manager> \
  -e STRIPE_API_KEY=<from-secrets-manager> \
  -p 3000:3000 \
  your-registry/hivehub-cloud-api:v1.0.0
```

---

## Testing

### Unit & Integration Tests

```bash
cd apps/api

# Run all tests
cargo test --all-features

# Run specific test category
cargo test services::auth_tests
cargo test routes::

# Run with logging
RUST_LOG=debug cargo test -- --nocapture

# Generate coverage report
cargo install cargo-llvm-cov
cargo llvm-cov --html
# Open: target/llvm-cov/html/index.html
```

### Server-to-Server (S2S) Tests

Requires running Hive services (Vectorizer, Nexus, Synap, LessTokens).

```bash
cargo test --features s2s
```

### Slow Tests

Tests expected to take >10–20 seconds:

```bash
cargo test --features slow
```

### Test Database

```bash
# Create test database
createdb hivehub_test

# Run migrations on test DB
DATABASE_URL=postgresql://postgres@localhost/hivehub_test sqlx migrate run

# Tests use this DB automatically
```

---

## Monitoring & Logs

### Structured Logging

Logs are JSON-formatted (structured):

```json
{
  "timestamp": "2025-12-04T10:00:00Z",
  "level": "INFO",
  "module": "hivehub_cloud_api::routes::auth",
  "user_id": "uuid",
  "message": "User login successful",
  "trace_id": "abc123..."
}
```

### Log Levels

| Level | Usage |
|-------|-------|
| ERROR | Errors requiring immediate attention (failed payments, DB errors) |
| WARN | Warnings (quota approaching, slow queries) |
| INFO | Normal operations (login, file upload, API calls) |
| DEBUG | Detailed diagnostics (request/response bodies, SQL queries) |
| TRACE | Very detailed (function entry/exit, variable values) |

### Viewing Logs

**Development**:
```bash
# Real-time logs
cargo run 2>&1 | grep -i error

# Filter by module
RUST_LOG=hivehub_cloud_api::routes::auth=debug cargo run
```

**Production**:
```bash
# Docker logs
docker logs -f container_id

# Kubernetes logs
kubectl logs -f deployment/hivehub-cloud-api

# Log aggregation (ELK, Datadog, etc.): Configure log shipper
```

---

## Performance Tuning

### Database Connection Pool

```env
DATABASE_MIN_CONNECTIONS=10      # Start with 10 connections
DATABASE_MAX_CONNECTIONS=100     # Scale to 100 under load
```

**Tuning**:
- Increase if seeing "connection timeout" errors
- Decrease if DB is overloaded (check `pg_stat_activity`)

### Tokio Runtime

```env
HIVEHUB_SERVER_WORKERS=4         # Set to CPU core count
```

**Tuning**:
- 1 worker per CPU core optimal
- Too many workers: context-switch overhead
- Too few workers: underutilizes CPU

### Database Indexes

Indexes pre-created in migrations:

```sql
CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_subscriptions_user_id ON subscriptions(user_id);
CREATE INDEX idx_files_user_id ON files(user_id);
CREATE INDEX idx_files_status ON files(status);
```

**Monitor index usage**:
```sql
SELECT * FROM pg_stat_user_indexes WHERE idx_scan = 0;  -- Unused indexes
```

### Query Optimization

```bash
# Explain slow queries
EXPLAIN ANALYZE SELECT * FROM files WHERE user_id = '...';

# Look for sequential scans (should use indexes instead)
```

---

## Backup & Recovery

### Database Backups

**PostgreSQL Native**:
```bash
# Full backup
pg_dump -U hivehub_user -d hivehub_cloud > backup.sql

# Restore
psql -U hivehub_user -d hivehub_cloud < backup.sql
```

**Managed Services (AWS RDS, Azure)**:
- Automated daily snapshots (configurable retention)
- Point-in-time recovery available

### File Storage Backups

**Local Filesystem**:
```bash
# Manual backup
tar -czf storage_backup_$(date +%Y%m%d).tar.gz $STORAGE_DIR

# Restore
tar -xzf storage_backup_20251204.tar.gz -C /
```

**S3 (Future)**:
- Cross-region replication
- Versioning enabled
- Lifecycle rules for cost optimization

---

## Scaling

### Horizontal Scaling

API is stateless; scale by running multiple instances:

```bash
# Run 3 instances behind load balancer
docker run -p 3000:3000 ... api:v1
docker run -p 3001:3000 ... api:v1
docker run -p 3002:3000 ... api:v1

# Load balancer (HAProxy, Nginx, AWS ALB) distributes traffic
```

### Database Scaling

**Bottleneck**: Usually database becomes bottleneck before API.

**Solutions**:
1. **Connection pooling**: PgBouncer in front of PostgreSQL
2. **Read replicas**: Offload read-heavy queries to replicas
3. **Sharding**: Partition data by user_id (future, if >1B documents)

---

## Upgrades & Maintenance

### Zero-Downtime Deployment

1. Deploy new version to canary instance
2. Run smoke tests
3. Gradually shift traffic (10% → 50% → 100%)
4. Monitor errors; rollback if needed

**With load balancer**: Remove old instance from pool, deploy new, add back.

### Database Migrations

Migrations run automatically on startup:

```rust
// In main.rs
sqlx::migrate!("./migrations")
    .run(&pool)
    .await?;
```

**Backward-Compatible**: Always make migrations backward-compatible (don't drop columns immediately).

### Dependency Updates

```bash
cd apps/api

# Check for updates
cargo update

# Run tests before committing
cargo test --all-features
cargo clippy -- -D warnings
cargo fmt --check
```

---

## Troubleshooting

### Database Connection Error

```
Error: Error connecting to database
```

**Check**:
1. PostgreSQL running: `psql -U hivehub_user -d hivehub_cloud`
2. DATABASE_URL correct: `echo $DATABASE_URL`
3. Network connectivity: `nc -zv localhost 5432`

### Out of Memory

```
Error: OutOfMemory
```

**Check**:
1. Connection pool size: `DATABASE_MAX_CONNECTIONS`
2. Worker count: `HIVEHUB_SERVER_WORKERS`
3. Query performance: Slow queries holding open connections

### Quota Enforcement Not Working

**Check**:
1. Service API key correct: `echo $VECTORIZER_API_KEY`
2. Service endpoint reachable: `curl $VECTORIZER_ENDPOINT/health`
3. Quota calculation: Run `SELECT * FROM usage_metrics WHERE user_id = '...'`

### Payment Webhook Not Received

**Check**:
1. Webhook URL registered in Stripe/PayPal dashboard
2. Webhook secret matches: `STRIPE_WEBHOOK_SECRET`
3. Logs: `RUST_LOG=debug cargo run | grep webhook`

---

## SLA & Targets

| Metric | Target | Notes |
|--------|--------|-------|
| **Uptime** | 99.5% | 3.6 hours downtime/month acceptable |
| **Response Time (p95)** | <100ms | Most endpoints <5ms; some slower |
| **Error Rate** | <0.1% | <1 error per 1000 requests |
| **Database Latency (p95)** | <50ms | Indicates DB health |
| **Throughput** | 10,000+ req/sec | Per instance; scales horizontally |
