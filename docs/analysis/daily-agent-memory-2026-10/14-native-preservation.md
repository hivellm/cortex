# Native-memory preservation and persistent-state inventory

## Scope

Task §1.11. This check reads native file bytes only to compute SHA-256 and reads
only scheduler state, table/column names and aggregate row counts from Cortex's
existing SQLite. No native Markdown text, memory body, repository identity,
credential or complete process command line is emitted. No live data, schedule,
installed configuration or binary is changed by this check.

## Historical baseline and its limit

[Evidence 08](08-implementation-evidence.md) records that all 535 top-level
Claude native-memory Markdown hashes matched before/after the §1.2 scheduler
disable. It does not name a retained hash manifest. A bounded search of the
existing task/evidence references and Cortex/phase34 temporary artifacts did not
find that original manifest. The recorded historical result remains evidence
from that operation; a new matching file count alone cannot prove those earlier
hashes still match today.

The current run therefore creates a fresh, reusable pre-activation manifest.
It keys each entry by SHA-256 of the relative native path and stores SHA-256 of
the bytes, keeping native filenames and content out of the artifact. It covers
`~/.claude/projects/*/memory/*.md` only, matching the historical top-level scope.
Nested Markdown and non-Markdown files are outside that claim.

## Observed pre-activation state

Captured `2026-10-16T11:04:26.540466+00:00`:

| Check | Observed result |
| --- | --- |
| Native Markdown files | 535 |
| Ordered manifest SHA-256 | `3e04d033abebcb5425d2386ee5a1d37d1beaa8f9686acc8b71f602c13bead115` |
| Existing metadata DB | `C:\Users\Bolado\.cortex\metadata.sqlite` |
| DB file size | 700,416 bytes |
| Existing tables | 22 |
| Retired scheduler row | Present; `retention.memory_consolidate`, `enabled=0` |
| Last recorded outcome | `success`, `2026-10-09T04:19:50.752596229+00:00` |
| Failure streak | 0 |
| Running retention sweeps | 0 |
| Daily-memory tables | None present before activation |
| Source DB/sidecar metadata during read | Unchanged; no WAL/SHM was created |

Manifest artifact:
`C:\Users\Bolado\AppData\Local\Temp\cortex-native-preservation-aec64bdc29c64b7f98940ab485994b1c.json`.

A separate process inventory found no Cortex retention/consolidator process.
Five existing `cortex-mcp-server.exe` processes had no `memory-consolidate` token
in their command lines; they were left untouched. This is a point-in-time
observation, not a scheduler lock or a guarantee against future manual commands.

## Persistent data activation must reuse

The workspace has no `.cortex` directory. The current user-level Cortex data
directory contains:

| Existing path under `C:\Users\Bolado\.cortex` | Observation and preservation requirement |
| --- | --- |
| `metadata.sqlite` | Authoritative existing scheduler/history DB above. Daily API must reuse and migrate this file, rather than silently choose a fresh alternative. |
| `cas.sqlite` | Existing 20,480-byte store; retain. No content read. |
| `overflow.wal` | Existing 15,459,154-byte legacy WAL; retain without daily-profile replay. No content read. |
| `archive/` | Existing archival data; retain. No archive decoded or content sampled. |
| `adapter.toml`, logs and cursor files | Existing operator/runtime state; retain. No secret values read. |
| `cortex-hook.exe` | Existing native hook executable; activation/replacement belongs to the operator workflow, outside this read-only check. |

The inspected workspace TOML did not override the selected metadata/profile
fields, and `CORTEX_HOME`, `CORTEX_METADATA_DB`, `CORTEX_CONFIG_FILE` were absent
from this checker process. These observations do not override an explicit
configuration chosen by the activation coordinator. `api_keys.sqlite` was not
present in the pre-activation home-directory inventory; no credential database
was opened or extracted by this work.

## Protection mechanism verified in source

`crates/cortex-workers/src/retention/scheduler.rs` seeds
`retention.memory_consolidate` disabled. `AUTO_RUN_RETIRED_JOBS` names it;
`retire_automatic_legacy_jobs` changes only `enabled` on existing rows, preserving
schedule, command and run history. `seed_defaults` performs that retirement on
every seed. `select_auto_runnable_jobs` retires it again, then excludes retired
names from the returned runnable set, including a racing re-enable.

`crates/cortex-api/src/retention_daemon.rs::list_due` calls the same selection
helper. This matters because API's daemon executes commands directly rather
than going through the scheduler's ordinary tick. The explicit manual
`memory-consolidate` / `schedule run-now` paths remain operator actions; this
protection is not a blanket filesystem permission boundary. Evidence08 records
the earlier scheduler/daemon regression gates; this agent ran no Cargo command.

## Reusable read-only check

`scripts/daily_memory_preservation_check.py` defaults to read-only inspection.
`--output` writes a new artifact only and refuses overwrite. `--baseline`
compares added, missing and changed native hashes without printing their names.
It never invokes installed clients, tools, schedulers or providers.

```powershell
python scripts/daily_memory_preservation_check.py --self-test
python scripts/daily_memory_preservation_check.py --baseline `
  "$env:TEMP/cortex-native-preservation-aec64bdc29c64b7f98940ab485994b1c.json" `
  --output "$env:TEMP/cortex-native-preservation-after-activation.json"
```

SQLite inspection uses `mode=ro&immutable=1` and `query_only=ON`. A nonempty WAL
causes an explicit unavailable result: immutable reads would omit committed WAL
changes, while normal SQLite readers can create/update shared-memory sidecars.
The checker chooses neither a stale read nor a live-file mutation. A concurrent
DB/WAL/SHM metadata change also marks the result untrusted. Native-file hashes
still remain useful independently of SQLite availability. A missing baseline or
an unavailable SQLite snapshot must not be reported as a successful historical
or scheduler verification.

Offline checks passed for path-name privacy, hash comparison, changed-file
detection, disabled scheduler selection, zero running sweeps, absence of daily
tables, unchanged SQLite/sidecar metadata and nonempty-WAL rejection. Synthetic
fixtures remain retained in temporary storage. Post-activation results must be
compared against the named fresh manifest before claiming activation preserved
all 535 current native files.

## First active launch: native-file comparison

At `2026-10-16T11:18:04.565234+00:00`, after the coordinator's first installed
API/adapter launch, all 535 current native files matched the fresh pre-activation
manifest: **0 changed, 0 added, 0 missing**. Ordered manifest SHA-256 remained
`3e04d033abebcb5425d2386ee5a1d37d1beaa8f9686acc8b71f602c13bead115`.

Retained comparison:
`C:\Users\Bolado\AppData\Local\Temp\cortex-native-preservation-first-launch-f4714d51581b416590fa378b2a7f0d2d.json`.

The checker found a 135,992-byte live SQLite WAL and a 32,768-byte SHM, so it
explicitly refused immutable SQLite queries. It did not create/update sidecars
or claim a fresh scheduler/schema count from the main DB alone. Separately, the
activation coordinator reported its live integrity check passed with 35 total
tables and 13 daily-memory tables, using API/adapter PIDs `60228`/`62224`. Those
live DB results are coordinator evidence, not results of this immutable reader.
This snapshot proves native-file preservation through the first launch; later
runtime updates and the real installed-hook model session require their own
recorded checks.

## Final installed-workflow preservation

At `2026-10-16T11:39:54.638994+00:00`, after installed binaries/configuration
updates, the named-pipe command correction and the successful public synthetic
native Claude session, all 535 files still matched the same pre-activation
manifest: **0 changed, 0 added, 0 missing**. Manifest SHA-256 remained
`3e04d033abebcb5425d2386ee5a1d37d1beaa8f9686acc8b71f602c13bead115`.

Artifact:
`C:\Users\Bolado\AppData\Local\Temp\cortex-native-preservation-final-e5c1e2ee0b98437a81715bb8573aaf9d.json`.
The immutable reader again declared live SQLite counts unavailable because a
568,592-byte WAL existed; native hash comparison passed independently.

The installation owner intentionally corrected seven Claude hook pipe arguments
to `//./pipe/cortex-adapter-claude`; that authorized configuration change is
separate from native-memory preservation. The successful native session itself
left user settings unchanged before/after, with SHA-256
`9acdff9314fa39a6a402254a5ebe8d138b9a19faba09be7d919ee6e405bff48f`.
[Evidence 15](15-installed-workflow.md) records actual installed-hook delivery,
model use of the public canaries and the preserved earlier failure.
[Evidence 22](22-authorized-live-validation.md) records the subsequent grants and
completed live verification for the named summaries. After the final installed
model run, all 535 native hashes still matched the original baseline, with zero
changed, added or missing files. The grants exclude unrelated memories, including
the later Windows pipe lesson; exact summary copies used in the isolated Claude
test do not establish general historical-memory migration.
