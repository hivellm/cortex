# Daily client activation plan

Date: 2026-10-16. Task `phase34_daily-agent-memory-recovery`, §1.11.
This document records the reviewed change plan and observed activation checks.
Root applied the guarded five-file plan. Loaded-client and manual transport
checks below remain distinct from model consumption.

## Current selected inventory

Only Cortex MCP fields, Cortex hook definitions, plugin enablement and token
presence were reported. Provider credential stores were not opened. No 9router
key was extracted. Native model/provider settings remain authoritative.

| Client/scope | Current Cortex entry | Proposed action |
| --- | --- | --- |
| Claude user `C:/Users/Bolado/.claude.json` | `mcpServers.cortex.command = cortex-mcp-server`, args `serve` | Pin validated executable and daily runtime environment under same server name |
| Claude project `E:/HiveLLM/Cortex/.mcp.json` | Same bare command and API17000; overlaps user declaration | Align existing entry with user entry; do not add another server name |
| Claude user `C:/Users/Bolado/.claude/settings.json` | Seven bare `cortex-hook` lifecycle commands; SessionStart has `--fire-forget` | Pin hook executable and explicit Windows pipe; make SessionStart synchronous |
| Codex user `C:/Users/Bolado/.codex/config.toml` | `[mcp_servers.cortex]` uses bare command; URL17000 and Unix-style socket | Pin same validated MCP executable and daily environment; preserve all other TOML bytes |
| Codex user `C:/Users/Bolado/.codex/hooks.json` | Six bare Cortex hook commands | Preserve file and existing trust hashes; use explicit MCP recall contract |
| Kimi user `C:/Users/Bolado/.kimi-code/mcp.json` | Missing | Create Cortex-only MCP registration; preserve native `config.toml` and login |

Claude's user settings explicitly disable `cortex@hivellm-cortex`. Keep it
disabled: enabling packaged hooks alongside global hooks could duplicate
lifecycle firing. Project Claude settings contain no Cortex hooks. No project
Codex configuration or project Kimi MCP file exists in this snapshot.

Existing Cortex MCP environments contain API URL and adapter socket, no
`CORTEX_API_TOKEN`. Process `CORTEX_API_TOKEN` and root `cortex.toml`
`dashboard.api_token` are absent in this snapshot. The final ignored runtime
configuration supplies private local Cortex authentication; this activation
does not transfer provider authentication or print local API credentials.

## Exact reviewed fields

The selected local installation directory is
`E:/HiveLLM/Cortex/.cortex-state/daily-runtime/bin`. Root creates validated
copies of the binaries there before activation. This directory is isolated
from `.cargo/bin`; future rebuilds of `target/debug` or `target/release` cannot replace its files.
The expected-state manifest binds the two client binaries by SHA-256.

Every existing/new `cortex` MCP entry receives:

```json
{
  "command": "E:/HiveLLM/Cortex/.cortex-state/daily-runtime/bin/cortex-mcp-server.exe",
  "args": ["serve"],
  "env": {
    "CORTEX_RUNTIME_PROFILE": "daily",
    "CORTEX_CONFIG_FILE": "E:/HiveLLM/Cortex/.cortex-state/daily-runtime/runtime.toml",
    "CORTEX_API_URL": "http://127.0.0.1:17000",
    "CORTEX_ADAPTER_PIPE": "\\\\.\\pipe\\cortex-adapter-claude",
    "CORTEX_ADAPTER_SOCK": "\\\\.\\pipe\\cortex-adapter-claude"
  }
}
```

Existing non-Cortex environment names and settings remain. No token is added
to MCP environment tables: server reads private `dashboard.api_token` from
the selected runtime TOML. Adapter inherits the same runtime configuration.
Keep that file out of version control and evidence output.

Claude hook command pattern:

```text
"E:/HiveLLM/Cortex/.cortex-state/daily-runtime/bin/cortex-hook.exe" SessionStart --pipe "//./pipe/cortex-adapter-claude"
```

Apply corresponding event names to `UserPromptSubmit`, `PreToolUse`,
`PostToolUse`, `Stop`, `SubagentStop`, `Notification`. First three are
synchronous. Last four retain `--fire-forget`. Existing matchers, unrelated
hooks, permissions, environment and plugin enablement remain unchanged.
Native hook total transport bound remains 1,500 ms; local prethinking's
600 ms contract is a separate inner bound.

Use the forward-slash Windows pipe spelling in hook commands only. Git Bash
interprets backslashes inside double quotes before native argv: the previous
canonical `\\.\pipe\...` argument arrived with one leading backslash and
failed transport. MCP environment and daemon configuration retain their
canonical backslash spelling because they do not traverse that shell parser.

Codex hook commands and `[hooks.state]` trusted hashes remain unchanged.
Changing hook definitions requires native user trust review; writing hashes
would fabricate approval. Current bare hooks therefore remain an older
installation path, not evidence of activated daily lifecycle injection.
Codex activation is explicitly through its MCP server. Kimi registration is
configuration only until a real fresh client run observes discovery and use.

The [official OpenAI MCP documentation](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)
places shared desktop/CLI/IDE MCP configuration in user `config.toml`, with
trusted project scope supported. [Official plugin documentation](https://developers.openai.com/plugins/build/plugins)
also describes native trust review for non-managed plugin hooks. Installed
`codex mcp add --help` confirms stdio `COMMAND` and per-server `--env`; local
CLI startup still reports home-directory limitations in this sandbox. Native
client configuration must be checked outside that restriction before labeling
its installed setup broken.

## Guarded helper

[`activate_daily_memory_clients.py`](../../../scripts/activate_daily_memory_clients.py)
defaults to dry-run. It changes only the five files in the inventory action
column: three MCP JSON files, Claude hook settings and Codex MCP TOML.
It never edits Codex hooks, trust state, provider/login files or Kimi native
configuration. JSON preserves unrelated values; TOML preserves unrelated bytes
and verifies the entire parsed result against the narrow intended change.

Unexpected Cortex hook syntax, duplicates, enabled Cortex plugin, missing
existing registration files, non-stdio servers or missing pinned binaries
stop the helper. Runtime TOML must explicitly select `daily`. Rerunning an
already applied plan produces no file writes.

Example review command, after root creates private runtime files:

```powershell
python scripts/activate_daily_memory_clients.py `
  --bin-dir .cortex-state/daily-runtime/bin `
  --runtime-config .cortex-state/daily-runtime/runtime.toml `
  --write-manifest .cortex-state/daily-runtime/client-activation.expected.json
```

The manifest contains paths and before/after hashes, binary hashes and runtime
hash only. Exclusive creation prevents silently replacing an earlier review.
Apply requires `--apply --expected-manifest <that-file>` with identical
arguments and exact file/binary/runtime state. Helper prechecks every client
file, rechecks immediately before each write, creates verified timestamped
adjacent backups for existing files and verifies each written hash. Backups
can contain private configuration and stay beside native files, not in repo
evidence. Apply is sequential, not a multi-file transaction; a write failure
reports its file and retains prior backups. Existing files are written in
place to preserve native file ACLs. No deletion or automatic rollback occurs.

## Verification and evidence boundary

Offline in-memory checks passed for syntax, unrelated TOML byte preservation,
unchanged private sentinel settings, synchronous Claude lifecycle and
idempotence. No Cargo command or external configuration mutation was run by
the helper author. Root owns activation authorization and final gates.

Root then applied the expected-state manifest: five file updates, adjacent
backups for existing files. Subsequent dry-run exited 0 with equal before/after
SHA-256 for all five files, proving no additional edits were proposed.

| Observed check | Result | Evidence limit |
| --- | --- | --- |
| Native `claude mcp get cortex` | Exit0; user scope, Connected, stdio, final pinned command and args `serve` | Loaded registration/connection; no model call |
| Native `codex mcp get cortex --json` | Sandbox failed home lookup; same invocation outside sandbox exit0, enabled=true, final pinned command, args `serve`, five expected Cortex environment names | Native loaded configuration; no model call |
| Native `kimi doctor` | Exit0; installed help has no MCP list/get subcommand | Configuration validation; native MCP discovery remains unverified |
| Manual stdio using each installed Claude/Codex/Kimi entry | All three initialize successfully, enumerate exactly12 daily tools, return `cortex_capabilities` with `isError=false` | Real server transport using installed config; not an invocation by those clients/models |
| Codex trust preservation | Entire parsed `[hooks]` subtree equals adjacent pre-activation config backup; all six hook commands equal initial selected inventory | No initial whole-file digest was captured for `hooks.json`, so historical byte-hash equality is not claimed |

Current unchanged-command Codex `hooks.json` SHA-256:
`fcf5a22e60eea325962ec4bb6b54539b97f3a91eb7660444dc145d840dd70036`.
No trust hashes were written or regenerated. Selected verification output
excluded complete settings, environment values containing credentials and
provider auth. API was ready before transport checks; final seeded memory
read remains pending in this snapshot.

Before seeding project memory, automatic approval review rejected three
planned captures because the then-current production handler would classify
their internal contents through external TypeSafe/Jev without payload-specific
authorization. Those captures did not run. The safer implementation adds
`classify`, a boolean defaulting to false, to HTTP capture and MCP schema.
Missing/false commits locally and returns advisory status `unavailable` with
reason `not_requested`; it never invokes classification even with provider
credentials configured. Only explicit `classify=true` may send that candidate
externally. Other stored memories are never sent as neighbors: candidate opt-in
does not establish their consent. This affects advice
only: captured content, receipt, retry identity and correction rules remain
local and unchanged. Final native-client checks were paused until this path
passed validation and updated pinned API/MCP binaries were active; the later
observations below supersede that pause.

The local-only/candidate-only revision passed API/MCP `cargo check` and
`cargo clippy -- -D warnings` with `--all-targets -j 4`. API advice tests:
21 passed, one explicit live test ignored; MCP memory contract tests: eight
passed. API capture/restart integration: seven passed; cache integration:
six passed. The first security suite run used parallel test execution and
retained one failure: `production_builder_wires_separate_key_store_and_respects_local_auth_mode`
received 503 `daily_memory_unavailable` instead of 200 for an enabled-auth
query. The complete four-test security suite then passed using this session's
established `--test-threads=1` setting. No production deadlines were relaxed;
the exact unavailable stage under parallel load was not established. This
parallel failure remains part of evidence rather than being filtered away.
The explicit TCP/mock test proves missing/false classification makes zero
provider calls with configured credentials, opt-in makes one call, retries
reuse the receipt/cache, and a distinct same-repo local-only canary is absent
from an empty-neighbor provider payload. No real provider was called.
Final API/MCP binary build also exited 0 (`cargo build` with the two named
binary targets and `-j 4`, 51.71 seconds). Root refreshed both private pins
and restarted its owned runtime before the final project-memory check.

### Final configured-transport verification

After root's API_READY v2, root reported three committed local-only captures
with advice `not_requested` and zero provider attempts before/after. Manual
stdio probes then used each installed client entry, without copying credentials
or calling a model. Query: repository `Cortex`, task `daily memory`, limit5,
budget1,536 bytes. Every configured transport returned exactly:

| Memory ID | Version |
| --- | --- |
| `adr039-daily-runtime` | 1 |
| `daily-value-priority` | 1 |
| `prepared-is-not-consumed` | 1 |

All three returned 1,309 UTF-8 context bytes, revision3 and a distinct delivery
ID equal to the response query ID. Read-only SQLite verification matched the
returned memory list exactly, confirmed byte length and stage `prepared`.

| Installed configuration used | Prepared receipt ID |
| --- | --- |
| Claude user `.claude.json` | `01M527H8JP466N7CPTEXAD8MV8` |
| Codex user `.codex/config.toml` | `01M527H91GGC8A7XMKT2KKQMZE` |
| Kimi user `.kimi-code/mcp.json` | `01M527H9CFJV7NJJX8ZZPT2FZX` |

Each actual pinned server enumerated12 tools and advertised boolean
`classify` defaultfalse with candidate-only disclosure. Its SHA-256 was
`3fdded9678907c31d03a7e35e69d312521d1b9210342569b8e593231c14c1daa`.
After all reads, SQLite contained zero advice attempts, cache rows and counters.
No provider call or model invocation occurred in these probes. In particular,
this is not proof of native Kimi MCP discovery or model-visible context.

Private sanitized evidence: `.cortex-state/daily-runtime/client-transport-verification-v3.json`.
An initial observer attempt used nonexistent SQLite column `delivery_id`
instead of schema column `id`; those observer errors remain in separate
`client-transport-verification-v2.json`. Production was not changed to resolve
that observer error. All probe children were terminated and awaited.

### Installed Claude hook pipe correction

Independent hook diagnosis found the shell/argv mismatch above: the old
backslash command returned no context, while a forward-slash pipe argument
returned 987 context bytes containing two synthetic canaries. The activation
helper now changes only Windows hook pipe arguments and accepts an upgrade
only from the exact prior pinned command, original known bare command, or
already corrected command. Unknown command flags and duplicate hooks still
stop activation. Runtime/MCP pipe values and Codex hooks/trust remain untouched.

The follow-up expected-state manifest proposed exactly one file change:
`C:/Users/Bolado/.claude/settings.json`. Authorized application succeeded with
an adjacent verified backup. New settings SHA-256:
`9acdff9314fa39a6a402254a5ebe8d138b9a19faba09be7d919ee6e405bff48f`.
An offline native Python argv probe launched through installed Git Bash
reproduced the one-backslash failure and preserved `//./pipe/...` exactly.
Guard checks covered exact pinned upgrade, unchanged unrelated settings,
rejection of extra flags and idempotence. Subsequent dry-run proposed zero
changes across all five client files. Comparing full parsed Claude settings
against its prior backup after reversing only these pipe strings found no
other changes. No Rust/Cargo/provider/model work was performed for this fix.
Subsequent independent model/hook verification using these installed settings
passed for public synthetic canaries; [analysis15](15-installed-workflow.md)
records the actual context and model output. Transport/argv checks alone do not
prove model consumption. The separately authorized test subsequently passed:
[analysis22](22-authorized-live-validation.md) records native Claude's recall of
exact copies of the two named summaries through real installed hooks in an
isolated scope. Those copies do not establish general memory migration.

After activation, open fresh clients. Use selective parsed output from
`claude mcp get cortex` and `codex mcp get cortex --json`, excluding credential
values, to confirm loaded command and environment names. Kimi installed help
offers no `mcp` subcommand; use `/mcp` in a fresh trusted session, or retain
actual MCP initialize/tools-list frames. Never treat an absent provider/model
as successful model consumption.

Daily MCP must enumerate exactly 12 tools. Explicit ordinary-task workflow:
before edits, call `cortex_memory_read` with actual repo/task/branch/paths;
after an authorized durable decision/correction/lesson, call
`cortex_capture_memory` and retain committed receipt. These instructions
request explicit tool use; they are not automatic lifecycle injection.
Observe calls/results and exact receipt IDs before claiming transport success;
observe model context/task action separately before claiming memory use.

Claude synchronous hook delivery and public-canary use on the installed daily
adapter passed in [analysis15](15-installed-workflow.md). Synthetic fixture
acceptance and manual transport probes remain separate evidence. The operator
has granted the named real-memory payload permissions;
[analysis22](22-authorized-live-validation.md) records their scope and completed
native Claude recall and original-candidate Jev checks. Kimi/Grok native CLI use
remains unproven by these Claude and manual-transport results.
