## ADDED Requirements

### Requirement: Self-discovered runtime config
Cortex binaries SHALL locate their runtime config without any client-supplied
path. When `CORTEX_CONFIG_FILE` is unset, they MUST load `config.toml` from the
Cortex home directory (`~/.cortex`, overridable by `CORTEX_HOME`). Precedence
SHALL be CLI > environment > `CORTEX_CONFIG_FILE` > user config >
`./cortex.toml`. The repository `cortex.toml` bootstrap contract MUST NOT change.

#### Scenario: MCP server launched with no env block
Given `~/.cortex/config.toml` selects the daily profile
And the MCP server process has no `CORTEX_*` variables
When the server starts from any working directory
Then it resolves the daily profile, API URL and adapter pipe from the user config.

#### Scenario: Explicit file still wins
Given both `CORTEX_CONFIG_FILE` and `~/.cortex/config.toml` exist
When a binary loads config
Then values come from `CORTEX_CONFIG_FILE`, layered under environment and CLI.

### Requirement: Credential stays with the binaries
The daily API token SHALL be read from a credential file under the Cortex
home, restricted to the current user, when `CORTEX_API_TOKEN` is unset. No
plugin file, client config or repository file MAY contain the token. The token
MUST NOT appear in logs, `Debug` output or error bodies.

#### Scenario: Plugin-launched MCP authenticates
Given the credential file exists and no client config carries a token
When the plugin-scoped MCP server calls the daily API
Then the request authenticates and the tool returns data.

### Requirement: App-managed lifecycle without OS autostart
When the adapter pipe is absent, `cortex-hook` SHALL start the daily runtime
detached on `SessionStart` or the first synchronous event. It SHALL hold a
single-instance lock, honor a spawn backoff and stay within the hook budget.
It MUST NOT register a Run key, scheduled task, Startup entry or service.
It SHALL emit the operator `systemMessage` only when starting fails, at most
once per session, and never as `additionalContext`.

#### Scenario: First session after a reboot
Given no Cortex process is running
When Claude Code fires `SessionStart`
Then the runtime starts in the background and the hook exits 0 within budget
And a following `UserPromptSubmit` receives local memory context.

#### Scenario: Concurrent sessions
Given two sessions fire `SessionStart` at the same time with no runtime running
When both hooks run
Then exactly one runtime instance starts and both hooks exit 0.

#### Scenario: Start fails
Given the runtime binary is missing or exits immediately
When `SessionStart` fires
Then the operator sees one `systemMessage` naming the failure
And no further notices appear in that session.

### Requirement: Self-contained plugin distribution
The repository root SHALL publish a Claude Code marketplace whose Cortex plugin
bundles hooks, a plugin-scoped MCP entry and skills. The plugin MUST NOT need a
project `.mcp.json` or hand-written entries in user settings. Its launcher SHALL
find `cortex-hook` on PATH or known user directories and honor only an absolute
override. The MCP entry SHALL run `cortex-mcp-server serve` with no env block
and no path. A Codex plugin SHALL mirror this within Codex's supported hooks.

#### Scenario: Clean install
Given a host with Cortex binaries on PATH and `~/.cortex` initialized
When the operator runs `claude plugin marketplace add` and `claude plugin install`
Then a new session captures lifecycle events and lists the Cortex MCP tools
And no Cortex entry exists in `~/.claude/settings.json`, `~/.claude.json` or `.mcp.json`.

#### Scenario: Relative binary override
Given the plugin's binary-directory option holds a relative path
When a hook runs
Then the override is ignored and PATH discovery is used.

#### Scenario: Legacy hooks still present
Given legacy Cortex hooks remain in `~/.claude/settings.json`
When an event fires
Then the plugin path skips it, the event is captured once, and doctor reports the double install.

### Requirement: Reversible migration
Migration SHALL remove existing absolute-path Cortex hook and MCP entries and
the stale marketplace registration only after the plugin is installed and
verified. It SHALL default to dry-run, write hash-verified backups and provide
rollback. It MUST NOT change unrelated settings, native memory files or data.

#### Scenario: Migration then rollback
Given the plugin is verified on this host
When migration applies and rollback then runs
Then client files return byte-identical to their backups
And all 535 native memory files keep their hashes.
