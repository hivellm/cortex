# 03 — Local Stack (docker-compose)

> **Status:** 🟢 Implemented · **Owner:** Core team · **Depends on:** 02
>
> Implementation: [`docker-compose.yml`](../../docker-compose.yml), [`.env.example`](../../.env.example), [`bin/`](../../bin/) (bash + PowerShell wrappers), [`Makefile`](../../Makefile), and the `cortex-ops` binary (a `[[bin]]` target inside [`crates/cortex-cli/`](../../crates/cortex-cli/) — `cargo run -p cortex-cli --bin cortex-ops`) which emits the bootstrap plan JSON consumed by `bin/cortex-init.sh` and hosts the `doctor*` probes.

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

### Service map (12 services — reconciled with the shipped `docker-compose.yml`, 2026-07-14)

**Backends (pulled images):**

| Service       | Image                                | Ports (host:container, defaults)                    | Volume(s)                                   | Health |
|---------------|--------------------------------------|-----------------------------------------------------|---------------------------------------------|--------|
| `vectorizer`  | `hivehub/vectorizer:3.5.0-fastembed` | `17001:15002` (REST), `17005:15503` (binary RPC)    | `vec-data:/data` (config), `vec-state:/.local/share/vectorizer` (real state — collections/vectors/auth) | `/health` (no compose healthcheck) |
| `nexus`       | `hivehub/nexus:2.5.0`                | `17002:15474` (REST), `17012:15475` (binary RPC)    | `nexus-data:/app/data`                      | `nexus-server --healthcheck` (exec-form; image is distroless, no shell) |
| `synap`       | `hivehub/synap:1.0.0`                | `17003:15500` (HTTP), `17013:15501` (WS), `16379:6379` (RESP) | `synap-data:/data`                | `synap-server --health-check` (exec-form; distroless) |
| `meilisearch` | `getmeili/meilisearch:v1.10`         | `17004:7700`                                        | `meili-data:/meili_data`                    | `wget /health` |
| `cortex-reranker` | `ghcr.io/huggingface/text-embeddings-inference:89-1.8` | — (bridge-only)                   | `tei-data:/data`                            | none |

**Cortex services (built locally, `cortex/<name>:dev`):**

| Service                    | Image                        | Port (host)      | Depends on                                          |
|----------------------------|------------------------------|------------------|-----------------------------------------------------|
| `cortex-ingestion`         | `cortex/ingestion:dev`       | `17010`          | `synap`                                             |
| `cortex-classifier-worker` | `cortex/classifier-worker:dev` | `17021`        | `synap`, `cortex-ingestion`                         |
| `cortex-embedder-worker`   | `cortex/embedder-worker:dev` | `17022`          | `synap`, `vectorizer`                               |
| `cortex-fulltext-worker`   | `cortex/fulltext-worker:dev` | `17023`          | `synap`, `meilisearch`                              |
| `cortex-graph-worker`      | `cortex/graph-worker:dev`    | `17024`          | `synap`, `nexus`                                    |
| `cortex-api`               | `cortex/api:dev`             | `17000`          | `synap`, `nexus`, `meilisearch`, `vectorizer`, `cortex-ingestion` |
| `cortex-claude-archive`    | `cortex/claude-archive:dev`  | `17030`          | `synap`, `cortex-ingestion`                         |

Every host port is overridable via `.env` (`*_HOST_PORT` / `CORTEX_*_PORT` variables); the values above are the defaults. The `1700x`/`170xx` range is reserved for Cortex local dev, picked to avoid collisions with the standalone defaults of each Hive service when running outside Cortex. Host port `17011` is deliberately NOT used by any container — it is reserved for the host-side `cortex-adapter-claude` daemon admin `/healthz`.

The original two-container plan (`cortex-api` + a single `cortex-workers`) evolved into one container per worker so each indexer restarts, scales, and reports health independently; the SQLite CAS/metadata/archive stores live on the host under `~/.cortex` (bind-mounted into `cortex-ingestion`, `cortex-api`, and `cortex-claude-archive`) rather than in named volumes.

### Networks

- `cortex-net` — internal bridge; only `cortex-api` ports are exposed to host by default (operator-facing). All inter-service traffic stays on the bridge.

### Volumes

| Volume       | Purpose                                                             |
|--------------|---------------------------------------------------------------------|
| `vec-data`   | Vectorizer `/data` working dir (holds `config.yml`)                 |
| `vec-state`  | Vectorizer XDG data dir — the REAL persistent state (collections, vectors, auth keys, snapshots). Without this mount every recreate wiped every collection (discovered 2026-05-27) |
| `nexus-data` | Nexus WAL + page store (`/app/data` — Nexus 2.x WORKDIR)            |
| `synap-data` | Synap snapshot + AOF                                                |
| `meili-data` | Meilisearch LMDB                                                    |
| `tei-data`   | Reranker (text-embeddings-inference) model cache                    |

Backend state uses **named volumes** so a `docker volume rm` reset is one command. The Cortex-side stores (CAS SQLite, event archive, metadata DB) intentionally do NOT use volumes: they bind-mount the host's `~/.cortex` (override via `CORTEX_HOME_HOST`) so the host-side CLI tools, the adapter daemon, and the containers all see the same files; `cortex-claude-archive` additionally bind-mounts the host's `~/.claude/projects` read-only, and `cortex-graph-worker` persists its consumer offsets under `./.cortex-state/graph`.

### Environment (`.env.example`)

```dotenv
# Host ports (every service's host-facing port is overridable)
CORTEX_API_PORT=17000
VECTORIZER_HOST_PORT=17001
NEXUS_HOST_PORT=17002
SYNAP_HOST_PORT=17003
MEILI_HOST_PORT=17004

# Backend URLs as seen from the HOST (cortex-doctor, host-side CLI).
# In-cluster consumers dial the bridge hostnames + container ports
# instead (e.g. http://synap:15500, vectorizer://vectorizer:15503).
VECTORIZER_URL=http://127.0.0.1:17001
NEXUS_URL=http://127.0.0.1:17002
SYNAP_URL=http://127.0.0.1:17003
MEILI_URL=http://127.0.0.1:17004
CORTEX_EMBEDDER_VECTORIZER_URL=http://127.0.0.1:17001
CORTEX_EMBEDDER_SYNAP_URL=http://127.0.0.1:17003
CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:17004
CORTEX_FULLTEXT_SYNAP_URL=http://127.0.0.1:17003
```

The full knob inventory (classifier mode/model, embedder batch sizes, budgets, retention windows, …) lives in [`.env.example`](../../.env.example) and is type-checked by `cortex-config` (ADR-016); this spec only pins the topology-level variables.

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
synap, meilisearch, vectorizer, nexus, cortex-reranker  (no inter-deps; start in parallel)
        │
        ├──> cortex-ingestion (depends_on: synap)
        │        │
        │        ├──> cortex-classifier-worker (synap + ingestion)
        │        ├──> cortex-claude-archive    (synap + ingestion)
        │        └──> cortex-api               (synap + nexus + meilisearch + vectorizer + ingestion)
        ├──> cortex-embedder-worker  (synap + vectorizer)
        ├──> cortex-fulltext-worker  (synap + meilisearch)
        └──> cortex-graph-worker     (synap + nexus)
```

Each worker depends only on the backends it actually writes to, so a single unhealthy backend degrades one lane instead of blocking the whole stack. Workers additionally carry their own runtime backpressure (pause + half-open probe + sustained-stall restart supervisor, phase28 §1.4) so a backend that turns unhealthy AFTER boot degrades gracefully too.

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

Defined inline in compose for the backends that ship a usable probe. The nexus 2.5 / synap 1.0 images are **distroless** (no shell, no curl/wget), so their healthchecks are exec-form invocations of the server binary's own health flag — a `CMD-SHELL` probe exec-fails on the missing `/bin/sh` and marks the container unhealthy forever:

```yaml
nexus:
  healthcheck:
    test: ["CMD", "/usr/local/bin/nexus-server", "--healthcheck"]
synap:
  healthcheck:
    test: ["CMD", "/usr/local/bin/synap-server", "--health-check"]
meilisearch:
  healthcheck:
    test: ["CMD-SHELL", "wget -q -O /dev/null http://127.0.0.1:7700/health || exit 1"]
```

`vectorizer` and the Cortex services expose HTTP health endpoints (`/health` / `/healthz`) probed by `bin/cortex-doctor` and the `cortex-api` `/v1/health` aggregator rather than compose healthchecks.

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
        vectorizer ................. up
        nexus ...................... healthy
        synap ...................... healthy
        meilisearch ................ healthy
        cortex-reranker ............ up
        cortex-ingestion ........... up
        cortex-classifier-worker ... up
        cortex-embedder-worker ..... up
        cortex-fulltext-worker ..... up
        cortex-graph-worker ........ up
        cortex-claude-archive ...... up
        cortex-api ................. up
[3/3] First-run init: provisioning collections, indexes, streams...
Cortex is up at http://127.0.0.1:17000  →  http://127.0.0.1:17000/dashboard
```

## Acceptance criteria

- [ ] `git clone && cp .env.example .env && bin/cortex-up` brings the stack up on a fresh machine in < 90 s (warm pull).
- [ ] The three compose healthchecks (nexus, synap, meilisearch) pass within the start period, and `bin/cortex-doctor` reports `ok` for all four backends.
- [ ] `bin/cortex-doctor` reports green status for every backend after init.
- [ ] `docker stats` shows total RAM ≤ 10 GB at idle.
- [ ] Killing any backend container triggers automatic restart and Cortex-API recovers without operator intervention.
- [ ] `bin/cortex-reset` completes in < 60 s and produces an empty stack identical to a fresh install.
- [ ] Override `docker-compose.gpu.yml` is exercised in CI on a GPU runner (if available) without breaking non-GPU runs.
- [ ] Windows `.ps1` wrapper achieves parity with bash wrapper (same commands, same exit codes).

## Decisions

1. **Compose, not Kubernetes, for v1.** Kubernetes lives in Phase 5. Compose covers single-node dev + small-team self-host, which is all v1 needs.
2. **Named volumes default; bind mounts opt-in.** Avoid host-permission headaches on first run.
3. **Port range `17000–15099`.** Sequential, easy to remember, away from common dev tools.
4. **Synap exposed via Redis protocol** (it speaks RESP); reuse Redis CLI for diagnostics.
5. **Distroless final images** for Cortex services — small, secure, no shell to be hijacked.
6. **No TLS in v1 dev.** Default bind is `127.0.0.1` and the network is internal. Production override adds Caddy as TLS reverse proxy.

## Open questions

*(none — defaults locked)*

## CI

The local stack participates in three workflows under
`.github/workflows/`:

- [`relevance.yaml`](../../.github/workflows/relevance.yaml) —
  boots the full docker-compose stack and replays the labelled
  query set with a 2pp recall@10 / MRR regression gate.
- [`health-smoke.yml`](../../.github/workflows/health-smoke.yml) —
  smoke-tests the operator-side wrappers (`cortex-up`, `cortex-doctor`,
  `cortex-down`) against a fresh checkout.
- [`retention-canary.yml`](../../.github/workflows/retention-canary.yml) —
  phase9j; runs the in-process retention canary
  (`cargo test -p cortex-retention --test canary`) on every PR
  touching the retention surface plus a nightly schedule. The
  canary uses in-memory backends rather than booting docker so the
  job stays under 15 minutes; the docker-compose-driven end-to-end
  variant lands when phase9k's cron scheduler integrates the
  retention jobs against the live stack.

## References

- Spec 02 — Storage layout (what gets created on first run).
- Spec 04 — Cortex Core (the binary built by `cortex-api` / `cortex-workers`).
- Vectorizer `docker-compose.yml`, Nexus `docker-compose.yml`, Synap `docker-compose.yml` — referenced for canonical service config.
- Meilisearch official Docker docs.
