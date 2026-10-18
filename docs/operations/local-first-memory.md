# Standalone local memory operations

The default runtime is the single `cortex` executable from `cortex-local`.
Capture, history, literal search, graph, MCP, hooks and the embedded web UI
operate on local files. They require no Docker, GPU, remote database, adapter
daemon or provider key. Previously opened legacy clients can still keep old
API/adapter processes alive; this is not a claim that every old process stops
when the new executable is installed.

The [legacy runtime guide](daily-memory-runtime.md) describes the older stack.
Do not mix its install/setup commands with this executable.
The [local evaluation guide](local-memory-evaluation.md) covers reproducible
offline checks, measured retrieval limits and installed-surface verification.

## Build, install and verify

These PowerShell examples use the default home. Substitute an explicit path
consistently for a custom home. On Unix, use the executable without `.exe` and
the corresponding home-directory syntax.

```powershell
cargo build --release --locked -p cortex-local --bin cortex
& .\target\release\cortex.exe build-info
& .\target\release\cortex.exe --home "$env:USERPROFILE\.cortex" install
& "$env:USERPROFILE\.cortex\bin\cortex.exe" --help
& "$env:USERPROFILE\.cortex\bin\cortex.exe" doctor
& "$env:USERPROFILE\.cortex\bin\cortex.exe" --repository Cortex status
```

`build-info` must report `runtime: standalone`, `install_protocol: 1`,
`release: true` and `required_services: []`. It does not open the store.
`install` applies the installation immediately; it has no `--apply` or
`--from` option. It installs the binary and its embedded plugin bundle, then
prints a report, backing up replaced artifacts. It initializes a missing
`local/` store but does not repair an existing one. Use the paths and backup
location printed by the installer.

Home resolution is `--home`, then `CORTEX_HOME`, then `~/.cortex`.
The installed MCP manifests contain the absolute executable path and home;
PATH is optional for those manifests. For a custom home, launch the clients
with the same `CORTEX_HOME` so lifecycle launchers resolve the correct binary.

## Client registration and refresh

The installer publishes the local marketplace at
`<home>/plugins/cortex-standalone`, with `claude/` and `codex/` subdirectories.
Its marketplace name is `hivellm-cortex-local`.

The phase31 bundle is version 0.4.1. It documents all fifteen tools, including
audit inspection; hook definitions are unchanged from 0.4.0. Refresh the client
plugin cache after installing the matching binary/bundle. A version change alone
does not prove that an already running client loaded the updated skills.

The phase31 acceptance installation uses release SHA-256
`3959770ba6cd594664993c210f1741f6bee203b93f5060b86693114d1e09ccaf`.
Its schema-2 upgrade preserves the previous CE680 binary in installation backup
`install-01M56Q4H7H711SGH205ZHRGC8N`. At this checkpoint, Claude/Codex caches are
updated to 0.4.1; restart active clients before validating that they load it.

```powershell
claude plugin marketplace add "$env:USERPROFILE\.cortex\plugins\cortex-standalone"
claude plugin install cortex@hivellm-cortex-local --scope user
codex plugin marketplace add "$env:USERPROFILE\.cortex\plugins\cortex-standalone"
codex plugin add cortex@hivellm-cortex-local
```

Use the client's plugin UI or current `plugin --help` if its registration
surface differs. Disable overlapping old Cortex plugins/hooks before enabling
the standalone lifecycle hooks. Refresh the client cache after each binary or
bundle update, restart the client, and review changed Cortex hooks for trust.
An installed file or a successful direct launcher invocation is not evidence
that a running client loaded that file or executed its hooks.

In Codex, inspect `/hooks` and trust the Cortex entries individually. Do not
trust unrelated plugins as part of this operation. The five lifecycle events
are SessionStart, UserPromptSubmit, PostToolUse, Stop and SubagentStop.
The Windows manifest explicitly invokes
`cmd.exe /d /c call "${PLUGIN_ROOT}\scripts\cortex-hook-launcher.cmd" EVENT`.
This works when Codex launches `commandWindows` through PowerShell as well as
through CMD; a quoted batch path alone is not a PowerShell invocation.

Hooks prepare bounded context or capture observations; they do not read
transcripts, repair the index, start the legacy stack or call Jev. Fail-open
`{}` output does not distinguish every missing-context, launch or store failure.
Check a fresh client session, its hook results and a real scoped MCP read.
Proving model use requires an observed answer using the cited memory; a receipt
or delivered context alone does not prove consumption or general usefulness.

OpenCode's default configuration invokes direct `cortex mcp`. Automatic OpenCode
lifecycle capture is not implemented by that default. The old HTTP lifecycle
adapter remains an explicit legacy opt-in.

## Scope and retrieval

Configure the repository identity in its existing `cortex.toml`:

```toml
[cortex]
id = "Cortex"
```

Identity is exact and case sensitive: `Cortex`, `cortex` and
`legacy:unscoped` are separate scopes. The CLI accepts `--repository` explicitly;
otherwise it reads `[cortex] id` from the working tree, stopping at a repository
boundary. It does not infer identity from the folder name. `status --all` is an
explicit inventory across scopes, not a default cross-repository search.

```powershell
cortex --repository Cortex search "release build" --limit 20
cortex --repository Cortex recent --limit 20
cortex --repository Cortex get RECORD_ID --revision 1
cortex --repository Cortex graph RECORD_ID --depth 1 --limit 50
cortex status --all
cortex mcp --list-tools
```

Search is local SQLite FTS with literal terms combined using AND. It is not
semantic/vector retrieval; long natural-language prompts may have no match.
Try fewer precise terms, inspect provenance, and respect the exact scope.
Raw imported events remain searchable observations but are excluded from
automatic memory recall. Pre-thinking/memory-read context is capped at 6 KiB
and includes identities, revisions, authority and source citations. No result
is automatically promoted to an accepted decision.

The fifteen implemented MCP tools are:

| Tool | Operation |
| --- | --- |
| `cortex_search` | Scoped literal search |
| `cortex_query` | Alias for local search |
| `cortex_get` | Current or exact historical revision |
| `cortex_capture_memory` | Explicit memory with provenance |
| `cortex_revise` | New revision with expected-revision check |
| `cortex_graph` | Bounded typed relationships |
| `cortex_status` | Scoped counts |
| `cortex_pre_thinking` | Bounded cited memory context |
| `cortex_memory_read` | Bounded cited memory context |
| `cortex_classification_enqueue` | Local advisory queue admission |
| `cortex_classification_jobs` | Scoped advisory job inventory |
| `cortex_audit_list` | Scoped audit records; limit 1..100 (default 20), UTC after/before, kind, cursor and include_archived |
| `cortex_audit_get` | Exact scoped audit ID, including archived evidence |
| `cortex_audit_metrics` | Scoped evidence denominators; limit 1..10000 (default 10000), UTC after/before and include_archived |
| `cortex_audit_retain` | Required UTC cutoff; limit 1..100 (default 100), preview unless apply=true; archives visibility without deleting canonical sources |

Use `cortex mcp --list-tools` for current input schemas. Legacy code-index,
law-enforcement and compliance-audit endpoints are not part of this surface.

## Audit evidence and retention

Recall returns six fields: `repository`, `context`, `query_id`, `delivery_id`,
`audit_persisted` and `delivery_consumption`. A successful receipt has a local
query ID, a distinct delivery ID and `audit_persisted=true`. If persistence
fails, context can still be returned with a null delivery ID and false status.
`delivery_consumption` remains `not_observed`. These local IDs are not native
session/turn IDs. Native attribution is stored only when supplied; credentials
are not an attribution mechanism. Receipts bind the rendered citations to exact
memory revisions, content/source hashes and clipping status; selection alone
does not mean a memory appeared in the bounded context.

```powershell
cortex --repository Cortex audit list --limit 20 --kind delivery
cortex --repository Cortex audit get AUDIT_ID
cortex --repository Cortex audit metrics --limit 10000
cortex --repository Cortex audit list --after 2026-10-01T00:00:00Z --before 2026-11-01T00:00:00Z --include-archived
cortex --repository Cortex audit retain --cutoff 2026-10-01T00:00:00Z --limit 100
# After reviewing the preview, explicitly archive visibility:
cortex --repository Cortex audit retain --cutoff 2026-10-01T00:00:00Z --limit 100 --apply
```

List pages contain at most 100 entries (default 20), with an opaque `next_cursor`
for the next request. Metrics sample at most 10,000 matching records, reporting
matching, sampled and omitted counts, truncation, delivery attempts/prepared/
empty/error/unknown counts, action classes and measured latency. UTC windows
and archived visibility restrict their denominator. The citation rate is
prepared deliveries with captured citations divided by sampled prepared
deliveries; a missing denominator yields null. Observations whose delivery lies
outside the sample are reported separately. GUI metrics cover all kinds in the
current repository/archive view, even when its list uses a kind filter.

`prepared_delivery` records preparation, not confirmed model consumption.
`observed_related_action` requires supported linkage, such as matching observed
native session and turn; temporal adjacency alone does not establish use.
`captured_citation` is narrowly verified canonical Stop-event citation
evidence: an exact delivered revision/hash citation is captured in the final
answer. It does not prove that the memory caused a decision, that a tool action
complied with it, or that productivity improved. Self-reported use, ambiguous
matches and missing attribution remain unknown unless independent supported
evidence establishes the link. Method, verification and uncertainty remain
inspectable alongside labels.

Retention previews affected delivery families and references without writing.
Explicit apply appends an immutable archive-control record; it does not delete
audit files, memories, events, imports or backups and does not reclaim disk.
Archived evidence remains available through exact get and include-archived
inspection. No background schedule applies retention.

## Files, backups and recovery

The store lives at `<home>/local/`. Immutable memory revisions are Markdown
records under `records/`; raw events use bounded NDJSON (`.jsonl`) segments
under `segments/`. `index.sqlite3` is a derived search/history/graph projection.
The archive-import ledger uses `archive-import.sqlite3`; daily checkpoints use
`import-state/daily-<hash>.json`. Classification jobs use JSON files under
`classification/jobs/<repository-hash>/`.
Back up the whole home to retain canonical data, ledgers, queue state, installed
artifacts and rollback backups. Quiesce clients and other writers before making
a filesystem backup; copying a live SQLite file alone is not a consistent backup.

Audit records are bounded immutable JSON under `local/audit/`, separate from
memory revisions, event segments and classification jobs. Each record is limited
to 64 KiB. Schema 2 adds their rebuildable SQLite projection. Normal store open
(including `doctor`) upgrades a valid schema-1 index; lifecycle hooks never
perform migration or recovery and fail open until an explicit normal open
prepares the store. Back up canonical audit files with the rest of the home;
index-only backups cannot reconstruct missing receipts. Reindex validates audit
references against exact retained historical revisions and preserves a working
index when canonical input is invalid.

Files are plaintext, not encrypted by Cortex. Redaction is best effort and is
not a complete secret scanner. Historical imports intentionally preserve full
source payloads and provenance; those can contain sensitive content. Protect
the home, source archives and backups using OS permissions and storage controls.

```powershell
cortex reindex
cortex reindex --repair
```

`reindex` rebuilds the derived index from canonical files; it does not recreate
missing canonical data. Use explicit `--repair` for a corrupt index: original
index/sidecars and recovery evidence are preserved under `local/index-backups/`.
Invalid canonical input aborts recovery rather than discarding source evidence.
Do not delete historical archives, `local/`, sidecars or recovery markers to
make an error disappear. Confirm counts and representative historical records
after recovery. Dangling graph links are reported without inventing parents.

There is no automatic retention/pruning policy. Revisions, captured events,
import ledgers, queue history and backups grow on disk. Monitor disk use and
plan backups; an automatic cleanup or archival schedule is not provided here.

## Historical migration

Preserve a source inventory and hashes before importing. Both formats read the
source without modifying it; the destination is the standalone local store.
Use explicit, stable namespaces and mappings. Preview first:

```powershell
cortex import daily "C:\history\metadata.sqlite" --namespace legacy-daily-2026-10 --dry-run
cortex import archive "C:\history\archive" --namespace legacy-archive-2026-10 --unscoped-repository legacy:unscoped --dry-run
```

After reviewing counts, scope mappings and errors, run the same command without
`--dry-run`. Repeat that command to resume. Defaults bound each invocation to
100,000 files and 10,000 records; `--max-files` and `--max-records` adjust those
budgets. `limit_reached` means another pass is needed. `--map ORIGINAL=TARGET`
explicitly changes a scope; `--global-repository TARGET` explicitly maps legacy
global memories. Missing archive repository identity requires
`--unscoped-repository`; historical cwd is evidence, not an inferred scope.
Changing mappings under an existing namespace is rejected. Do not use a new
namespace merely to bypass a resumable error.

Migration opts into a 4 MiB decoded-line ceiling; ordinary live archive readers
retain 1 MiB. The canonical record still has its own 4 MiB bound, while the
searchable body projection is bounded to 1 MiB. Full source payloads are not
silently truncated. Oversized decoder failures stop that file at its safe
checkpoint and remain retryable; they are not counted as decoded rejections.

Inspect `imported`, `duplicates`, `rejected`, `decoder_failed_files`,
`line_limit_files`, `open_frame_files`, `partial_line_files` and
`partial_line_bytes`. `complete=false` can mean a flushed but unfinalized zstd
frame, even when all complete decoded lines were recovered and no partial-line
bytes remain. It does not prove historical producer completeness or corruption.
Do not rewrite sources just to close those frames. Both `quarantine` and
`_quarantine` are excluded; `quarantined_archives` and `quarantine_diagnostics`
separate compressed sources from diagnostics. Preserve both categories.

Unchanged files previously imported with decoded rejections keep their ledger
results; parser updates do not automatically replay them. Investigate those
entries explicitly before planning a new import identity. Verify source hashes
again and compare destination counts, old scoped records and provenance after
the migration. Counts alone do not establish useful recall.

## Optional Jev classification

Enqueueing and listing jobs stay local. Only explicit CLI processing contacts
`https://api.typesafe.ai/v1/systemone`, using `TYPESAFE_API_KEY` and the requested
model. Obtain authorization for the candidate data and destination before that
call; redaction/truncation does not substitute for authorization.

```powershell
cortex --repository Cortex classify enqueue RECORD_ID
cortex --repository Cortex classify jobs
cortex --repository Cortex classify get JOB_KEY
# Only after authorization and secure credential provisioning:
cortex --repository Cortex classify run --max-jobs 1 --seconds 30
```

Do not put credentials in committed files or terminal transcripts. Inspect job
status, usage and typed errors. `classify retry JOB_KEY` explicitly retries a
failed job; completed and stale jobs cannot be retried through this command.
An uncertain prior request can still have incurred provider cost.
Advice and confidence do not change the original accepted memory or its
authority. Offline capture/retrieval do not depend on the provider.

## Local browser interface

```powershell
cortex --repository Cortex serve --port 17200
```

The process binds loopback and prints an authenticated URL with a random token
in its fragment. Open that exact URL; do not publish it, copy it into shared
logs or expose the port through a proxy. The page removes the fragment after
bootstrap, but browser extensions or security integrations may observe the
initial URL before page code runs. During acceptance, this host's Kaspersky
integration observed a fictitious fixture token in the initial URL; that
fixture process was stopped and its token invalidated. This is not a guarantee
of zero browser-originated external traffic. Choose a browser environment
appropriate for the sensitivity of the local records.

Stop the serving process when finished. Its token is process-scoped; stopping
the process closes the port and invalidates that session. The UI supports local
record/history/source inspection, capture/revision and graph navigation. Its
classification queue actions do not implicitly drain jobs to a provider.

The Audit inspection section offers bounded pages, kind/archive filters,
evidence-class metrics, receipt details and linked deliveries. Source/action
buttons read the exact historical revision named by the audit reference, not
the current revision. IDs, hashes, attribution and imported source text render
as text; changing repository invalidates pending audit responses and selection.

## Update and rollback

```powershell
& .\target\release\cortex.exe build-info
cortex update --binary .\target\release\cortex.exe
```

The candidate must identify as standalone install protocol 1. Its own installer
publishes its matching embedded assets. Installation verifies artifact hashes
and preserves replaced artifacts under `<home>/backups/install-.../`. Binary
and complete plugin bundle publication is not one crash-atomic transaction.
After an interruption, inspect the report/backups and rerun the known candidate's
installer to reconcile artifacts; then refresh client caches, restart and renew
changed hook trust. Do not assume an existing client automatically reloads them.

For rollback to a compatible previous standalone release, verify the saved
binary before selecting it:

```powershell
& "C:\backup\cortex.exe" build-info
cortex update --binary "C:\backup\cortex.exe"
```

Use the actual backup path from the installation report. Confirm data-format
compatibility before opening a newer store with an older release. Preserve a
complete backup and rerun scoped acceptance after rollback.

An older multi-process legacy binary does not implement standalone protocol 1
and is not an accepted `update --binary` candidate. Returning to that stack is a
separate selective restore of its binaries, plugin configuration and original
legacy data, following its runbook. Preserve `local/` and all migrated canonical
data; do not restore an entire old home over it. Re-enable only the intended
client hooks to avoid duplicate lifecycle capture.

Measured acceptance and remaining limitations are recorded in
[implementation evidence](../analysis/local-first-core-2026-10/05-implementation-evidence.md).
