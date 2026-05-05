# Cortex — Operational Deployment & Monitoring

## Docker Deployment

**Image:** `cortex/ingestion:dev` (and per-worker targets) from multi-stage Dockerfile.

### Build & Deployment

**Build command:**
```bash
# From Cortex repo root:
CORTEX_GIT_SHA=$(git rev-parse HEAD) \
CORTEX_GIT_DIRTY=$(git status --porcelain | head -c1 | grep -q . && echo true || echo false) \
docker compose build
```

**Run command:**
```bash
docker compose up -d
```

This brings up: Vectorizer, Nexus, Synap, Meilisearch, cortex-ingestion, cortex-classifier-worker, cortex-embedder-worker, cortex-fulltext-worker, cortex-graph-worker, cortex-api, cortex-claude-archive (optional, phase11i §5).

**Key build features (Dockerfile phases):**
- Multi-stage builder (Debian trixie-dev, Rust 1.93.1 via rustup).
- BuildKit cache mounts: cargo registry, git index, target dir stay warm across rebuilds (fast iteration).
- Per-binary leaf stages: `docker-compose.yml` targets by name (e.g., `target: cortex-api`).
- Git SHA / dirty flag forwarded at build time (phase11e hotfix) — `/healthz` version block stamps real values, not "unknown".

### Ports (default + configurable)

| Service | Container Port | Host Port (env override) | Purpose |
|---------|-----------------|--------------------------|---------|
| cortex-api | 17000 | `CORTEX_API_PORT` | Query API + dashboard backend |
| cortex-ingestion | 17010 | `CORTEX_INGESTION_PORT` | Event ingest bus |
| cortex-adapter-claude | 17011 | N/A (local socket) | Hook daemon (Claude Code) |
| cortex-classifier-worker | 17021 | `CORTEX_CLASSIFIER_PORT` | Classification + health |
| cortex-embedder-worker | 17022 | `CORTEX_EMBEDDER_PORT` | Embedding + health |
| cortex-fulltext-worker | 17023 | `CORTEX_FULLTEXT_PORT` | Indexing + health |
| cortex-graph-worker | 17024 | `CORTEX_GRAPH_PORT` | Graph writes + health |
| cortex-claude-archive | 17030 | `CORTEX_CLAUDE_ARCHIVE_PORT` | Archive watcher + health (phase11i §5) |
| **vectorizer** | 15002 → | `VECTORIZER_HOST_PORT` (17001) | Vector API |
| **nexus** | 15474 → | `NEXUS_HOST_PORT` (17002) | Graph API |
| **synap** (HTTP) | 15500 → | `SYNAP_HOST_PORT` (17003) | Event bus HTTP |
| **synap** (WS) | 15501 → | `SYNAP_WS_HOST_PORT` (17013) | Event bus WebSocket |
| **synap** (RESP) | 6379 → | `SYNAP_RESP_HOST_PORT` (16379) | Redis-compatible |
| **meilisearch** | 7700 → | `MEILI_HOST_PORT` (17004) | Search API |

### Environment Variables (Key Cortex)

```bash
# Ingestion
CORTEX_INGESTION_BIND=0.0.0.0:17010
CORTEX_SYNAP_URL=http://synap:15500
CORTEX_ARCHIVE_ROOT=/var/lib/cortex/archive

# Workers
CORTEX_CLASSIFIER_BIND=0.0.0.0:17021
CORTEX_CLASSIFIER_SYNAP_URL=http://synap:15500

CORTEX_EMBEDDER_BIND=0.0.0.0:17022
CORTEX_EMBEDDER_SYNAP_URL=http://synap:15500
CORTEX_EMBEDDER_VECTORIZER_URL=http://vectorizer:15002
CORTEX_EMBEDDER_VECTORIZER_USER=admin (default)
CORTEX_EMBEDDER_VECTORIZER_PASSWORD=cortex-dev-admin (default)
CORTEX_EMBEDDER_DIM=512

CORTEX_FULLTEXT_BIND=0.0.0.0:17023
CORTEX_FULLTEXT_SYNAP_URL=http://synap:15500
CORTEX_FULLTEXT_MEILI_URL=http://meilisearch:7700
CORTEX_FULLTEXT_MEILI_API_KEY=cortex-dev-master-key (default)
CORTEX_FULLTEXT_BATCH=100
CORTEX_FULLTEXT_WORKERS=1

CORTEX_GRAPH_BIND=0.0.0.0:17024
CORTEX_GRAPH_SYNAP_URL=http://synap:15500
CORTEX_GRAPH_NEXUS_URL=http://nexus:15474
CORTEX_GRAPH_CYPHER_DIR=/opt/cortex/cypher

# API
CORTEX_HOME=/var/lib/cortex
CORTEX_INGESTION_URL=http://cortex-ingestion:17010
CORTEX_SYNAP_URL=http://synap:15500
CORTEX_NEXUS_URL=http://nexus:15474
CORTEX_VECTORIZER_URL=http://vectorizer:15002
CORTEX_VECTORIZER_USER=admin (default)
CORTEX_VECTORIZER_PASSWORD=cortex-dev-admin (default)
CORTEX_VECTORIZER_JWT_WARMUP_SECS=0
CORTEX_FULLTEXT_MEILI_URL=http://meilisearch:7700
CORTEX_FULLTEXT_MEILI_API_KEY=cortex-dev-master-key (default)
CORTEX_RULEBOOK_ROOTS=/workspaces/Cortex/.rulebook,... (multi-repo, comma-separated)
CORTEX_COVERAGE_SLUGS_ONLY=cortex,vectorizer,nexus,synap,rulebook,... (for coverage metrics)
CORTEX_QUERY_REPORT_MISSING_COLLECTIONS=0 (set to 1 to log Vectorizer misses)

# Archive watcher (phase11i §5)
CORTEX_CLAUDE_ARCHIVE_BIND=0.0.0.0:17030
CORTEX_SYNAP_URL=http://synap:15500
CORTEX_CLAUDE_PROJECTS_HOST=${USERPROFILE}/.claude/projects (bind mount source)

# Upstream services
VECTORIZER_ADMIN_PASSWORD=cortex-dev-admin (default)
MEILI_MASTER_KEY=cortex-dev-master-key (default)
MEILI_ENV=development
```

### Persistent Volumes

| Mount | Host path | Container path | Purpose |
|-------|-----------|-----------------|---------|
| cortex-home | `${CORTEX_HOME_HOST}` (default `~/.cortex`) | `/var/lib/cortex` | Archive, consumer offsets, logs |
| cortex-workspaces | `${CORTEX_WORKSPACES_HOST}` (default `..` = parent dir) | `/workspaces:ro` | Bound Cortex/Nexus/Synap/etc. repos for bootstrap |
| claude-projects | `${CORTEX_CLAUDE_PROJECTS_HOST}` (default `~/.claude/projects`) | `/data/claude-projects:ro` | Claude Code archive for cortex-claude-archive watcher |
| vec-data | `vec-data` (named volume) | `/data` | Vectorizer persistence |
| nexus-data | `nexus-data` (named volume) | `/app/data` | Nexus persistence (note: v2.x uses `/app/data`, not `/data`) |
| synap-data | `synap-data` (named volume) | `/data` | Synap persistence |
| meili-data | `meili-data` (named volume) | `/meili_data` | Meilisearch persistence |

### Health Checks & Readiness

Each Cortex service exposes `/healthz` (container listen port):

```bash
# Via docker-compose:
curl http://localhost:17000/healthz         # cortex-api
curl http://localhost:17010/healthz         # cortex-ingestion
curl http://localhost:17021/healthz         # classifier-worker
curl http://localhost:17022/healthz         # embedder-worker
curl http://localhost:17023/healthz         # fulltext-worker
curl http://localhost:17024/healthz         # graph-worker
curl http://localhost:17030/healthz         # claude-archive (phase11i §5.2)
```

**Health aggregator:** cortex-api `/v1/status` probes all upstream services and reports:
- Vectorizer, Nexus, Synap, Meilisearch connectivity.
- Worker reachability (classifier, embedder, fulltext, graph, archive-watcher).
- Consumer offset lag (if available).
- Classifier mode (StaticClassifier vs. HaikuCli).

### Docker-Compose Healthcheck Details

Example from cortex-api:
```yaml
healthcheck:
  test: ["CMD-SHELL", "curl -fsS http://127.0.0.1:17000/healthz >/dev/null || exit 1"]
  interval: 10s
  timeout: 3s
  start_period: 20s
  retries: 6
```

All services have similar checks with staggered `start_period` values (15s–30s) to allow initialization.

## Monitoring & Observability

### Logging

- **Level control:** `RUST_LOG` env var (e.g., `RUST_LOG=cortex=debug,synap_sdk=info`).
- **Output:** stdout (Docker captures in `/var/lib/docker/containers/*/`); can be redirected to syslog/ELK.
- **Per-crate:** Each crate uses `tracing` (not `log`). Subscriber initialized at binary startup.

### Metrics (Not Yet Centralized)

Phase 4 hardening work includes:
- Prometheus scrape targets (planned; not yet exposed).
- Per-worker operation counts (classify, embed, index, graph-write success/fail).
- Query lane latencies (vector, keyword, graph, RRF fusion).
- Consumer offset lag per worker.

### Admin Operations (cortex-ops)

**Planned subcommands (not yet implemented):**
- `cortex ops doctor` — consistency check across backends (Vectorizer, Nexus, Meili, archive).
- `cortex ops prune` — cleanup stale indexes, re-index after SDK fixes.
- `cortex ops reindex` — rebuild a specific backend (e.g., after Meili version upgrade).

See `.rulebook/tasks/phase4d_consistency-check` (future).

## CI/CD Integration

### GitHub Workflows

- `.github/workflows/health-smoke.yml` — runs `docker compose up` on every PR, runs smoke tests against live stack.
- `.github/workflows/doctor.yml` — phase4d consistency checks (planned).

**Fixture repos:** 3 repos walked by bootstrap (Cortex, Nexus, Rulebook) for CI testing. Env var: `CORTEX_BOOTSTRAP_FIXTURE_REPOS=3`.

## Troubleshooting

**Common issues:**

| Symptom | Diagnosis | Fix |
|---------|-----------|-----|
| `cortex-api` can't reach Vectorizer | Check `CORTEX_VECTORIZER_URL` (should be `http://vectorizer:15002` in docker, `http://localhost:17001` on host) | Update docker-compose.yml or `.env` |
| Workers restart loop | Check worker healthcheck (`docker logs <worker-name>`) | Usually a dependency not ready; stagger `start_period` |
| Meili index not created | Check `CORTEX_FULLTEXT_MEILI_API_KEY` matches `MEILI_MASTER_KEY` | Sync env vars in docker-compose.yml |
| Archive watcher not tailing | Check `CORTEX_CLAUDE_PROJECTS_HOST` mount exists and contains `ULID/` subdirs | Verify Claude Code has run at least once on host |
| Consumer offset not advancing | Check SQLite file in `CORTEX_ARCHIVE_ROOT` is writable | `ls -la ~/.cortex/cortex-*.consumer-state.sqlite` |

## Performance Tuning (Not Yet Documented)

Future work (phase4f):
- Async buffer sizes per worker.
- Batch window tuning (Meili, Vectorizer batch sizes).
- Query RRF weights per intent.
- Cache TTL for pre-thinking bundles.
