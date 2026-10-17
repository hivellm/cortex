# Installed client contracts for daily memory

Date: 2026-10-16. Canonical task: `phase34_daily-agent-memory-recovery`,
item 1.7. This is a capability and independent-review snapshot, not an
activation record or a declaration that item 1.7 is complete.

## Evidence boundary

The [daily-memory specification](../../../.rulebook/tasks/phase34_daily-agent-memory-recovery/specs/daily-memory/spec.md)
requires real installed-harness tests. The upstream comparison in
[analysis03](03-ai-memory-comparison.md) and prior Claude coordination in
[analysis04](04-claude-coordination.md) establish useful hypotheses, not current
installed-client behavior. No installation, global configuration change, OS
schedule, Rust edit or Cargo invocation was performed for this document.
Only relevant configuration fields were inspected; credentials and complete
global configuration were not printed.

Keep four observations separate:

| Evidence | What it establishes | What it does not establish |
| --- | --- | --- |
| Server preparation receipt | Exact memory IDs/versions prepared for a response | Client received the response |
| Successful MCP result or hook stdout | Content returned across that tested transport | Harness inserted it into a model request |
| Captured model request/context or independently observed nonce in model output | Context reached that tested model execution, with limits stated | Model followed every instruction in it |
| Paired task action following an exact correction | Observable use in that seeded task | General recall quality or future compliance |

A receipt, a successful MCP handshake, a registered tool and a provider review
must not be promoted into proof of automatic lifecycle injection. A model's
claim that it read memory is insufficient on its own.

## Observed installed surfaces

These initial inspection commands were run on this Windows host. Paths are
local observations, not portable prerequisites. The real-client observations
later in this document supersede their unverified status where stated.

| Surface | Executable/version | Configuration observation | Verified scope and remaining gap |
| --- | --- | --- | --- |
| Claude Code | `C:/Users/Bolado/.local/bin/claude.exe`, `2.1.291` | `claude mcp get cortex` exits 0: user scope, stdio, `cortex-mcp-server`, args `serve`, status `Connected` | MCP discovery/connection is observed. This command does not perform daily-memory capture/read or prove model-visible lifecycle context. |
| Codex CLI | VS Code extension binary `openai.chatgpt-26.51007.21434-win32-x64/bin/windows-x86_64/codex.exe`, `0.162.0-alpha.17.2` | `~/.codex/config.toml` contains `[mcp_servers.cortex]`, command `cortex-mcp-server`, args `serve` | Configuration exists. Standalone `codex mcp get cortex --json` exits 1 with a home-directory error in this execution environment; a process-local `HOME` set to the real user directory did not resolve it. This invocation does not verify loaded configuration or tool execution. |
| Kimi Code CLI | `C:/Users/Bolado/.kimi-code/bin/kimi.exe`, `0.29.2` | `kimi doctor` exits 0. Existing `~/.kimi-code/config.toml` is empty TOML. Neither user nor project `.kimi-code/mcp.json` exists in this snapshot. | Executable and configuration validation are observed. Cortex registration, provider readiness, MCP capture/read and lifecycle delivery are unverified. |
| Grok Build CLI | No `grok` executable resolved on `PATH` | No corresponding installed-client contract established | Unavailable on the inspected PATH; native harness integration is unverified. This does not assert absence from every directory or require an installation. |

`cortex-mcp-server`, `cortex-hook` and `cortex-adapter-claude` resolve to
`C:/Users/Bolado/.cargo/bin/`. Resolving a path does not establish that its
installed binary contains the current source changes. The new memory tool must
appear in that binary's actual `tools/list` before a new-client test relies on it.
The source-level TCP/SQLite scenarios and four-key 586 ms result in
[analysis08](08-implementation-evidence.md) are separate from installed-harness
acceptance.

## Shared explicit MCP contract

The preferred common path is a client-managed **stdio MCP process**:
`cortex-mcp-server serve`. Its HTTP API base is configured by `CORTEX_API_URL`
(local default `http://127.0.0.1:17000`). That API base is not itself an MCP
endpoint: do not register `http://127.0.0.1:17000` as a streamable-HTTP MCP
server without a separately implemented MCP HTTP transport.

Current source loads the configured API token through `CORTEX_API_TOKEN` /
`dashboard.api_token` and forwards it as Bearer authentication. Keep it in the
private parent-process environment or private operator configuration, rather
than putting a credential in a repository MCP file or prompt. API authorization
uses the verified identity; `clientInfo.name`, adapter caller headers and model
agent labels provide attribution only.

After initialization and `notifications/initialized`, the client must enumerate
tools and actually invoke the relevant descriptor:

| Tool | Required invocation | Expected observable result |
| --- | --- | --- |
| `cortex_capture_memory` | Explicit `kind`, `body`, exact `repo`; decisions/corrections use the typed decision fields | Durable local capture result and stable memory identity. Optional index completion is separate. |
| `cortex_memory_read` | Exact `repo`, ordinary task `query`; include actual `branch` and repository-relative `paths` when applicable | Bounded cited context, exact prepared IDs/versions, `query_id` and `delivery_id`. |
| `cortex_pre_thinking` | Actual `cwd`, current prompt and applicable recent-file information from its descriptor | Shared bounded local-first bundle, with local receipt metadata distinct from optional-search query IDs. |

For example, a direct memory read's tool arguments are:

```json
{
  "repo": "cortex",
  "query": "Which correction applies before changing the authentication cache?",
  "branch": "feature/example",
  "paths": ["crates/cortex-api/src/service.rs"],
  "include_global": false,
  "limit": 5,
  "budget_bytes": 1536
}
```

Use the actual repository identity and current branch, not these example values.
Do not invent session/turn IDs when the harness does not expose them. Explicit
correction requires an authorized accepted successor, its predecessor and the
matching expected revision. An ordinary model reply or negative feedback does
not perform supersession.

For an MCP-only client, an instruction such as "Before editing, call
`cortex_memory_read` with this repository and task; cite applicable memory IDs"
is an explicit workflow contract. It does not create automatic prompt hooks or
guarantee tool use. Record the actual tool-call/result frames.

## Client-specific setup and invocation semantics

The following setup commands are documented recipes, **not commands executed by
this review**. Preserve existing configuration and use the correct scope rather
than registering duplicate servers. Refresh/restart the affected client after
changing its server configuration.

### Claude Code

Installed help confirms this stdio registration syntax:

```text
claude mcp add --scope project cortex --env CORTEX_API_URL=http://127.0.0.1:17000 -- cortex-mcp-server serve
claude mcp get cortex
```

For this host, the existing entry is user-scoped and already reports Connected;
that is not a reason to add a second entry. Project MCP declarations use
`.mcp.json`. Their trust/approval state still matters.

Claude's [hook reference](https://code.claude.com/docs/en/hooks) documents
`SessionStart` and `UserPromptSubmit` context output. The intended synchronous
JSON shape is:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "UserPromptSubmit",
    "additionalContext": "bounded cited memory context"
  }
}
```

Use `SessionStart` for that event's response. A top-level `systemMessage` is an
operator-facing message, not evidence that memory reached the model.

The inspected global settings still contain
`cortex-hook SessionStart --fire-forget`, while `UserPromptSubmit` is synchronous.
The packaged shell shims also contain publish-only paths in this pre-repair
snapshot. Item 1.7 owns their repair and real invocation with a running adapter.
Do not install both global capture hooks and equivalent plugin hooks without
checking duplicate firing. Claude `--bare` deliberately skips configured hooks;
the provider consultation below used it and is therefore **not a lifecycle test**.

### Codex

Installed `codex mcp add --help` confirms:

```text
codex mcp add cortex --env CORTEX_API_URL=http://127.0.0.1:17000 -- cortex-mcp-server serve
codex mcp get cortex --json
```

The [official MCP documentation](https://developers.openai.com/codex/mcp)
specifies TOML configuration and trusted project scope. Equivalent configuration:

```toml
[mcp_servers.cortex]
command = "cortex-mcp-server"
args = ["serve"]

[mcp_servers.cortex.env]
CORTEX_API_URL = "http://127.0.0.1:17000"
```

The inspected user entry has API URL and adapter-socket environment names;
inherited private credentials were not examined. The repository has no
`.codex/config.toml` in this snapshot. `.mcp.json` alone is not proof that this
standalone Codex CLI loaded the same server.

Current [official hook documentation](https://developers.openai.com/codex/hooks)
includes `SessionStart`, `UserPromptSubmit` and `SessionEnd`, with trust and
surface-dependent execution requirements. Therefore "Codex has no hooks" is
not an accurate generic statement. However, no installed Codex hook was executed
by this review. Cortex's supported acceptance path remains explicitly tested
MCP until that particular harness's hook registration/output is demonstrated.
Do not treat old speculative `beforeCommand`/`onPrompt` adapter descriptions as
the installed CLI's contract.

### Kimi Code CLI

Use the documentation namespace linked by the installed executable:
[`kimi-code` MCP](https://moonshotai.github.io/kimi-code/en/customization/mcp.html).
It differs from older `kimi-cli` documentation. Current documented files are
`~/.kimi-code/mcp.json` and project `.kimi-code/mcp.json`; project entries with
the same name override user entries. Example:

```json
{
  "mcpServers": {
    "cortex": {
      "command": "cortex-mcp-server",
      "args": ["serve"],
      "env": {"CORTEX_API_URL": "http://127.0.0.1:17000"}
    }
  }
}
```

The documented TUI commands are `/mcp-config` for configuration and `/mcp` for
connection status. The installed CLI supports `kimi -p "..."` and
`--output-format stream-json`. Project MCP servers require workspace trust;
headless mode cannot show that trust prompt. Configuration written during an
existing session is not sufficient proof that a new server is registered there.
No Kimi MCP configuration or trust setting was changed in this review.

Do not prescribe the older `--mcp-config-file` flag: it is absent from this
installed executable's help. Hook output rules from analysis03's upstream
comparison are not evidence that this Kimi version consumed Cortex output.

### Grok

No native Grok invocation or configuration file is prescribed without the
installed client's help and a real session. A Grok model selected in a different
MCP-capable client is a valid test of **that client plus that model**, not of
Grok Build CLI hooks. A direct model request is provider participation only.

## Independent Claude provider review

An independent review completed through the installed Claude CLI routed through
the local 9router endpoint, using `--bare`, `--strict-mcp-config`, no tools,
`--system-prompt` with the bounded review request, `--model
cc/claude-fable-5-1`, and `--no-session-persistence`. It did not resume or copy the
unreachable `cortex-73` session. No additional SendMessage attempt was made.

- Requested model: `cc/claude-fable-5-1`; CLI-reported canonical model:
  `claude-fable-5-1`. These are execution metadata, not independent proof of
  upstream provider identity.
- Result: success, one turn, 73.8 seconds. Reported list-cost estimate:
  USD 0.2639325; cache creation input 1,512 tokens, reported output 4,786 tokens,
  including 2,164 thinking tokens. This is not a provider invoice.
- Local raw artifact:
  `%TEMP%/cortex-recovery-model-review/cc-claude-fable-5-1-section17-lifecycle-review.raw`.

The reviewer prioritized synchronous lifecycle output, per-client configuration
and observable separation of preparation, transport return, model-visible
context and task action. It proposed a synthetic canary, correction/removal and
restart checks. Its comments about `tool_input` and `/v1/laws/check` reused facts
supplied from analysis04; they are not a fresh code inspection and must be
rechecked against the repaired code. Its suggested additional action gate does
not change checklist order: comprehensive behavior remains item 1.10.

## Optional reproducible installed-client probe

[`scripts/daily_memory_live_harness.py`](../../../scripts/daily_memory_live_harness.py)
prepares an explicit, bounded capture/read probe for the installed Claude, Codex
or Kimi executable. Default invocation prints help and makes no model call.
`--self-test` checks synthetic scope enforcement, exact-ID/version validation,
soft-error rejection and CLI usage extraction offline. Those offline checks
passed on 2026-10-16; they are not installed-client acceptance evidence.

Build current `cortex-mcp-server` and the `daily_memory_fixture` example before
running this optional probe. Set `CORTEX_LIVE_ROUTER_KEY` privately in the parent
process environment; do not put its value in a command line, this document or
the evidence bundle. A sample PowerShell invocation, using the current working
tree's binaries, is:

```powershell
python scripts/daily_memory_live_harness.py `
  --client claude --model cc/claude-fable-5-1 `
  --mcp-binary target/debug/cortex-mcp-server.exe `
  --fixture-binary target/debug/examples/daily_memory_fixture.exe `
  --timeout 120 --run
```

Use `--client codex --model <verified-router-model>` or `--client kimi --model
kimi/kimi-k3` for separate fresh runs. `--client-binary` selects an explicit
installed executable when it is not on PATH. `--router-url` defaults to the
local router at `http://127.0.0.1:20128`; Claude uses that base, while Codex and
Kimi use its `/v1` API. Each invocation is one client attempt. No model gateway,
installation, global configuration edit or OS schedule is involved.

`--native-auth` instead lets the installed client resolve its own configured
login/provider; it does not inspect, copy or inject credentials. This mode needs
no `CORTEX_LIVE_ROUTER_KEY`. A model alias must match that native provider.
For example, the verified Codex run used `--client codex --native-auth --model
cx/gpt-6.1-sol`, with its already-configured local router provider. The actual
Claude lifecycle probe used native `claude-fable-5-1`, without the router's
`cc/` prefix.

Every run creates a retained private TEMP artifact directory and a new fixture
database with a unique nonce and memory ID. The fixture exposes an ephemeral
loopback API, repository `phase34-harness`, disabled authentication explicitly
limited to synthetic data, unavailable optional indexing lanes and disabled
model advice. Alternatively, `--ready-json` accepts ready metadata from an
already-running fixture with those same explicit properties; start a fresh
fixture for each test. It is not an arbitrary production API URL option.

The temporary stdio MCP entry starts a transparent proxy around the specified
server executable. The proxy advertises only `cortex_capture_memory` and
`cortex_memory_read`, permits the exact synthetic capture and bounded read, and
rejects other tool calls and global scope. It gives the server an empty explicit
Cortex configuration, removes inherited Cortex settings and credentials, and
passes the fixture URL explicitly. This prevents accidental production memory
access even when the client retains its normal home directory.

Claude uses `--bare`, strict explicit MCP configuration, no built-in tools and
two explicit MCP permissions. Codex preserves its real HOME/auth lookup while
using `--ignore-rules`, read-only sandbox and explicit MCP overrides. The
private-environment provider mode also uses `--ignore-user-config` and explicit
provider overrides. Native-auth mode retains other installed configuration;
MCP table overrides merge rather than removing other servers. The observed
native Codex session initialized unrelated installed servers, but its model
called only the requested Cortex tools. Both modes set documented per-tool
`approval_mode = "approve"` only for the two synthetic tools behind the proxy.
See the [official OpenAI configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
for these MCP policy keys.

Kimi's private-environment mode uses a temporary `KIMI_CODE_HOME`, explicit
provider environment settings and two MCP permission rules. Native-auth mode
preserves its existing home/login and writes only a fresh fixture project's
`.kimi-code/mcp.json`. Its process-local workspace trust flag does not persist
trust. User credentials/configuration are not copied into retained artifacts.

The MCP journal records initialize identity, client requests, upstream results
and the exact client-facing tool results. Only the advertised tool catalog is
filtered; capture/read results pass through unchanged. Known credential values
are redacted before persistence. `report.json` requires a committed capture for
the exact memory ID/version 1, its response before the read request, matching
read ID/version, cited nonce-bearing context within 1,536 UTF-8 bytes, and equal
nonempty delivery/query IDs. Client exit status and timeout also affect pass.
Model prose alone cannot make the probe pass.

Timeout is 120 seconds by default, bounded to at most 300. Kimi has six maximum
steps and one attempt per step; its private-environment mode additionally sets
the supported 1,536-token completion cap. Claude's
optional `--max-budget-usd` forwards its supported native budget flag. No
unverified Codex output/turn cap or universal dollar cap is claimed. Recognized
CLI-reported usage/cost fields are retained exactly; absent spend is reported
as unavailable. Termination targets only processes started by the probe.

This harness tests explicit real-client MCP tool transport. It does not test
Claude lifecycle hooks, prove an ordinary prompt triggers memory automatically,
establish restart/correction behavior, or complete item 1.7 on its own. Sandbox
home/auth failures must be distinguished from installed-client failures; repeat
the important failing CLI outside that execution restriction before concluding
the client is broken.

## Real installed-client observations

The following fresh-session runs used the current locally built server and
synthetic fixture. These were real installed model clients, rather than direct
API requests or a model's unsupported claim that it used tools. All processes
started by these probes ended. Raw artifacts remain local in the indicated
TEMP directories; their `report.json`, invocation, version, CLI output and MCP
journal retain the detailed evidence.

| Installed path | Observed result | Exact returned evidence | Local artifact directory under `%TEMP%` |
| --- | --- | --- | --- |
| Claude Code 2.1.291, `cc/claude-fable-5-1`, bare MCP probe | Capture followed by read passed, exit 0 | `probe-618ffcad0bee4a47b3b85d7f5f5191e0`, version 1; delivery/query `01M520H0Y52VA42PH1B5KPNHM2`; cited context 283 UTF-8 bytes | `cortex-claude-memory-probe-2mhcjoy8` |
| Codex 0.162.0-alpha.17.2, native configured provider, `cx/gpt-6.1-sol` | Capture followed by read passed, exit 0 | `probe-bc62d4c5f87e428984435329c55ee369`, version 1; delivery/query `01M520YN58V52J3TSXQP63HPRW`; cited context 288 UTF-8 bytes | `cortex-codex-memory-probe-w2x9m_ru` |
| Kimi Code 0.29.2, native configured provider, requested `kimi/kimi-k3` | MCP initialized and listed both tools; model configuration failed before capture/read, exit 1 | Client identity `kimi-code`, reported MCP version `0.0.0`; no successful tool result or model usage | `cortex-kimi-memory-probe-ykaabeze` |
| Grok Build CLI | No executable resolved on inspected PATH | No native-client execution or tool-delivery claim | None |

Claude's actual MCP initialize identity was `claude-code` version `2.1.291`.
Codex's was `codex-mcp-client` version `0.162.0-alpha.17.2`. Their committed
capture receipts preceded their read requests; both returned the exact unique
ID/version and equal nonempty delivery/query IDs. Read receipt stage remained
`prepared`. Context returned through MCP is observed transport evidence;
general instruction compliance is not inferred from the receipt.

Claude's successful MCP probe reported 32.129 seconds and list-cost estimate
USD 0.0894905, with 8 input, 3,746 cache-creation input, 5,892 cache-read input
and 466 output tokens. Codex's successful native run reported 67,598 input
tokens, including 49,536 cached input, and 363 output tokens, including 40
reasoning output. Codex reported no dollar cost; none is estimated here.
These CLI metrics are not invoices or independent provider-identity proof.

Important setup failures were retained rather than hidden:

- Codex's sandboxed launch failed before MCP with `Could not find home
  directory` (`cortex-codex-memory-probe-bao59sa2`). A secret-free native
  `codex mcp list --json` succeeded outside that restriction, finding ten
  configured servers including Cortex. The home failure was therefore an
  execution-environment restriction, not proof of a broken installed client.
- An escalated repeat that extracted the active router key from its private
  SQLite store was rejected by automatic approval review. Its reason was that
  model-testing authorization did not specifically authorize that credential
  source. No escalated model ran from the rejected command; no further store
  extraction or indirect key reuse followed. Native client auth provided the
  successful Codex and lifecycle alternatives.
- Two native Codex starts failed configuration parsing before a model call:
  quoting names inside dotted `-c` overrides created literal quoted server
  names, and disabling an unrelated discovered entry without a transport
  created an invalid configuration. A table override subsequently merged
  existing entries rather than isolating the catalog. The final contract does
  not claim native-auth mode disables those other servers.
- Codex's first actual model turn selected the exact capture arguments, but
  the client rejected the call before sending it: `MCP tool call requires
  approval, but approval policy is never`. Its retained failed run is
  `cortex-codex-memory-probe-674p451u`; usage was 32,191 input and 341 output
  tokens, dollar cost unavailable. The documented per-tool `approval_mode =
  "approve"` for the two scope-restricted tools fixed the setup. There was no
  unexplained provider retry.
- Kimi's exact native failure was `config.invalid: Model "kimi/kimi-k3" is
  not configured in config.toml. Add a [models."kimi/kimi-k3"] entry with
  max_context_size.` The MCP handshake proves discovery, not capture/read or
  model-visible memory. No user model configuration or login was changed.

## Actual Claude SessionStart model-visible delivery

[`scripts/daily_memory_claude_hook_probe.py`](../../../scripts/daily_memory_claude_hook_probe.py)
adds the separate lifecycle check. It copies the current fixture, adapter and
native hook executables into a private TEMP directory, starts a fresh adapter
on a unique named pipe and seeds one accepted pinned synthetic memory. Its
wrapper runs the real packaged `cortex-session-start.sh`, records the actual
Claude hook input and native stdout, and forwards stdout unchanged. The
adapter/hook use only explicit temporary configuration and the synthetic API.

The real Claude client runs without `--bare`, using `--setting-sources ""`,
temporary explicit `--settings`, no built-in tools, empty strict MCP config,
no session persistence, low effort, a USD 3 supported native budget flag and a
180-second timeout. Native client auth is preserved; no credential store is
read by the probe. A reproducible invocation is:

```powershell
python scripts/daily_memory_claude_hook_probe.py `
  --fixture-binary target/debug/examples/daily_memory_fixture.exe `
  --adapter-binary target/debug/cortex-adapter-claude.exe `
  --hook-binary target/debug/cortex-hook.exe `
  --claude-binary C:/Users/Bolado/.local/bin/claude.exe `
  --model claude-fable-5-1 --timeout 180 --max-budget-usd 3 --run
```

The successful artifact directory is
`%TEMP%/cortex-claude-lifecycle-probe-xv6cisys`. The capture committed memory
`lifecycle-61ae523a84be47778870cac55b70f623`, version 1. The packaged hook returned
`hookSpecificOutput.hookEventName = "SessionStart"` and `additionalContext`
containing its exact citation and marker
`hookcanary5e704cb0754a40c5b8207268a11eff15`. That marker was absent from the user
prompt, system prompt and invocation arguments. The actual Claude final model
result was exactly the same marker, with no tool available to fetch it by
another route. This establishes model-visible delivery for this tested
SessionStart execution.

A subsequent read-only inspection of this disposable fixture's database found
committed preparation receipt `01M5217W9PG1VAQC4X3VVXFSXW`, stage `prepared`,
repository `phase34-harness`, revision 1 and the same exact memory ID/version.
Its rendered memory context was 304 UTF-8 bytes. The export is retained as
`prepared-deliveries.json` beside the probe artifacts. The receipt independently
establishes preparation; the real hook return and matching model result supply
the separate transport and model-visibility observations.

The CLI completed successfully in 10.602 seconds, reporting 2 input tokens,
886 cache-creation input tokens, 27 output tokens and list-cost estimate
USD 0.01909. An earlier native attempt with router alias
`cc/claude-fable-5-1` reached the hook but the provider rejected that model
name before usage (`cortex-claude-lifecycle-probe-3hs4_x2c`, zero reported tokens
and USD 0). One correction to observed native canonical `claude-fable-5-1`
produced the successful run; it did not inject a router credential.

This is one exact synthetic lifecycle delivery, not proof of broad recall,
cross-session correction, restart behavior or general instruction compliance.
The Rust daemon/packaged-hook integration tests cover outage and timeout paths;
their quality-gate evidence belongs to the task's implementation record.
Comprehensive behavioral acceptance remains item 1.10 and operator activation
remains item 1.11. Kimi model execution and native Grok remain explicitly
unverified for the concrete availability/configuration reasons above.
