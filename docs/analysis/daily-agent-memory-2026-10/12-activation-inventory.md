# Daily activation: existing stack and preserved storage

Read-only inventory for phase34 section 1.11, captured
`2026-10-16T11:03:43.136127+00:00` (08:03 America/Sao_Paulo).
Owner: Codex/Astra. No container was changed while preparing the inventory.
After the coordinator confirmed a consistent backup and explicitly authorized
the transition, this owner executed the reversible container stops recorded
below. Native activation remains owned by the coordinating agent.

## Evidence and ownership

Docker CLI used a new empty temporary config directory and the explicit
`npipe:////./pipe/docker_engine` endpoint. Inspection selected only container
ID/name/image, the four Compose ownership labels, mounts, published ports,
restart policy, running state, health status and network names. `Config.Env`,
container logs, credentials and operator `.env` files were not read. The local
`docker-compose.yml` and `bin/cortex-{up,down,daily}.ps1` source files were read.

Raw selected-field inventory is retained at:

`C:\Users\Bolado\AppData\Local\Temp\cortex-activation-inventory-8fnhw_l0\inventory.json`

SHA-256:
`d752e418146dd501579b7fee647a877c1564f81ce1db679ec9c51cdb6a8605b6`.

There are 41 existing containers, including stopped/created containers.
Exactly 12 have all these ownership properties:

- `com.docker.compose.project=cortex`.
- `com.docker.compose.project.working_dir=E:\HiveLLM\Cortex`.
- `com.docker.compose.project.config_files=E:\HiveLLM\Cortex\docker-compose.yml`.
- Network membership consists only of `cortex-net`.

All 12 were running and healthy with restart policy `unless-stopped` and
`MaximumRetryCount=0`. Every published port below had both `0.0.0.0` and `::`
bindings. A dash means no published host port, not no internal listener.

| Container | ID prefix | Compose service | Image | Host ports |
| --- | --- | --- | --- | --- |
| `cortex-api` | `75646c09ab26` | `cortex-api` | `cortex/api:dev` | 17000 |
| `cortex-ingestion` | `5b9c540e323e` | `cortex-ingestion` | `cortex/ingestion:dev` | 17010 |
| `cortex-claude-archive` | `094ea40575e8` | `cortex-claude-archive` | `cortex/claude-archive:dev` | 17030 |
| `cortex-classifier-worker` | `1983a876c019` | `cortex-classifier-worker` | `cortex/classifier-worker:dev` | 17021 |
| `cortex-embedder-worker` | `18702863acef` | `cortex-embedder-worker` | `cortex/embedder-worker:dev` | 17022 |
| `cortex-fulltext-worker` | `82bda91f6f94` | `cortex-fulltext-worker` | `cortex/fulltext-worker:dev` | 17023 |
| `cortex-graph-worker` | `173171824f36` | `cortex-graph-worker` | `cortex/graph-worker:dev` | 17024 |
| `cortex-vectorizer` | `6c56254647f8` | `vectorizer` | `hivehub/vectorizer:3.6.1-fastembed` | 17001, 17005 |
| `cortex-nexus` | `c0845316b703` | `nexus` | `hivehub/nexus:3.0.0-alpha` | 17002, 17012 |
| `cortex-synap` | `f38b817362bf` | `synap` | `hivehub/synap:1.3.1` | 16379, 17003, 17013 |
| `cortex-meilisearch` | `d1656727fcb9` | `meilisearch` | `getmeili/meilisearch:v1.10` | 17004 |
| `cortex-reranker` | `cf03f45fcb9b` | `cortex-reranker` | `ghcr.io/huggingface/text-embeddings-inference:89-1.8` | - |

Port 17000 is published by `cortex-api`; stopping that exact container is the
minimal Docker action needed to let the new native API bind 127.0.0.1:17000.
The adapter is deliberately native and is not a Compose service.

## Preserved data and native paths

| Data | Current mount/location | Native access and preservation |
| --- | --- | --- |
| Main Cortex home | Host `C:\Users\Bolado\.cortex` mounted RW at `/var/lib/cortex` in API, ingestion and archive watcher | Reuse this host directory; no copy or relocation needed |
| Main metadata SQLite | Expected `/var/lib/cortex/metadata.sqlite`; host `C:\Users\Bolado\.cortex\metadata.sqlite` | Native stat/open succeeded; 700,416 bytes; first 16 bytes match SQLite header; no row contents read |
| Historical archive | Compose config `/var/lib/cortex/archive`; host `C:\Users\Bolado\.cortex\archive` | Directory exists and is natively accessible; no recursive walk or decode performed |
| Graph worker offsets | Host `E:\HiveLLM\Cortex\.cortex-state\graph` mounted RW at `/var/lib/cortex-graph` | Its `metadata.sqlite` is 253,952 bytes with valid SQLite header; preserve separately from main metadata |
| Native Claude files | Host `C:\Users\Bolado\.claude` mounted RW at `/root/.claude` in old API | Preserve untouched; old API can reach native memory files through this mount |
| Claude transcripts | Host `C:\Users\Bolado\.claude\projects` mounted RO at `/data/claude-projects/projects` in watcher | Preserve untouched; stopping watcher does not delete transcript history |
| Workspace files | Host `E:\HiveLLM` mounted RO at `/workspaces` in API | No data move; native paths differ from container paths |

The main metadata `-wal` and `-shm` companion files were absent at the native
filesystem snapshot. This is not a quiescence guarantee for later activation:
the archive writer was still running and the data can change. Main metadata
placement follows the checked-in Compose `CORTEX_HOME=/var/lib/cortex` and the
observed bind mount. Effective container environment overrides were deliberately
not inspected, so unexpected runtime overrides are not excluded by this
inventory alone. Native activation should resolve the intended explicit paths.

These six named volumes remain attached to their existing stopped containers
under the proposed plan:

| Volume | Container destination | Purpose |
| --- | --- | --- |
| `cortex_vec-data` | Vectorizer `/data` | Config/data mount |
| `cortex_vec-state` | Vectorizer `/.local/share/vectorizer` | Actual persistent Vectorizer collections/state |
| `cortex_nexus-data` | Nexus `/app/data` | Graph store |
| `cortex_synap-data` | Synap `/data` | Bus/store state |
| `cortex_meili-data` | Meilisearch `/meili_data` | Search indexes |
| `cortex_tei-data` | Reranker `/data` | Model/cache data |

All six are Docker `local` volumes whose selected live mount source is
`/var/lib/docker/volumes/<volume-name>/_data`. That is a Docker VM location,
not a native Windows folder to rewrite or relocate. The daily profile needs
none of these stores to capture/read durable local memory. Do not remove
volumes, recreate containers, or use `compose down -v` for this activation.

Authentication requires separate verification. In current source,
`api_keys_db_path` uses explicit `dashboard.api_keys_db` or the process home
directory; it does not derive its default from `ingestion.home`/`CORTEX_HOME`.
The checked-in Compose file does not specify `CORTEX_API_KEYS_DB`, and the live
API does not mount `/root/.cortex`. Therefore the shared metadata bind does not
prove that old Docker credentials and a native key store are the same. Root
must pin/verify the intended native authentication path independently. No key
store was opened for this inventory and no credential continuity is claimed.

## Dependencies that remain untouched

No non-Cortex container in the 41-container inventory belongs to `cortex-net`
or has a mount source exactly matching a Cortex mount source. The four
database/bus containers and reranker above are dedicated to this Compose
project by the observed labels/network/mount topology; they are not services
borrowed from the other Compose projects. An elevated TCP snapshot found no
established native-host connection to published backend ports 16379, 17001-17005,
17012 or 17013. This point-in-time observation cannot exclude transient or
external clients using published ports.

The following are outside the proposed stop scope:

- Docker Desktop, its backend/build processes, Docker engine and WSL VM.
  They support unrelated projects and must remain running.
- All five `buildx_buildkit_*` containers: `vec3710`, `vectorizer-builder0`,
  `fluxum-builder0`, `nexus-builder0`, `synap-builder0`. They have no Cortex
  Compose label, use `bridge`, and own separate build-cache volumes. Names
  referring to backend projects do not make them Cortex runtime services.
- The `ar-v3-dashboard` project: dashboard, consolidator, OTLP ingestor,
  Prometheus, gateway, exporter, PostgreSQL and its existing helper containers.
- `ar-v3-database` from `ar-database-sync`; the independent `ar-test-pg-*`
  PostgreSQL/Timescale containers; all `ar-carga-*`, `ar-portoes-*`, `ar-ensaio`,
  `ar-v3-cov*`, `ar-legacy-restore`, `ar-v3-redis-test` and `enc-*` containers.
- Native MCP servers and any processes not identified/owned by the native daily
  launcher. No image-name-wide termination is proposed.

## Approved transition and rollback

The exact IDs/names and Compose ownership were revalidated before every stop.
The coordinator confirmed native configuration readiness. A minimal transition can
stop only `cortex-api`; the remaining 11 bulk services continue consuming
resources. For the intended daily-only runtime, the concrete project-scoped
approved transition stops all 12 listed containers while retaining containers,
networks, restart policies, bind mounts and named volumes.

The executed stop phases followed producer/consumer/store dependencies, using
an explicit empty Docker CLI config and the Docker Desktop named-pipe endpoint
as in the inventory. The table names were resolved to the exact revalidated
full container IDs before each `docker stop --time 30` command.

| Phase | Exact containers passed to `docker stop --time 30` | Reason |
| --- | --- | --- |
| Producers and legacy API | `cortex-claude-archive cortex-api` | Halt archive watching/scheduled API work; release host port 17000 |
| Bulk consumers | `cortex-classifier-worker cortex-embedder-worker cortex-fulltext-worker cortex-graph-worker` | Stop consumers before their backing stores |
| Archive ingestion | `cortex-ingestion` | Stop archive/bus publisher after watcher and workers |
| Optional index stores/model | `cortex-reranker cortex-vectorizer cortex-nexus cortex-meilisearch` | Release optional model/index resource use without dropping state |
| Bus | `cortex-synap` | Stop bus after all known Cortex producers/consumers |

`docker stop` is reversible with `docker start`; it preserves the stopped
container and its attached data. With `unless-stopped`, an explicit manual stop
keeps these containers stopped across Docker daemon restarts until explicitly
started again. No restart-policy mutation is necessary. Do not use a global
`docker stop $(docker ps ...)`, container deletion, volume pruning or stack
reset script.

Rollback should first stop only the native processes whose recorded PID,
executable path and creation time match the daily launcher's ownership state.
Then start the existing backing stores (`cortex-synap`, `cortex-vectorizer`,
`cortex-nexus`, `cortex-meilisearch`, `cortex-reranker`), wait for their health,
start `cortex-ingestion`, then the four workers and archive watcher, and finally
the existing `cortex-api` after port 17000 is free. Starting existing containers
retains their previous deployment configuration and avoids config drift from
recreation. Verify health before moving to each dependent group.

`bin/cortex-up.ps1` runs `docker compose up -d --wait` plus initialization and
can bring the full stack back; it is not the daily activation command.
`bin/cortex-down.ps1` runs `docker compose down`, which removes containers and
network, so the proposed reversible stop does not use it. The new
`bin/cortex-daily.ps1` creates only native API/adapter children, forces the daily
profile, refuses occupied ports/pipes, and records ownership for later stop.
It does not stop Docker or migrate existing storage automatically. Root owns
the explicit configuration/authentication selection and activation checks.

## Execution record

The coordinator confirmed the backup
`.cortex-state/daily-runtime/metadata.before-activation-verified.sqlite` with
SQLite integrity check `ok` and 22 tables before authorizing these stops.
The stop sequence ran from `2026-10-16T11:11:32.598815+00:00` to
`2026-10-16T11:16:03.161442+00:00`. Every container's full ID, name, project,
service, project directory, Compose file and restart policy matched the saved
inventory immediately before its stop. All commands returned success; each
target was then re-inspected as `exited` with unchanged restart policy.

| Container | Finished at UTC | Exit code | OOM killed |
| --- | --- | ---: | --- |
| `cortex-claude-archive` | 11:11:38.369915 | 0 | false |
| `cortex-api` | 11:12:10.120402 | 137 | false |
| `cortex-classifier-worker` | 11:12:40.732738 | 137 | false |
| `cortex-embedder-worker` | 11:13:11.367587 | 137 | false |
| `cortex-fulltext-worker` | 11:13:41.898104 | 137 | false |
| `cortex-graph-worker` | 11:14:12.513517 | 137 | false |
| `cortex-ingestion` | 11:14:43.201474 | 137 | false |
| `cortex-reranker` | 11:14:44.890349 | 0 | false |
| `cortex-vectorizer` | 11:14:57.787315 | 0 | false |
| `cortex-nexus` | 11:15:28.479842 | 137 | false |
| `cortex-meilisearch` | 11:15:29.087520 | 143 | false |
| `cortex-synap` | 11:15:59.706551 | 137 | false |

Exit 137 following the 30-second grace interval is consistent with Docker's
SIGKILL fallback; exit 143 reflects SIGTERM. This was not uniformly graceful
shutdown. No container reported OOM kill. The coordinator was informed so
post-stop storage verification can accompany native activation. The API's
successful stop/released port was communicated immediately while the remaining
stops continued; this owner did not start a native process.

At the end, all 29 unrelated containers had exactly their recorded pre-stop
running/exited/created states. All six named Cortex volumes were present by
explicit `docker volume inspect` of names only. No container, network, volume,
bind-mounted file or image was removed or recreated; no restart policy was
changed. Stopping containers naturally stops their workload; persistence
is preserved, but no byte-for-byte database equality is claimed while the old
services were still running before their stop.

Raw selected-field execution evidence:

- `C:\Users\Bolado\AppData\Local\Temp\cortex-daily-activation-stop-qpqfp5a_\stop-results.json`
  contains each ID/result/time, all 29 before/after unrelated states and the six
  preserved volume names. SHA-256:
  `a138dce23d2d7a1a6b72475f95cd855e3afc1123279ffeeffe4cf663049cb42b`.
- `C:\Users\Bolado\AppData\Local\Temp\cortex-daily-activation-stop-qpqfp5a_\exit-verification.json`
  records the subsequent exit-code/OOM/restart-count verification at
  `2026-10-16T11:16:29.864510+00:00`.

The reversible commands remain `docker start` on these same preserved
containers, following the dependency/health order above after the native
launcher has stopped its owned processes and released port 17000. No rollback
was run as part of this inventory/stop task.

## Active native runtime: operational follow-up

At `2026-10-16T11:39:38+00:00`, a read-only operational check of the
coordinator-activated runtime found no concrete runtime blocker. The launcher
`status` command validated the recorded process identities as running:

| Role | PID | Installed executable |
| --- | ---: | --- |
| API | 100928 | `E:\HiveLLM\Cortex\.cortex-state\daily-runtime\bin\cortex-api.exe` |
| Adapter | 82260 | `E:\HiveLLM\Cortex\.cortex-state\daily-runtime\bin\cortex-adapter-claude.exe` |

The four installed API/adapter/MCP/hook binaries matched both the saved
manifest and current build files. Hashes and the subsequent 60-second
installed-use resource observation are recorded in
[analysis10](10-resource-acceptance.md). A later selected-field Docker check
confirmed all 12 Cortex containers still `exited`; no service was restarted
for the operational review.

The active private runtime config has SHA-256
`0eacb4764ecb3ff66fb8648edbf3e8e031d651ffb07a92ff021f81fe2e7ed49c`.
Only non-secret selected fields were reported: profile `daily`, access control
enabled, `deny_on_missing_principal=true`, default level 0, existing native
`C:/Users/Bolado/.cortex/metadata.sqlite`, existing archive root, and archive
incremental processing disabled. It explicitly selects
`E:/HiveLLM/Cortex/.cortex-state/daily-runtime/api_keys.sqlite`, addressing the
old-container/native-default credential-path ambiguity identified above. A
local API token is present; its value was never printed. No provider credentials
were found in this TOML; inherited provider configuration was not inspected.

Observed HTTP results:

- Authenticated `GET /v1/status`: 200; PID 100928; `runtime_profile=daily`;
  local SQLite memory `ready`.
- API `GET /healthz` and adapter `GET /healthz`: 200, both explicitly report
  `extras.runtime_profile=daily`.
- Adapter health reports `bulk_capture_enabled=false`.
- Bulk indexers, monitoring, scheduled maintenance and query enrichment report
  `disabled_by_profile`; archive is disabled with no poll and zero snapshot hits.
- Missing-token and invalid-token `POST /v1/memory/query` probes both return
  401. These probes did not capture or read an operator memory.

Status deliberately retains `delivery_consumption=not_observed` and marks the
adapter `unprobed` from the API's own capability perspective. The separate
adapter health and actual installed client/model evidence do not change those
honest API status semantics into a general consumption guarantee.

README now directs routine memory users to the operations guide. That guide
documents the installed explicit start/status/stop paths, manual restart after
reboot, reconnecting clients so their MCP processes load current registrations,
the Git Bash named-pipe spelling, exact repository identity `Cortex`, a concrete
first `cortex_memory_read` request, and explicit `classify=true` opt-in for
external advice. No OS scheduler/autostart was added. The generic new-install
example appearing before the host-specific activation commands was flagged to
the guide owner for clearer labeling, so a returning user does not accidentally
start different config/state paths. This owner did not edit client settings,
the guide, launcher source or another agent's files during this review.
