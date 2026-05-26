# Phase11w §10 — End-to-end smoke plan

> The §10 end-to-end smoke runs against a live OpenCode session and
> therefore must be operator-led. This document is the script the
> operator follows once the §2 + §4 + §5 artefacts are committed.
> The automated §2.6 HTTP-parity IT covers the daemon's HTTP listener
> contract independently; this smoke covers the OpenCode → plugin →
> daemon → Synap end-to-end path.

## Prerequisites

- `cortex-api`, `cortex-adapter-claude-code` (with HTTP listener),
  Synap, Vectorizer, Nexus, Meili running locally (the
  `bin/cortex-init.sh` brings them up).
- `cortex-mcp-server` on PATH.
- OpenCode CLI installed (`opencode --version` succeeds) per the
  spike doc's pinned version.
- `@hivellm/cortex-opencode-plugin` resolvable. During local dev,
  point `opencode.json` `plugin` entry at the package's
  `dist/index.js` after `pnpm -C packages/cortex-opencode-plugin
  build` lands a clean build.

## Step-by-step

1. **Bring up the stack**:
   ```bash
   bin/cortex-init.sh
   CORTEX_ADAPTER_HTTP_BIND=127.0.0.1:17004 cortex-adapter-claude daemon &
   ```

2. **Launch OpenCode inside this repo**:
   ```bash
   opencode
   ```

3. **Confirm the MCP tools land**:
   - In the TUI, type `/mcp` and verify `cortex_query`,
     `cortex_pre_thinking`, `cortex_status`,
     `cortex_active_work`, `cortex_similar_sessions`,
     `cortex_decision_chain`, `cortex_keyword_search`,
     `cortex_vector_search`, `cortex_graph_query`,
     `cortex_audit`, `cortex_capture_memory`,
     `cortex_session_replay`, and `cortex_forget` appear.

4. **Submit a representative prompt** that triggers at least one
   tool call:
   - "Read `crates/cortex-adapter-claude-code/src/dispatcher.rs`
     and summarise the dispatch contract."
   - Expect the assistant to call a `read_file` tool, return a
     summary, and have access to the pre-thinking bundle in its
     context (look for `## active work` or `## consolidations`
     sections in the assistant's reasoning trace).

5. **Drive a subagent**:
   - Invoke `@researcher` (one of the ported agents from
     `.opencode/agents/researcher.md`).
   - Confirm the subagent runs with the configured
     `permission` block (no Write/Edit allowed).

6. **Inspect Synap `cortex.events.raw`** for the session:
   ```bash
   synap consume cortex.events.raw --limit 16 --format json | \
     jq '. | select(.tool == "opencode")'
   ```
   - Expect ≥ 4 envelopes carrying `tool: "opencode"`:
     * 1 × `Turn` (UserPromptSubmit)
     * 1+ × `ToolCall` (the model's tool call)
     * 1 × `AgentCall` (subagent boundary)
     * 1 × `Turn` (final Stop)

7. **Kill-switch parity**:
   - Set `CORTEX_OPENCODE_DISABLE=1`; restart OpenCode.
   - Re-run step 4.
   - Expect zero envelopes on `cortex.events.raw` for the
     session and a WARN log line `[cortex-opencode]
     CORTEX_OPENCODE_DISABLE=1; plugin inactive`.

8. **Law-check parity** (when laws are configured):
   - Pose a prompt that exercises a known-deny law (e.g. an
     attempted destructive git op when LAW-CORTEX-001 applies).
   - Confirm the plugin's `permission.asked` handler returns
     `"deny"` and the OpenCode TUI surfaces the denial.

## Pass / fail criteria

| Check | Pass condition |
|-------|----------------|
| 3 | All 13 cortex MCP tools listed by `/mcp`. |
| 4 | Pre-thinking bundle visible in assistant context. |
| 5 | Subagent runs with the configured permission block. |
| 6 | ≥ 4 `tool: "opencode"` envelopes on the lane. |
| 7 | Zero envelopes; WARN log line emitted. |
| 8 | Tool call denied with the law's reason string. |

A failure on any row blocks promoting the §10 task item to `[x]`.
Mismatches against the spike answers in
[`00-spike.md`](./00-spike.md) land as a follow-up task that
patches the plugin's runtime feature-detection.
