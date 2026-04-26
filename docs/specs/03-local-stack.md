# 03 — Local Stack (docker-compose)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 02
>
> Implementation: [`docker-compose.yml`](../../docker-compose.yml), [`.env.example`](../../.env.example), [`bin/`](../../bin/) (bash + PowerShell wrappers), [`Makefile`](../../Makefile), and [`crates/cortex-ops/`](../../crates/cortex-ops/) which emits the bootstrap plan JSON consumed by `bin/cortex-init.sh`. `cortex-api` / `cortex-workers` container stanzas land with spec 04.

## Goal

Provide a single `docker-compose.yml` (plus a small `bin/cortex-up` wrapper) that brings the entire Cortex backend up on a developer's machine in one command. The stack must be reproducible, hot-restartable, and small enough to run on a 16 GB laptop while still being representative of production.

## Scope

**In:**
- Docker Compose service definitions for Vectorizer, Nexus, Synap, Meilisearch.
- Cortex-API and Cortex-Workers service definitions (built from the local repo).
- Volumes, networks, healthchecks, dependency ordering.
- Environment variables and `.env` template.
- Bootstrap scripts that create collections/indexes/streams from spec 02.
- A `cortex-up` / `cortex-down` / `cortex-reset` wrapper.

**Out:**
- Production deployment (Helm/Kubernetes — defer to Phase 5).
- TLS termination and auth in front of the stack (defer; localhost-only by default).
- Multi-node / HA topology (Vectorizer and Nexus already support this; not exercised here).

## Inputs / Outputs

### Service map

| Service           | Image                                                  | Ports (host:container) | Volume                 | Health |
|-------------------|--------------------------------------------------------|------------------------|------------------------|--------|
| `vectorizer`      | `hivellm/vectorizer:2.5.1`                             | `15001:15001`          | `vec-data:/data`       | `/health` |
| `nexus`           | `hivehub/nexus:v1.14.0`                                | `15002:15474`          | `nexus-data:/data`     | `/health` |
| `synap`           | `hivellm/synap:0.11.0`                                 | `15003:15003`          | `synap-data:/data`     | `PING`    |
| `meilisearch`     | `getmeili/meilisearch:v1.10`                           | `15004:7700`           | `meili-data:/meili_data`| `/health`|
| `cortex-api`      | (built locally from `./cortex-api/Dockerfile`)         | `15000:15000`          | `cortex-cas:/cas`, `cortex-archive:/archive`, `cortex-meta:/meta` | `/healthz` |
| `cortex-workers`  | (built locally from `./cortex-workers/Dockerfile`)     | —                      | shares `cortex-cas`, `cortex-archive`, `cortex-meta` | `/healthz` |

Port range `15000–15099` is reserved for Cortex local dev, picked to avoid collisions with the standalone defaults of each Hive service when running outside Cortex.

### Networks

- `cortex-net` — internal bridge; only `cortex-api` ports are exposed to host by default (operator-facing). All inter-service traffic stays on the bridge.

### Volumes

| Volume             | Purpose                                              |
|--------------------|------------------------------------------------------|
| `vec-data`         | Vectorizer `.vecdb` files + snapshots                |
| `nexus-data`       | Nexus WAL + page store                               |
| `synap-data`       | Synap snapshot + AOF                                 |
| `meili-data`       | Meilisearch LMDB                                     |
| `cortex-cas`       | SQLite blob store (CAS, spec 02)                     |
| `cortex-archive`   | Parquet event archive                                |
| `cortex-meta`      | SQLite metadata DB                                   |

All volumes are **named** (not bind mounts) so a `docker volume rm` reset is one command. A `--bind` flag on `cortex-up` switches to bind mounts under `./data/` for inspection.

### Environment (`.env.example`)

```dotenv
# Cortex
CORTEX_LOG_LEVEL=info
CORTEX_API_PORT=15000
CORTEX_BIND=127.0.0.1

# Backends (internal hostnames on the cortex-net bridge)
VECTORIZER_URL=http://vectorizer:15001
NEXUS_URL=http://nexus:15002
SYNAP_URL=redis://synap:15003
MEILI_URL=http://meilisearch:7700
MEILI_MASTER_KEY=cortex-dev-master-key   # dev only

# Classifier (spec 05)
CORTEX_CLASSIFIER_MODE=cli               # cli | sdk
CORTEX_CLASSIFIER_MODEL=claude-haiku-4-5
CLAUDE_CODE_BIN=claude                   # CLI path; required if mode=cli
ANTHROPIC_API_KEY=                       # required only if mode=sdk

# Embedder
CORTEX_EMBED_MODEL=nomic-embed-text-v1.5
CORTEX_EMBED_BATCH=64

# Budgets
CORTEX_CLASSIFIER_USD_PER_DAY=20

# Retention
CORTEX_RETENTION_PII_HIGH_DAYS=30
CORTEX_RETENTION_PII_MED_DAYS=365
CORTEX_RETENTION_FP32_TO_PQ_DAYS=30
CORTEX_RETENTION_PQ_TO_BIN_DAYS=365
```

### Bootstrap script — `bin/cortex-init.sh`

Runs once after `docker compose up -d` returns healthy:

1. **Vectorizer:** create the 12 collections from spec 02 §"Vectorizer collections" with the right HNSW params and embedding model.
2. **Nexus:** create database `cortex`; run a Cypher init that creates property indexes.
3. **Meilisearch:** create the 8 indexes from spec 02 §"Meilisearch indexes"; set primary keys, searchable attrs, ranking rules.
4. **Synap:** declare streams and pub/sub topics; set TTLs on KV namespaces.
5. **SQLite metadata:** apply `cortex-meta/schema.sql` (spec 02 §"Metadata store").
6. **CAS:** initialize empty `cas_blobs` table.
7. Verify each backend with a `cortex doctor` call.

Idempotent — re-running is a no-op unless `--reset` is passed.

### Wrapper CLI

```bash
bin/cortex-up           # docker compose up -d, wait for health, run cortex-init.sh if first run
bin/cortex-down         # docker compose down
bin/cortex-reset        # cortex-down + docker volume rm <all> + cortex-up + cortex-init.sh --reset
bin/cortex-logs [svc]   # docker compose logs -f [svc]
bin/cortex-doctor       # health probe + count of objects in each backend
```

Implemented in bash + a Powershell sibling for Windows hosts.

## Design

### Compose dependency ordering

```
synap, meilisearch, vectorizer, nexus  (no inter-deps; start in parallel)
        │
        └──> cortex-api (depends_on: all four, with `service_healthy` condition)
                │
                └──> cortex-workers (depends_on: cortex-api healthy)
```

`condition: service_healthy` is mandatory — Cortex services never start until their backends pass healthchecks, so retries on first boot are unnecessary.

### Resource limits (compose v3 deploy.resources)

For laptop dev, conservative defaults:

```yaml
vectorizer:    { cpus: 2.0, memory: 4G }
nexus:         { cpus: 1.5, memory: 2G }
synap:         { cpus: 0.5, memory: 1G }
meilisearch:   { cpus: 1.0, memory: 1G }
cortex-api:    { cpus: 1.0, memory: 512M }
cortex-workers:{ cpus: 2.0, memory: 1G }
```

Total: ~10 GB RAM, fits on a 16 GB laptop with room for the IDE. Production-like limits are set via override `docker-compose.production.yml`.

### Healthchecks

All defined inline in compose, 5 s interval, 30 s start-period, 3 retries. Failures restart the service.

```yaml
vectorizer:
  healthcheck:
    test: ["CMD", "curl", "-f", "http://localhost:15001/health"]
nexus:
  healthcheck:
    test: ["CMD", "curl", "-f", "http://localhost:15002/health"]
synap:
  healthcheck:
    test: ["CMD", "redis-cli", "-p", "15003", "PING"]
meilisearch:
  healthcheck:
    test: ["CMD", "curl", "-f", "http://localhost:7700/health"]
cortex-api:
  healthcheck:
    test: ["CMD", "curl", "-f", "http://localhost:15000/healthz"]
```

### Override files

- `docker-compose.override.yml` — auto-loaded by compose; default contains `--bind` mode disabled.
- `docker-compose.gpu.yml` — adds Vectorizer Metal/CUDA when host has a GPU.
- `docker-compose.production.yml` — production resource limits, TLS, optional Postgres replacing SQLite metadata.
- `docker-compose.classifier-sdk.yml` — wires `ANTHROPIC_API_KEY` from secret store when running classifier in SDK mode.

### Image strategy

- Hive services (`vectorizer`, `nexus`, `synap`) pulled from `hivehub/*` Docker Hub. Versions pinned in `.env`.
- `meilisearch` from official upstream, version pinned.
- `cortex-api` and `cortex-workers` are multi-stage Rust builds (`rust:1.92` → `gcr.io/distroless/cc-debian12`); ~30 MB final image.
- The Claude Code CLI is baked into the `cortex-workers` image when `CORTEX_CLASSIFIER_MODE=cli` (separate Dockerfile target `cortex-workers-with-cli` since it adds ~200 MB).

### First-run UX

After `git clone` and `cp .env.example .env`:

```
$ bin/cortex-up
[1/3] Pulling and building images...           ✓
[2/3] Starting services (this may take ~30s on first run)...
        vectorizer ........ healthy
        nexus ............. healthy
        synap ............. healthy
        meilisearch ....... healthy
        cortex-api ........ healthy
        cortex-workers .... healthy
[3/3] First-run init: provisioning collections, indexes, streams...
        12 Vectorizer collections created
        14 Nexus labels + indexes created
        8 Meilisearch indexes created
        6 Synap streams + 5 KV namespaces declared
        SQLite metadata schema applied
Cortex is up at http://127.0.0.1:15000  →  http://127.0.0.1:15000/dashboard
```

## Acceptance criteria

- [ ] `git clone && cp .env.example .env && bin/cortex-up` brings the stack up on a fresh machine in < 90 s (warm pull).
- [ ] All six healthchecks pass within the start period.
- [ ] `bin/cortex-doctor` reports green status for every backend after init.
- [ ] `docker stats` shows total RAM ≤ 10 GB at idle.
- [ ] Killing any backend container triggers automatic restart and Cortex-API recovers without operator intervention.
- [ ] `bin/cortex-reset` completes in < 60 s and produces an empty stack identical to a fresh install.
- [ ] Override `docker-compose.gpu.yml` is exercised in CI on a GPU runner (if available) without breaking non-GPU runs.
- [ ] Windows `.ps1` wrapper achieves parity with bash wrapper (same commands, same exit codes).

## Decisions

1. **Compose, not Kubernetes, for v1.** Kubernetes lives in Phase 5. Compose covers single-node dev + small-team self-host, which is all v1 needs.
2. **Named volumes default; bind mounts opt-in.** Avoid host-permission headaches on first run.
3. **Port range `15000–15099`.** Sequential, easy to remember, away from common dev tools.
4. **Synap exposed via Redis protocol** (it speaks RESP); reuse Redis CLI for diagnostics.
5. **Distroless final images** for Cortex services — small, secure, no shell to be hijacked.
6. **No TLS in v1 dev.** Default bind is `127.0.0.1` and the network is internal. Production override adds Caddy as TLS reverse proxy.

## Open questions

*(none — defaults locked)*

## References

- Spec 02 — Storage layout (what gets created on first run).
- Spec 04 — Cortex Core (the binary built by `cortex-api` / `cortex-workers`).
- Vectorizer `docker-compose.yml`, Nexus `docker-compose.yml`, Synap `docker-compose.yml` — referenced for canonical service config.
- Meilisearch official Docker docs.
