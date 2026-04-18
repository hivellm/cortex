# 17 — Additional adapters (Cursor, Codex, Gemini CLI)

> **Status:** 🟡 Draft · **Owner:** Core team · **Depends on:** 10

## Goal

Extend capture to the other CLI/agent surfaces the team uses — **Cursor**, **Codex CLI**, and **Gemini CLI** — by reusing the reference adapter pattern from spec 10. Each additional adapter is a thin shim + a small daemon reusing most of `cortex-adapters/common/`; per-tool divergence is in *hook semantics*, not in Cortex internals.

## Scope

**In:**
- `cortex-adapters/common/` crate — extracts the reusable pieces from spec 10 (IPC, publisher, session correlation, redaction, install/uninstall framework, overflow WAL).
- Three new adapter crates:
  - `cortex-adapters/cursor/`
  - `cortex-adapters/codex/`
  - `cortex-adapters/gemini/`
- Per-tool hook wiring + envelope mapping (event kinds → Cortex kinds).
- Shared install CLI: `cortex-adapters install <tool>`.
- Per-tool acceptance tests (script-level).

**Out:**
- Any change to the Cortex core pipeline.
- UI-rich IDE integrations (VSCode extension, JetBrains plugin) — separate future specs.
- Cross-tool session unification (a user using Claude Code + Cursor in the same repo stays as two sessions; correlation is graph-side).
- Tools we do not currently use (Aider, Continue, etc.) — easily added later via the same template.

## Inputs / Outputs

### Layout

```
cortex-adapters/
├─ common/
│  ├─ ipc/            (UDS + named-pipe)
│  ├─ publisher/      (batched HTTP → cortex-core, overflow WAL)
│  ├─ session/        (session_id, turn_id, tool_call_id correlation)
│  ├─ redact/         (pattern catalog; mirrors cortex-core)
│  ├─ pre_thinking/   (shared; spec 12 lives in common)
│  ├─ law_check/      (shared)
│  ├─ install/        (framework + per-tool hook scaffolding)
│  └─ config/
├─ claude-code/       (spec 10 reference impl; uses common/)
├─ cursor/            (this spec)
├─ codex/             (this spec)
└─ gemini/            (this spec)
```

All adapters are **single binary**: `cortex-adapter-<tool>`. `cortex-adapters install <tool>` is a sub-command of a fourth binary (`cortex-adapters`) that orchestrates install/uninstall/status for all of them.

### Per-tool surface matrix

| Capability                     | Claude Code | Cursor | Codex CLI | Gemini CLI |
|--------------------------------|:-----------:|:------:|:---------:|:----------:|
| Capture user prompt            | ✅           | ✅     | ✅         | ✅          |
| Capture tool calls             | ✅           | ⚠️ partial | ✅     | ⚠️ partial  |
| Blocking laws (sync `PreToolUse`) | ✅        | ❌ no hook| ✅      | ❌ no hook  |
| Pre-thinking injection         | ✅           | ⚠️ workspace prompt | ✅ | ⚠️ prepended message |
| Session lifecycle events       | ✅           | ⚠️         | ✅         | ⚠️          |
| Notification events            | ✅           | —          | —          | —          |

"⚠️" means we approximate via alternative hooks (file watchers, CLI argument wrappers, or message prepending).

### Adapter-specific hook tables

#### Cursor

Cursor's programmatic surface is narrower (no per-tool hook). We capture what we can:

| Source                                           | Cortex kind                     |
|--------------------------------------------------|---------------------------------|
| `cursor chat` workspace prompt (file watcher)    | `turn.user`                     |
| Cursor's edit events (filesystem watcher)        | `tool_call.edit_inferred`       |
| `rules/*.md` changes                             | `memory.imported` (new scope)   |
| Session boundary (process start / exit)          | `turn.session_start/stop`       |

Blocking laws are **not** supported (no `PreToolUse`). Critical violations become observational + reminders into the next prompt via a Cursor-`rules`-file fragment we write before each prompt.

#### Codex CLI

Codex has a plugin system with a programmatic pre-command hook. Good parity with Claude Code:

| Source                    | Cortex kind                       |
|---------------------------|-----------------------------------|
| `beforeCommand`           | `tool_call.requested` + sync law check |
| `afterCommand`            | `tool_call.completed`              |
| `onPrompt`                | `turn.user` + pre-thinking inject  |
| `onSessionStart/Stop`     | session lifecycle                  |

Blocking laws supported; pre-thinking injected as a `system`-role message prepended on `onPrompt`.

#### Gemini CLI

Gemini's CLI exposes a `GEMINI_CLI_PRE_PROMPT_HOOK` env hook and an `onToolCall` callback via the experimental SDK. Partial parity:

| Source                       | Cortex kind                     |
|------------------------------|---------------------------------|
| Pre-prompt hook              | `turn.user` + pre-thinking (prepended to the user message; not a true system message) |
| `onToolCall`                 | `tool_call.requested` (no sync law check — callback is post-confirmation) |
| Session end (process exit)   | `turn.session_stop`              |

Blocking laws not supported — all governance is observational for Gemini.

### Install commands

```
cortex-adapters install claude-code
cortex-adapters install cursor
cortex-adapters install codex
cortex-adapters install gemini

cortex-adapters install all               # idempotent fan-out
cortex-adapters status                    # table: tool / daemon / hooks / last event

cortex-adapters uninstall <tool>           # reverts that tool only
cortex-adapters uninstall all             # reverts everything; keeps WAL + logs
cortex-adapters uninstall all --purge     # also removes WAL/logs
```

## Design

### `cortex-adapters/common/` extraction

The spec-10 reference implementation is **refactored**, not re-copied, so that a bug fixed in common benefits every adapter.

Key traits:

```rust
pub trait Adapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn install(&self, opts: &InstallOpts) -> Result<InstallReport>;
    fn uninstall(&self, opts: &UninstallOpts) -> Result<()>;
    fn envelope_from_hook(&self, hook: HookKind, raw: &serde_json::Value) -> Result<Vec<Event>>;
    fn supports_blocking_laws(&self) -> bool;
    fn pre_thinking_inject(&self, bundle: &str, raw_hook: &mut serde_json::Value) -> Result<()>;
}

pub struct Daemon<A: Adapter> {
    adapter: A,
    ipc: IpcServer,
    publisher: Publisher,
    session_mgr: SessionManager,
    redactor: Redactor,
    law_client: LawClient,
    query_client: QueryClient,
}
```

Each per-tool crate is basically:

- An `Adapter` impl (hook mapping + install glue).
- A `main.rs` that builds `Daemon<MyAdapter>` and runs it.
- Static hook shims (shell scripts or a tiny wrapper binary).

### Cursor specifics

- **File watcher:** `notify` crate monitors `<workspace>/.cursor/rules/*.md`, `<workspace>/.cursor/prompts/*.md`, and `<workspace>/.cursor/chat/*.jsonl`. New entries in the chat JSONL become `turn.user` events; diff-detection against a checkpoint keeps us from double-emitting.
- **Edit inference:** Cursor doesn't announce tool calls; we shadow-watch the workspace filesystem for *meaningful* edits (debounced, ignores `.git/` and other noise). Edits are tagged `tool_call.edit_inferred` so the graph writer keeps them cleanly separable from truly-observed tool calls.
- **Pre-thinking:** we write a transient file `<workspace>/.cursor/rules/_cortex_context.md` that Cursor auto-includes in its system context. The file is rewritten per-prompt and deleted on session end.
- **Governance:** only observational. A reminder-only workflow: after a violation, the `_cortex_context.md` gets a `> ⚠️ LAW-… reminder` block for the next prompt.

### Codex CLI specifics

- Plugin registered via `~/.codex/plugins.toml` → points at `cortex-adapter-codex`.
- Hooks: `beforeCommand`, `afterCommand`, `onPrompt`, `onSessionStart`, `onSessionStop`. Same sync/async contract as Claude Code.
- Pre-thinking: prepended as a `system` message to `onPrompt` payload (Codex supports multiple message roles).
- Blocking laws: `beforeCommand` returns `{ block: true, reason: "..." }` when needed.

### Gemini CLI specifics

- Installs a shell wrapper over `gemini` that sources `~/.cortex/adapter-gemini.env`, which sets `GEMINI_CLI_PRE_PROMPT_HOOK` to a shim script.
- The shim script pipes the prompt to our daemon and prepends the returned context bundle to the user message (Gemini CLI doesn't support a `system`-role prepend cleanly).
- Tool-call observation uses `@google/generative-ai`'s Node SDK experimental `onToolCall` callback, shipped as a small Node companion launched by the shim.
- No blocking path; every law is observational.

### Session correlation across tools

- Each adapter generates its own `session_id` — there is no cross-tool identifier.
- Nexus (spec 07) can still link via `Artifact` touches: if two sessions (one Claude Code, one Cursor) edit the same file within a time window, a graph traversal reveals both.
- We deliberately do **not** manufacture cross-session identity — too easy to merge unrelated work and poison retrieval.

### Per-tool redaction extras

Redaction is shared (one catalog in `cortex-adapters/common/redact/patterns.yaml`). Per-tool extras live in `cortex-adapters/<tool>/redact_overrides.yaml` and merge into the common catalog for that adapter only.

### Install framework

Shared:

- Writes hook scripts / registers plugins.
- Starts the daemon (systemd / launchd / Windows Service).
- Patches the tool's settings / plugin config idempotently (detects prior Cortex entries; leaves user entries alone).
- Creates the `~/.cortex/<tool>/` directory structure (sockets, WAL, logs).

Per-tool:

- Knows *where* the tool looks for hooks / plugins.
- Knows the exact idempotent patch needed to wire Cortex in.

Uninstall is the exact inverse, with the `--purge` flag deciding WAL/log retention.

### Failure modes

Identical to spec 10, plus:

| Failure                                              | Handling                                             |
|------------------------------------------------------|------------------------------------------------------|
| Tool auto-updates and rewrites its config (Cursor/Codex) | `cortex-adapters status` detects drift and re-patches on next run; warn the user |
| Gemini SDK version bump breaks `onToolCall`           | Degrade to prompt-only capture; metric `gemini.tool_observation_unavailable` |
| Cursor file watcher backlog > 10k events              | Coarsen debounce; warn; keep going                    |
| Multiple tools in the same session (e.g. user alt-tabs) | Each adapter emits with its own session_id; correlation happens in Nexus |

### Observability

Per-tool metrics namespace: `cortex.adapter.<tool>.*`, same shape as spec 10's `cortex.adapter.*`. Plus:

```
cortex.adapter.install.count          gauge, labels: tool
cortex.adapter.config_drift.detected  counter, labels: tool
cortex.adapter.cross_tool_overlap.sessions_within_5min counter
```

## Acceptance criteria

- [ ] `cortex-adapters/common/` compiles standalone; spec-10's claude-code adapter refactored on top of it without regression (entire spec-10 acceptance suite still passes).
- [ ] Cursor: install + real workspace session produces `turn.user`, `tool_call.edit_inferred`, and session-lifecycle events; `_cortex_context.md` appears before prompts and disappears on session close.
- [ ] Cursor: a critical law violation emitted for an inferred edit produces a reminder block in `_cortex_context.md` for the next prompt.
- [ ] Codex: install + scripted session produces `turn.user`, `tool_call.requested/completed`, and a sync-blocked tool call when a synthetic LAW-007 fires.
- [ ] Codex: pre-thinking block appears as a `system` message in the captured `onPrompt` event.
- [ ] Gemini: install + scripted prompt produces `turn.user` with prepended context; `onToolCall` events land as `tool_call.requested`; no blocking path invoked.
- [ ] `cortex-adapters install all` on a fresh machine installs all four tools idempotently; re-running is a no-op diff.
- [ ] `cortex-adapters status` table includes daemon status, last event timestamp, and config-drift flag for each tool.
- [ ] Tool auto-update drift: deliberately rewriting the Cursor config and running `status` flags drift; `install cursor` repatches.
- [ ] Per-tool overflow WAL works independently (no shared WAL file).
- [ ] Gemini SDK unavailable: adapter runs with prompt-only capture, metric `gemini.tool_observation_unavailable` bumped; no crashes.
- [ ] Three-adapter concurrent session (Claude Code + Cursor + Codex in the same repo) produces three distinct `Session` nodes; Nexus traversal surfaces all three when querying the shared Artifact.
- [ ] Telemetry counters non-zero after scripted soaks per tool.

## Decisions

1. **Common crate, thin adapter crates.** Diverging copy-paste would bit-rot. The common crate is the spec-10 reference promoted.
2. **No cross-tool session merging.** Different tools = different sessions. Graph traversal is the right place to unify, not the adapter.
3. **Cursor is file-watched, not hook-wired.** Cursor doesn't give us hooks; file-watching is the honest path, tagged `edit_inferred` so downstream can discount precision if needed.
4. **Gemini is observational-only.** Its CLI surface is too thin for sync blocking today. If Google ships a better hook API, we revisit.
5. **Pre-thinking injection is adapter-specific.** System message (Codex), user-message prepend (Gemini), rules file (Cursor). Same content; different plumbing.
6. **Install is idempotent, auto-repairing.** Tools update themselves; we can't break when they do.
7. **Single `cortex-adapters` binary for install/uninstall/status.** One entry point is less cognitive overhead than four.

## Open questions

1. **JetBrains IDE + VSCode.** Most-requested additions after this set. Probably a separate spec (they're plugin-based, not hook-based).
2. **Cross-tool trust scoring.** Spec 14 scores per `(model, repo)`; but a user's *preferred* tool for a repo is also signal. Worth a follow-up.
3. **Cursor rules-file pollution.** We write `_cortex_context.md` into the user's workspace. Some users may object — provide a `--no-workspace-write` flag that falls back to injecting via the shell completion prompt? Small, deferred.

## References

- Architecture §5.1 (capture layer).
- Spec 04 — Cortex Core.
- Spec 10 — Claude Code adapter (reference impl).
- Spec 11 — Query API (pre-thinking consumer).
- Spec 12 — Pre-thinking injection (shared module now in `common/`).
- Spec 13 — Laws DSL (detector contract for blocking path).
- Spec 14 — Governance engine (observational path).
- Cursor: https://docs.cursor.com (rules, chat JSONL layout).
- Codex CLI: https://openai.com/codex (plugin hook list).
- Gemini CLI: https://github.com/google-gemini/cli (pre-prompt hook; `onToolCall` experimental SDK).
