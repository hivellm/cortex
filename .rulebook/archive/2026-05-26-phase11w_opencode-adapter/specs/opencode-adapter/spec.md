# OpenCode adapter

## ADDED Requirements

### Requirement: Envelope tool enum accepts `opencode`

The envelope schema (`crates/cortex-core/schemas/envelope.schema.json`)
MUST include `"opencode"` as a valid value of the `tool` enum so
events captured from OpenCode sessions can be ingested without schema
errors.

#### Scenario: opencode envelope passes schema validation

Given an envelope with `tool = "opencode"` and otherwise spec-04-compliant fields
When the envelope is POSTed to `cortex-ingestion /v1/events/batch`
Then the response status SHALL be 2xx and the event SHALL appear on `cortex.events.raw`

#### Scenario: legacy claude-code envelopes still validate

Given an envelope with `tool = "claude-code"`
When the envelope is POSTed to `cortex-ingestion /v1/events/batch`
Then the response status SHALL be 2xx and the event SHALL appear on `cortex.events.raw`

---

### Requirement: Adapter daemon serves an HTTP transport

The `cortex-adapter-claude-code` daemon (renamed crate optional in a
future task) MUST accept hook payloads over HTTP `POST /hook` in
addition to the existing Unix-socket / named-pipe transport.

The HTTP listener MUST be controlled by env var
`CORTEX_ADAPTER_HTTP_BIND` (default `127.0.0.1:17004`). When the env
var is unset, the HTTP transport MUST NOT bind. When set, BOTH
transports run concurrently and funnel into the same `Dispatcher`.

#### Scenario: socket and HTTP produce identical envelopes

Given the daemon is running with both `CORTEX_ADAPTER_SOCK` and `CORTEX_ADAPTER_HTTP_BIND` set
When the same hook payload is POSTed once over the socket and once over HTTP
Then both invocations SHALL produce envelopes that are byte-for-byte identical except for `event_id`

#### Scenario: HTTP listener disabled by default

Given `CORTEX_ADAPTER_HTTP_BIND` is unset
When the daemon boots
Then no TCP socket SHALL be opened and `lsof -i :17004` SHALL show no process

---

### Requirement: OpenCode plugin captures session activity

The TS plugin `@hivellm/cortex-opencode-plugin` MUST subscribe to the
OpenCode lifecycle events that map to the canonical hook kinds (per
spec 10) and post each event to the daemon's HTTP listener as JSON.

The wire format MUST match the existing socket payload:
`{ "hook": "<HookKind>", "session_id": "...", "cwd": "...", "payload": {...} }`.

The plugin MUST fail open: when the adapter is unreachable, the
plugin SHALL log a warning and SHALL NOT block or break the OpenCode
session.

#### Scenario: tool.execute.before is forwarded as PreToolUse

Given the plugin is loaded into an OpenCode session
When OpenCode emits `tool.execute.before` for any tool
Then the plugin SHALL POST `{"hook":"PreToolUse", ...}` to `CORTEX_ADAPTER_HTTP_BIND/hook`

#### Scenario: adapter unreachable does not break the session

Given the plugin is loaded but `cortex-adapter` is not running
When the user submits a prompt
Then the prompt SHALL still be delivered to the model and the plugin SHALL log one warning per failed POST

#### Scenario: kill-switch disables the plugin

Given `CORTEX_OPENCODE_DISABLE=1` is set in the OpenCode environment
When OpenCode emits any lifecycle event
Then the plugin SHALL skip all POSTs and emit zero envelopes

---

### Requirement: Pre-thinking bundle injection

The plugin MUST inject the Cortex pre-thinking bundle into the
**current** model call so the model sees institutional context before
planning. The injection mechanism is the one validated by the spike
documented at `docs/analysis/opencode-adapter/00-spike.md`.

#### Scenario: bundle reaches the model on user prompt submit

Given the plugin is loaded and `cortex-api /v1/query` returns a non-empty bundle
When the user submits a prompt
Then the model's input for that turn SHALL include the bundle text and the audit envelope at `cortex-api` SHALL record the bundle's `query_id`

#### Scenario: bundle timeout fails open

Given `cortex-api /v1/query` takes longer than `CORTEX_OPENCODE_PRE_THINKING_TIMEOUT_MS` (default 1500)
When the user submits a prompt
Then the prompt SHALL still be delivered to the model with no bundle and the plugin SHALL emit one telemetry event flagging the timeout

---

### Requirement: MCP servers reachable inside OpenCode

The project's `opencode.json` MUST register `cortex-mcp-server` and
the `rulebook` MCP server under the `mcp` key with the same env vars
the existing `.mcp.json` provides to Claude Code.

#### Scenario: cortex_query is callable from OpenCode

Given an OpenCode session with the project's `opencode.json` loaded
When the model calls the MCP tool `cortex_query` with a non-empty query
Then the tool SHALL return a `QueryResponse` matching the spec-11 shape

---

### Requirement: Custom commands at parity with Claude Code

For every file under `.claude/commands/*.md`, an equivalent file
under `.opencode/commands/*.md` MUST exist with frontmatter compliant
to OpenCode's command schema (`template:` required) and the same
behavioural contract (positional `$1`/`$2` and `$ARGUMENTS`
expansions preserved).

#### Scenario: rulebook-task-list slash command

Given the project's `.opencode/commands/rulebook-task-list.md` is installed
When the user types `/rulebook-task-list` in OpenCode
Then the model SHALL receive the same prompt body the Claude Code version delivers and SHALL invoke `mcp__rulebook__rulebook_task_list`

---

### Requirement: Agents at parity with Claude Code

For every file under `.claude/agents/*.md`, an equivalent file under
`.opencode/agents/*.md` MUST exist with frontmatter encoding model,
temperature, max_steps, and permission rules (`allow`/`ask`/`deny`
per tool category, glob patterns supported).

#### Scenario: researcher subagent

Given `.opencode/agents/researcher.md` is installed
When the user invokes `@researcher find recent decisions about retention`
Then OpenCode SHALL spawn the subagent with the configured model and tool gating, and `cortex-adapter` SHALL receive the corresponding `AgentCall` envelope

---

### Requirement: Backwards compatibility

Adding the OpenCode adapter MUST NOT change any behaviour of the
Claude Code adapter. Specifically:

- The Unix-socket / named-pipe transport on `cortex-adapter-claude-code` MUST keep accepting payloads with the existing JSON shape.
- `tool = "claude-code"` envelopes MUST keep validating against the schema.
- The existing `.claude/{commands,agents,hooks,rules,skills}/` directories MUST keep being loaded by Claude Code without changes.

#### Scenario: Claude Code session unchanged

Given the project ships both `.claude/` and `.opencode/` directories
When a Claude Code session boots
Then it SHALL load only `.claude/` and SHALL NOT read any file from `.opencode/`

#### Scenario: socket transport regression suite green

Given the existing `crates/cortex-adapter-claude-code/tests/dispatcher.rs` suite
When the test binary runs after the HTTP listener change
Then every existing test SHALL pass without modification
