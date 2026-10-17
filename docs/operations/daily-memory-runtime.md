# Daily-memory runtime on Windows

## Self-contained install (phase35)

Cortex installs as a Claude Code or Codex plugin. No client file holds a Cortex
path, token or environment block, and no repository `.mcp.json` is needed.

1. **Binaries.** Build in release mode, then copy the four daily binaries into
   `~/.cortex/bin` and add that directory to `PATH`:
   `cortex-ops install-binaries --from target/release`, then the same command with
   `--apply`. The installer verifies SHA-256, moves a running executable aside
   before replacing it, and writes `install-manifest.json`.
   `cortex-ops doctor-install` reports missing, shadowed or stale binaries, and a
   double install.
2. **Configuration.** `cortex-ops setup --from <runtime.toml>`, then the same
   command with `--apply`. It writes `~/.cortex/config.toml` and moves
   `dashboard.api_token` into `~/.cortex/api-token`, which is readable only by
   the user (`icacls` on Windows, mode 0600 elsewhere). Every changed file is
   backed up first. Binaries select one runtime file: `CORTEX_CONFIG_FILE`,
   otherwise `<CORTEX_HOME or ~/.cortex>/config.toml`, otherwise `./cortex.toml`.
   The MCP server and adapter read the token file when no token is configured.
3. **Plugin.** Run `claude plugin marketplace add <repo>`, then
   `claude plugin install cortex@hivellm-cortex`. For Codex, run
   `codex plugin marketplace add <repo>`, install `cortex`, and trust its hooks
   in `/hooks`.
4. **Lifecycle.** The first synchronous hook that finds the adapter pipe absent
   starts `cortex-api` and `cortex-adapter-claude daemon` from the directory
   holding `cortex-hook`. A spawn lock and a 60 s backoff coordinate concurrent
   hooks; logs and state live in `~/.cortex/lifecycle/`. The hook shows a notice
   only when the start fails, once per session. Nothing is registered with the
   OS, so after a reboot the first session brings the runtime back (ADR 040).
   Set `CORTEX_HOOK_AUTOSTART=0` to keep starting manual. Hooks that pass an
   explicit `--pipe` never start a runtime.
5. **Migration.** After the plugin is installed and enabled, run
   `python scripts/activate_daily_memory_clients.py --write-manifest m.json`,
   review the plan, then run it with `--apply --expected-manifest m.json`. This
   removes the old Cortex hooks and MCP entries from `~/.claude/settings.json`,
   `~/.claude.json`, `~/.codex/config.toml` and `~/.codex/hooks.json`, plus a
   `hivellm-cortex` marketplace registration whose directory no longer exists.
   It writes verified backups and a record under `~/.cortex/migrations/`, which
   `--rollback <record>` restores byte-for-byte. No other file is touched.

The sections below describe the launcher-based runtime that preceded the
plugin. Use the launcher when you need explicit process control.

**This host runs the plugin install since 2026-10-17.** Binaries live in
`~/.cortex/bin` (first on the user PATH), configuration in `~/.cortex/config.toml`,
and the first Claude Code session starts the runtime
([acceptance](../analysis/daily-agent-memory-2026-10/24-plugin-install-acceptance.md)).
The pinned launcher state in [Local activation](#local-activation-on-2026-10-16)
remains as the rollback path. The generic examples below describe a new installation.

The daily profile runs native `cortex-api` and the Claude adapter. SQLite memory
capture, recall and feedback do not require Docker, GPU, Synap or bulk indexers.
MCP servers belong to each client and start over stdio when that client connects.
Optional archive and full-stack search capabilities must report their actual
disabled or unavailable state; API readiness does not prove model-visible recall.
The daily profile is resolved before optional clients and background jobs are
created. It preserves configured URLs, credentials and data locations; preserved
configuration does not mean a backend is connected. Full-profile capability
status uses `configured_unverified` until actual observations establish more.

Daily HTTP routes are limited to this core surface, with their normal auth and
method requirements:

| Surface | Paths |
| --- | --- |
| Basic health and metrics | `/healthz`, `/metrics`, `/v1/status`, `/v1/health` |
| Local capture, recall and query | `/v1/ingest`, `/v1/memory/query`, `/v1/query` |
| Durable feedback | `/v1/pre-thinking/feedback`, `/v1/feedback/list` |
| Local active-work view | `/v1/dashboard/active-work` |
| Access control and audit | `/v1/acl/*`, `/v1/audit/*` |

Other optional endpoints return HTTP 503 with `reason=disabled_by_profile` before
their handlers run. This includes bulk search, extended health and dashboard
routes apart from active-work. `/v1/status` reports `runtime_profile` at the top
level; basic health places it under `extras.runtime_profile`. Daily-memory
capability readiness checks SQLite open/read only and explicitly reports
`delivery_consumption=not_observed`. It does not prove a client consumed context.

Archive decoding stays off by default. Existing explicit
`ingestion.archive_incremental_enabled=true` or
`CORTEX_ARCHIVE_INCREMENTAL_ENABLED=true` can separately opt into the bounded
incremental archive projection. The launcher does not enable it. Adapter raw
archive publication remains disabled in daily mode, leaving its existing
overflow WAL untouched.

Use current validated binaries explicitly. Installed binaries on PATH can be
older than this checkout. Building and installing are separate steps; this
launcher does neither.

Build the four native executables in release mode before a new installation:

```powershell
cargo build --release --all-features -j4 `
  --bin cortex-api --bin cortex-adapter-claude --bin cortex-hook --bin cortex-mcp-server
```

The launcher defaults to `target/release`. Use optimized release builds for daily
operation: the workspace debug profile uses `opt-level=0`, and installed debug
Argon2 authentication took about 400-500 ms per request, approaching the 550 ms
memory HTTP deadline. Consecutive hook activity exposed intermittent recall
timeouts. The validated release runtime passed 40 consecutive hooks (20
SessionStart and 20 UserPromptSubmit) without an unavailable-memory notice;
pipeline, HTTP and shim bounds remain unchanged. See
[release validation](../analysis/daily-agent-memory-2026-10/23-release-runtime-validation.md).

```powershell
.\bin\cortex-daily.ps1 start -BinaryDirectory E:\HiveLLM\Cortex\target\release
.\bin\cortex-daily.ps1 status
.\bin\cortex-daily.ps1 stop
```

The launcher sets `CORTEX_RUNTIME_PROFILE=daily` only for its child processes.
Existing credentials remain inherited; the launcher never reads credential
stores or `.env` files and never prints credential values. `-ConfigFile` selects
an existing Cortex TOML file when custom metadata paths or authentication are
needed. Otherwise the launcher preserves an existing `CORTEX_CONFIG_FILE`, then
uses the calling directory's `cortex.toml` when present. If neither exists, a generated
empty config uses ordinary Cortex data defaults and the daily profile. Existing
metadata and full-stack volumes remain in place.
Children retain the calling directory, so configured relative data paths keep
their ordinary meaning. Run from the same directory as your existing native
configuration when it uses relative paths.

Default API address is `127.0.0.1:17000`; adapter admin port is `17011` and the
Claude named pipe is `\\.\pipe\cortex-adapter-claude`. Occupied ports or pipe
cause failure. No existing listener is adopted or stopped. Alternative ports,
pipe and `-StateDirectory` require matching client/hook configuration. Keep the
same state directory for subsequent status and stop commands.

State and separate process logs live under `%USERPROFILE%\.cortex\daily-runtime`
by default. Log names include a per-start timestamp; `state.json` records that
prefix. A lock prevents concurrent launcher operations. Stop checks each
recorded PID, executable path and process creation time before stopping it;
stale or reused PIDs are skipped. Windows process termination is immediate, so
wait for in-flight capture requests to finish before stopping. Only launcher-owned
processes are affected. State, logs and existing data files are retained.

For an MCP client, configure the validated binary with arguments:

```text
E:\HiveLLM\Cortex\target\release\cortex-mcp-server.exe --api-url http://127.0.0.1:17000 serve
```

Pass `CORTEX_RUNTIME_PROFILE=daily` to that client-owned server as well. Preserve
its existing authentication and avoid duplicate registrations. Reconnect clients
after changing their executable configuration. `cortex_memory_read` with the
actual repository and task verifies bounded cited recall; lifecycle injection
requires a separate real hook/model-visible check.

Daily MCP advertises 12 tools: `cortex_capture_memory`, `cortex_memory_read`,
`cortex_pre_thinking`, `cortex_query`, `cortex_status`, `cortex_audit`,
`cortex_active_work`, `cortex_feedback_signals`, `cortex_feedback_record`,
`cortex_acl_whoami`, `cortex_acl_grant`, and `cortex_capabilities`. Use
`cortex_memory_read` for explicit local decisions and lessons. `tools/list`,
`cortex_capabilities`, and dispatch share this registry; direct calls to omitted
bulk tools return JSON-RPC `-32601` before any HTTP request. Full retains all 42
tools. `print-tool-descriptors` also follows the configured runtime profile.

```powershell
.\bin\cortex-daily.ps1 -SelfTest
```

This check validates process identity guards, PID reuse rejection, argument
quoting and occupied-port rejection without launching Cortex. Startup requires
both health endpoints to confirm the `daily` profile; an old binary's generic
HTTP 200 is insufficient. Windows `Get-NetTCPConnection` also verifies that the
expected child PID owns each listening port, including a bind race after the
initial conflict check. Start failure cleans up processes it verified and
recorded during that attempt. If a just-spawned child's identity cannot be read,
cleanup uses its retained Windows process handle instead of an unchecked PID.
Inspect logs when readiness fails.

The full stack remains optional and separate. Existing `bin/cortex-up.ps1` starts
the complete Compose stack and bootstrap, including heavyweight services; it is
not the daily launcher. The daily launcher never stops existing Compose services;
those may continue using resources until a separately authorized activation step.
Do not use reset, volume removal, or the legacy adapter
Task Scheduler installer to select the daily profile. Activation and client
installation require the separately validated operator rollout.

An isolated Windows PowerShell 5.1 smoke on 2026-10-16 verified the rebuilt API
and adapter with temporary home, metadata, API-key DB, workspace, unique ports
and pipe. Start returned while both children remained available; status and stop
worked across separate launcher invocations. Both health endpoints confirmed
daily mode, SQLite was ready, a pre-existing synthetic overflow WAL remained
byte-for-byte unchanged, and all owned processes/listeners disappeared after
stop. An occupied-port attempt preserved the unrelated listener and launched no
services. That isolated smoke changed no installed runtime or user data.

## Local activation on 2026-10-16

The operator-authorized rollout stopped the 12 containers belonging exclusively
to the Cortex Compose project. All 29 unrelated containers retained their state;
all six named Cortex volumes remain present. Some old services required Docker's
30-second stop timeout and exited 137; this was not a uniformly graceful stop.
The existing metadata database passed SQLite integrity validation after the
transition. Inventory, exact stop results and dependency-ordered rollback are in
[analysis12](../analysis/daily-agent-memory-2026-10/12-activation-inventory.md).

Native daily API and adapter now use validated binary copies under
`E:/HiveLLM/Cortex/.cortex-state/daily-runtime/bin`. They reuse
`C:/Users/Bolado/.cortex/metadata.sqlite`; existing archive, CAS, graph data and
adapter overflow WAL are preserved. A consistent pre-activation SQLite backup
is retained as `.cortex-state/daily-runtime/metadata.before-activation-verified.sqlite`.
The earlier zero-byte backup attempt is an incomplete artifact, not a recovery
copy. Native-memory preservation has separate hash evidence in
[analysis14](../analysis/daily-agent-memory-2026-10/14-native-preservation.md).

Private `runtime.toml` in that state directory contains the local Cortex token
and explicit API-key database path. It is ignored by Git and has a restricted
Windows ACL. Client registrations carry the config path rather than the token.
The API listens on loopback only; unauthenticated memory requests return 401.
Authentication for Claude, Codex and other model providers is unchanged.

On this host, use the same explicit paths for subsequent manual operations:

```powershell
.\bin\cortex-daily.ps1 status -StateDirectory E:/HiveLLM/Cortex/.cortex-state/daily-runtime/processes
.\bin\cortex-daily.ps1 stop -StateDirectory E:/HiveLLM/Cortex/.cortex-state/daily-runtime/processes
.\bin\cortex-daily.ps1 start -BinaryDirectory E:/HiveLLM/Cortex/.cortex-state/daily-runtime/bin -StateDirectory E:/HiveLLM/Cortex/.cortex-state/daily-runtime/processes -ConfigFile E:/HiveLLM/Cortex/.cortex-state/daily-runtime/runtime.toml
```

No OS autostart or scheduler was created. Processes survive the launching shell;
after reboot, start the daily launcher manually. `bin/cortex-up.ps1` starts the
optional full stack and should not be used to start daily memory.

The earlier 2026-10-17 debug rollout updated the four pinned executables from the
exact candidate that passed formatting, workspace type-checking, Clippy and the complete suite
(3,955 passed, zero failed, nine ignored). Native Claude/Fable then recalled two
authorized saved decisions through the installed hooks; three original Jev
candidates classified successfully and reused their cache without changing memory
records. A 60-second observation of that debug runtime measured 97.367 MiB
combined final working set for API/adapter with background hook activity, not
exclusive idle.
See [final live evidence](../analysis/daily-agent-memory-2026-10/22-authorized-live-validation.md).
The subsequent release rollout and consecutive-hook validation are recorded in
[release validation](../analysis/daily-agent-memory-2026-10/23-release-runtime-validation.md).
Current release pins passed 40/40 consecutive hooks and repeated native Claude
recall. The API/adapter combined working set was 32.031 MiB during a 60-second
observation with 23 background hook frames; combined CPU was 0.1562% of one core.
These values exclude client-owned MCPs and other host services.

Claude's existing user hooks and MCP registration now point at validated copies.
Windows Claude hook commands use `--pipe "//./pipe/cortex-adapter-claude"`.
Git Bash removes one leading backslash from the equivalent quoted Windows pipe
argument; the forward-slash form reaches the same named pipe intact. MCP
environment values and the daemon retain the canonical Windows pipe path.
Codex's MCP registration is updated; its existing hook definitions and trust
hashes are preserved, so this rollout claims explicit MCP support for Codex.
Kimi's MCP entry is configured, but its native model remains unavailable. Grok
has no native CLI on the inspected PATH. Reconnect existing clients to load
changed registrations; an already-running MCP process retains its old binary
and environment. [Analysis13](../analysis/daily-agent-memory-2026-10/13-client-activation.md)
separates registration, transport checks and actual model execution.

## Daily agent workflow

Before substantive work, read relevant memory using the exact repository ID
(`Cortex` for this checkout), current task and applicable branch/path. Claude
hooks can supply this context automatically. MCP-only clients explicitly call
`cortex_pre_thinking` or `cortex_memory_read`. Save only accepted decisions or
verified lessons; uncertain ideas remain proposed. Use a named predecessor and
the observed revision for corrections. The shared workflow lives in
`AGENTS.override.md`, which Claude also imports.

Direct `cortex_memory_read` queries accept at most 4 KiB and 32 distinct terms;
use concise task text. Ordinary hook prompts can be longer: pre-thinking projects
them into that bounded query. A repository used by native hooks needs an absolute
working directory, a Git root and an explicit `[cortex] id` in `cortex.toml` that
the daemon's OS user can read. Sandbox-owned private temporary directories may
not be readable by that user. Missing explicit identity fails closed rather than
reading memory from a same-named directory.

Automatic daily lookup requires a valid explicit `[cortex] id` in `cortex.toml`
at the nearest Git root. Choose the exact repository ID used by capture/read;
give unrelated repositories distinct IDs. Without that identity, hooks and
`cortex_pre_thinking` skip daily lookup with `memory_outcome=scope_identity_required`
and hooks show the deduplicated memory-unavailable notice. Legacy optional
full-stack search keeps its directory-name fallback. Directory names alone cannot
isolate daily memories: two unrelated checkouts can both be named `app`.
Hook origin must also supply an absolute CWD. Missing or relative hook paths
return `scope_unresolved` without memory, optional-search or active-work lookup.
For MCP `cortex_pre_thinking`, use an absolute `cwd` to authorize daily lookup;
relative paths retain only the legacy optional-search behavior.

```toml
[cortex]
id = "Exact/Repository"
```

For a first check in a fresh MCP client, call `cortex_memory_read` with:

```json
{"repo":"Cortex","query":"daily memory","limit":5,"budget_bytes":1536}
```

The installed project has accepted decisions about the daily runtime and task
priority, plus a lesson distinguishing prepared delivery from actual use. A
successful read returns cited memory IDs and a delivery ID. Missing tools mean
the client may still have its old MCP process; reconnect before diagnosing
storage. For other repositories, use their exact configured repository ID and
capture their own decisions; Cortex memories do not cross that scope by default.

Capture is local by default. `classify=true` explicitly requests external Jev
classification of that authorized candidate after local commit. Other stored
memories are not sent as neighbors. The TypeSafe key can remain configured while
ordinary captures stay local. Advice has bounded calls/time/cost and cannot
change the captured status, content, scope, rules or supersession. A classifier
failure does not roll back a committed memory.

To add classification consent later, replay the exact original capture with its
same retry key and `classify=true`. An unrelated later capture does not invalidate
an unchanged candidate. The original commit receipt stays unchanged; advice uses
a fresh snapshot and still rejects a superseded, modified or stale candidate.
Keep the same retry key after a timeout or lost acknowledgement: SQLite work
may have committed even when the client did not receive a success response.
