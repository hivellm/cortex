# Daily memory resource and HTTP acceptance

Measured 2026-10-16, phase34 section 1.10. Owner: Codex/Astra. This evidence
covers isolated daily API/adapter resources and authenticated memory HTTP
latency. Cross-model recall and the 600 ms end-to-end hook/pre-thinking gate
have separate evidence; these HTTP measurements do not substitute for them.

## Result and scope

- 210/210 measured capture/read requests returned the expected durable receipt
  or exact seeded memory. No failed request was removed or retried.
- Two isolated daily processes completed a 60.026-second idle observation.
  Their CPU and I/O counters did not increase at the counters' observed
  resolution. Sampled peak working sets were 20.000 MiB for the API and
  13.676 MiB for the adapter.
- Twenty-one API starts, including twenty restarts, became healthy in
  241.8-275.1 ms. The seeded memory survived the cold-read restarts.
- The sentinel accepted zero connections to optional backends. The fixture
  database contained zero cron jobs and zero `archive_index_*` tables.
- All 22 fixture-owned PIDs (21 successive APIs and one adapter) were absent
  from the Windows process inventory after cleanup. Existing services remained
  running; no operator data, configuration, or credential store was edited.

These are observations from one host and one debug build. There is no measured
before/after resource reduction, no release-build performance claim, and no
CPU/RSS pass threshold in the accepted design. The successful result means the
functional assertions and measurement collection passed, not that arbitrary
performance budgets were introduced or proven.

## Reproduction and artifacts

Run from the repository root with current prebuilt binaries and an agreed
quiet window:

```powershell
python scripts/daily_memory_resource_probe.py --run
```

The script defaults to help without `--run`. It uses Python's standard library,
explicit loopback ports, a unique named pipe, a new temporary directory, a new
SQLite database, a newly issued API key and a unique synthetic repository. Child
environments retain only Windows executable bootstrap variables and explicit
fixture settings. Provider credentials are not inherited. No model calls occur.
The TOML runtime profile is `full`; the child environment overrides it to
`daily`, while configured optional backend URLs point to one TCP sentinel.
Access control is enabled with `deny_on_missing_principal=true`; a request
without credentials must return 401 before measurement starts. The adapter
receives the same temporary credential through its child environment.

The valid run lasted from `2026-10-16T10:54:15.043121+00:00` through
`2026-10-16T10:56:14.245238+00:00` (07:54-07:56 America/Sao_Paulo).
The host reported 32 logical CPUs. Binaries came from `target/debug`:

| Artifact | SHA-256 |
| --- | --- |
| `cortex-api.exe` | `89ec8e0cbe9a90fa94576a54c957e5492f727f4a2cd79090a882a9a0986e3e79` |
| `cortex-adapter-claude.exe` | `39551553a0601019b32ca6f27997d31bc999346af5b3345b5e1b33ddf66bb631` |
| Raw `report.json` | `800975eaf5fb60bf2d71eaba9401969e67a09a55ce1da63391b54717ac2818c9` |
| Later `baseline-after.json` | `2c6c7990591a8cf06e8ff94f377f77e84c811b2d1c8068a7c2a8de1e57d151da` |

Local retained evidence:

- `C:\Users\Bolado\AppData\Local\Temp\cortex-resource-acceptance-cb2asxf3\report.json`:
  raw one-second process samples, host counters, every request latency/status,
  cache-hit flags, startup PIDs/times, binary hashes and functional checks.
- The same directory retains synthetic runtime/adapter TOML, logs and databases.
  The key's cleartext is neither written to the report nor printed.
- `C:\Users\Bolado\AppData\Local\Temp\cortex-resource-baseline-v1nxxw0k\baseline-after.json`:
  the later elevated read-only Docker/Windows snapshot described below.
- An earlier preflight artifact at
  `C:\Users\Bolado\AppData\Local\Temp\cortex-resource-acceptance-0fxeducj\report.json`
  records an aborted fixture check: dashboard authentication alone does not
  require credentials on the memory route. The synthetic config was corrected
  to enable access control before the valid run. No timed samples came from
  that preflight; both children were stopped.

The raw report is retained unchanged. After the run, the script's Docker helper
was tightened to use an explicit empty CLI config directory and named-pipe
endpoint, avoiding automatic lookup of the operator's Docker config. No new
idle/latency run was needed for that baseline-only change.

## Idle methodology and results

The coordinator paused Cargo builds/tests and model calls for the complete
measurement. Existing host/container workloads were not stopped. A successful
startup/health/authentication preflight preceded the interval; no HTTP requests
were sent to either fixture child during idle. Windows process counters were
sampled approximately once per second for 60 seconds; the counter sample span
was 59.999858 seconds and total collector duration was 60.026084 seconds.

`GetProcessTimes` supplies kernel/user CPU seconds. CPU percent is
`100 * delta_cpu_seconds / counter_interval_seconds`, relative to one core.
`GetProcessMemoryInfo` supplies working set, lifetime peak working set, and
private bytes. `GetProcessIoCounters` supplies operation and byte counters;
these include network/device/file I/O, so they are not disk-only metrics.
`CreateToolhelp32Snapshot` supplies PID, executable name, and thread count.
Process creation times detect PID reuse. `GetSystemTimes` supplies host CPU
busy time normalized across logical CPUs.

| Process | PID | CPU seconds delta | One-core CPU | RSS start/end MiB | Sampled/lifetime peak RSS MiB | Private end MiB | Threads start/end |
| --- | ---: | ---: | ---: | --- | --- | ---: | --- |
| Daily API | 90452 | 0.000000 | 0.000% | 20.000 / 19.938 | 20.000 / 20.020 | 6.813 | 37 / 33 |
| Daily adapter | 65764 | 0.000000 | 0.000% | 13.676 / 13.629 | 13.676 / 13.676 | 3.742 | 36 / 33 |

Read, write and other byte deltas were all zero for both children; their read,
write and other operation-count deltas were also zero. Zero means no observable
counter increment over this interval, not proof that no instructions ran or
that future idle intervals are cost-free. Lifetime peaks include startup and
preflight; sampled peaks cover the observation window only. Working set is not
total system memory pressure or unique physical memory.

Host CPU averaged 22.309% across all cores during the same window. Five existing
native `cortex-mcp-server.exe` processes were visible (PIDs 58588, 63656, 33580,
41900, 57904); each had 34 threads and zero observed CPU/I/O increments. Their
profiles were not inspected. Docker Desktop and `vmmemWSL` counter reads were
denied by the sandbox during this interval, and both in-run Docker snapshots
failed. The script preserves these unavailable results rather than substituting
zeros. The later elevated snapshot is separate evidence.

## Authenticated HTTP latency

Every measured operation used a valid temporary API key and a fresh loopback
TCP connection. Timings run from HTTP request initiation through complete body
read and JSON parsing. They include transport/authentication/storage work and
exclude fixture startup, thread-barrier wait and explicit inter-wave pacing.
Sequential requests were paced by at least 50 ms; four-request waves by at
least 200 ms, below the documented sustained rate limit. This is a latency
probe, not a saturation/maximum-throughput benchmark.

Warm cases execute one successful operation before sampling. Cold cases are
the first authenticated operation or four-request wave after a fresh API
process starts; the existing fixture database is retained and the Windows file
cache is not flushed. Read-cold cases therefore test process/auth/query-cache
restart, not physically cold disks. One request in a cold wave may publish a
cache entry before its peers finish: 4/20 cold-wave reads reported cache hits;
0/5 cold-sequential reads did. All 80 measured warm reads reported cache hits.

Percentiles use nearest rank and include failure latency if any. Five cold
sequential samples cannot establish a stable tail estimate; their p95 equals
the sample maximum. The four-concurrent cold cases comprise five waves each.

| Endpoint | State | Concurrent | Requests | Failures | p50 ms | p95 ms | Max ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `POST /v1/ingest` | Warm | 1 | 40 | 0 | 316.5 | 365.0 | 395.5 |
| `POST /v1/ingest` | Warm | 4 | 40 | 0 | 410.5 | 431.3 | 447.1 |
| `POST /v1/memory/query` | Warm | 1 | 40 | 0 | 318.8 | 393.6 | 404.2 |
| `POST /v1/memory/query` | Warm | 4 | 40 | 0 | 403.6 | 417.6 | 421.0 |
| `POST /v1/ingest` | Cold | 1 | 5 | 0 | 292.9 | 297.9 | 297.9 |
| `POST /v1/ingest` | Cold | 4 | 20 | 0 | 303.5 | 334.8 | 335.1 |
| `POST /v1/memory/query` | Cold | 1 | 5 | 0 | 296.4 | 303.2 | 303.2 |
| `POST /v1/memory/query` | Cold | 4 | 20 | 0 | 296.8 | 323.8 | 325.0 |

Capture assertions require HTTP 202, the exact generated memory ID and
`durability=local_committed`. Read assertions require HTTP 200, exact
`resource-seed` identity, its unique token in rendered context and at most
1,536 rendered UTF-8 bytes. Setup/warmup requests are outside these 210 samples.
The adapter is alive during all measurements, but this table measures direct
API HTTP, not named-pipe hooks or actual model consumption. All timings below
600 ms here do not prove the separate 600 ms end-to-end hook requirement.

## Existing-stack read-only snapshot

After the quiet measurement ended, sandbox restrictions were bypassed with an
approved read-only invocation. Docker CLI used an empty temporary config and
`npipe:////./pipe/docker_engine`; no `docker inspect`, `Config.Env`, secrets or
operator key store was read. The saved snapshot started at
`2026-10-16T10:58:22.887344+00:00`, after other work was allowed to resume.

Selected exact `docker stats --no-stream` fields:

| Existing container | CPU snapshot | Memory usage | PIDs/tasks | Cumulative block read/write |
| --- | ---: | --- | ---: | --- |
| `cortex-api` | 96.29% | 10.07 GiB | 1006 | 27.1 MB / 4.96 MB |
| `cortex-vectorizer` | 0.06% | 3.955 GiB | 64 | 330 MB / 913 MB |
| `cortex-reranker` | 0.82% | 3.874 GiB | 38 | 2.4 GB / 0 B |
| `cortex-claude-archive` | 5.24% | 1.743 GiB | 17 | 4 MB / 0 B |
| `cortex-meilisearch` | 0.11% | 315.1 MiB | 36 | 358 MB / 596 MB |
| `cortex-nexus` | 3.12% | 135.6 MiB | 34 | 178 MB / 19.3 MB |

The saved raw snapshot also includes the four workers, ingestion, Synap and
other running containers. Docker's PIDs field includes kernel tasks/threads;
1006 is not a claim of 1006 independent API processes. Docker CPU is its own
short sampling interval, not the daily probe's 60-second average. Docker memory
accounting is not identical to Windows working set; cumulative block I/O is
not an interval delta. Seven Docker Desktop/backend/build Windows process
snapshots became readable after escalation; `vmmemWSL` PID 90152 still returned
WinError 5. Its memory/CPU remains unknown.

This confirms a substantial existing stack was running on the same host. Its
builds, loaded corpus, uptime, profile and workload were not normalized against
the new small daily fixture. Neither current-vs-historical savings nor a
controlled full-vs-daily percentage reduction can be inferred. No existing
container was restarted, reconfigured or stopped for this task.

## Follow-up: installed runtime observation

After section 1.11 activation, the installed native runtime was checked without
restarting it. This is separate from the earlier isolated fixture. Its API and
adapter reuse the operator's selected metadata/configuration, and the API/MCP
binaries include the subsequently validated explicit classification opt-in
change. All four installed hashes matched both `binary-manifest.json` and the
corresponding current `target/debug` files:

| Installed binary | SHA-256 |
| --- | --- |
| `cortex-api.exe` | `04370056107bd613f7f9928aaef0fd312f864b1afea64478a91ad8c7ac41745a` |
| `cortex-adapter-claude.exe` | `39551553a0601019b32ca6f27997d31bc999346af5b3345b5e1b33ddf66bb631` |
| `cortex-mcp-server.exe` | `3fdded9678907c31d03a7e35e69d312521d1b9210342569b8e593231c14c1daa` |
| `cortex-hook.exe` | `50250c95ee5477fc122c6fa94bc51b44a508599f71a0b3fb5abf2e909e067973` |

Observation ran from `2026-10-16T11:40:17.834992+00:00` through
`2026-10-16T11:41:17.886436+00:00`. Counter sample span was 60.000128 seconds;
collector duration was 60.028515 seconds. The collector sent no HTTP requests
during this interval and made no model calls. The coordinator reported that
the installed Claude canary had completed at 11:39:32 UTC. Ordinary clients
and host work were not suspended, and observed I/O is nonzero, so this is an
installed-use observation rather than a controlled idle benchmark.

| Process | PID | CPU seconds delta | One-core CPU | RSS start/end MiB | Sampled/lifetime peak RSS MiB | Private end MiB | Threads start/end |
| --- | ---: | ---: | ---: | --- | --- | ---: | --- |
| Installed daily API | 100928 | 0.296875 | 0.49479% | 25.250 / 25.273 | 25.363 / 44.344 | 7.734 | 33 / 33 |
| Installed daily adapter | 82260 | 0.015625 | 0.02604% | 19.738 / 19.758 | 19.758 / 20.074 | 5.781 | 34 / 34 |

API I/O deltas were 94,308 read bytes, 70,040 write bytes and 338 other bytes
(24/34/37 operations respectively). Adapter deltas were 59,100 read bytes,
3,337 write bytes and 5,040 other bytes (67/49/340 operations). API sampled
thread maximum was 34; adapter maximum was 34. As above, these are native
process I/O counters, not disk-only accounting. Host CPU averaged 6.095% across
all cores. The launcher's state file remained byte-for-byte unchanged throughout
the observation; no PID was replaced.

Raw samples are retained at
`C:\Users\Bolado\AppData\Local\Temp\cortex-installed-observation-j9aqecer\observation.json`,
SHA-256 `57a6c62a87282101c56267c9ab2dc34a90a282e492150c9382555b5024a18720`.
This confirms bounded observed local resource use in the installed session.
Different workloads, binary revisions and sampling windows prevent a causal
percentage-savings comparison with the earlier Docker snapshot or fixture.
The operational/authentication checks accompanying this observation are in
[analysis12](12-activation-inventory.md).
