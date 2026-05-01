# 01 — Corpus inventory

Everything we can index, with sizes, schemas, and exemplar paths.

## 1. `C:\Users\Bolado\.claude\projects\` — primary corpus

**Total:** 9 835 `.jsonl` files / 4.5 GB on disk / 31 project directories.

Directory naming encodes the working directory the session ran from:
prefix `e--` → `E:\…`, `f--` → `F:\…`, `C--` → `C:\…`, with `\` flattened
to `-`. So `e--HiveLLM-Cortex` is the Cortex repo at `E:\HiveLLM\Cortex`.

### 1.1 Top-10 by session volume

| Project dir | Sessions (.jsonl) | Bytes | Notes |
|---|---:|---:|---|
| `e--HiveLLM-Cortex` | 798 | 268 MB | This repo (meta — heavily self-referencing) |
| `F--Node-hivellm-tml` | 46 | 1.4 GB | TML compiler work, very long sessions |
| `e--UzEngine` | 45 | 1.8 GB | Game engine; C++ refactor traffic |
| `e--HiveLLM-Nexus` | 32 | 192 MB | Nexus perf + Neo4j compat |
| `e--HiveLLM-Vectorizer` | 17 | 111 MB | Vectorizer replication / embeddings |
| `f--Node-project-v` | 9 | 276 MB | Feature dev + tests |
| `f--Node-hivellm-synap` | 6 | 118 MB | Graph/DB work |
| `e--HiveLLM-Rulebook` | 3 | 8 MB | Rulebook feature impls |
| `e--UzEngine-0-2` | 0 | 279 MB | Sub-agent worktree (no top-level JSONL) |
| `f--Node-hivellm-rulebook` | 2 | 42 MB | Rulebook hivellm fork |

Dormant siblings (≤2 sessions): 21 directories.

### 1.2 JSONL record types (7 disjoint shapes)

Each session file is newline-delimited JSON; one Claude Code session
produces ~hundreds of records mixing the types below. All fields are
stable across `version` 2.1.x.

| `type` | Discriminator | Carries | Maps to Cortex kind |
|---|---|---|---|
| `user` | `message.role == "user"` | prompt text, `cwd`, `gitBranch`, `permissionMode` | `Turn.user_message` |
| `assistant` | `message.role == "assistant"` | content array (`text` / `thinking` / `tool_use`), `model`, `usage` | `Turn.assistant_message` (text+thinking) **and** `ToolCall` per `tool_use` |
| `attachment` | `attachment.type == "hook_*"` / `tool_result` / `skill_listing` / `deferred_tools_delta` / `file-history-snapshot` | hook stdout/exit, tool result body, transient harness state | `ToolCall.output` (when `tool_result`); ignore other subtypes |
| `system` | `subtype == "local_command"` | local command stdout/stderr | `ToolCall` (synthesized, tool="local_command") |
| `file-history-snapshot` | own type | `messageId`, `trackedFileBackups`, `timestamp` | drop (transient) |
| `last-prompt` | own type | session-restore breadcrumb | drop |
| `queue-operation` | own type | enqueue/dequeue marker | drop |

**Common fields on every persisted record:**

```
sessionId   ULID-style UUID; stable per Claude Code session
uuid        per-record UUID (nullable parentUuid links into a tree)
parentUuid  message-tree edge — reconstruct conversation as DAG
timestamp   ISO-8601 UTC, ms precision
cwd         absolute working directory at record time
gitBranch   git branch at record time (empty when no repo)
version     "2.1.112", "2.1.120", … harness version
entrypoint  "claude-vscode" in this corpus
userType    "external" (user) — internal turns rare
isSidechain false on main convo, true for sub-agent inner work
```

**Assistant-message extras:**

```
message.model       claude-opus-4-7 | claude-sonnet-4-6 | claude-haiku-4-5
message.id          msg_… (Anthropic message id)
message.usage       input_tokens, cache_creation_input_tokens,
                    cache_read_input_tokens, output_tokens, service_tier
message.stop_reason end_turn | tool_use | max_tokens
requestId           req_… (correlation id with API logs)
```

### 1.3 Mapping to Cortex `Envelope`

Every meaningful JSONL record collapses to one of the existing
`Kind` variants in
[`crates/cortex-core/src/events.rs`](../../../crates/cortex-core/src/events.rs):

```text
user + assistant pair (matched by parentUuid) →
  Envelope {
    kind: Turn,
    tool: "claude-code",
    model: <assistant.model>,
    occurred_at: <user.timestamp>,
    session_id: <sessionId>,
    context: { repo: <slug_for(cwd)>, cwd, branch: gitBranch, … },
    payload: Turn {
      user_message,
      assistant_message: text_blocks + thinking_blocks,
      tokens: <usage>,
      tool_call_event_ids: [<emitted tool_call ids>],
    },
  }

assistant.tool_use[] →
  Envelope {
    kind: ToolCall,
    tool: <tool_name>,
    parent_event_id: <turn event_id>,
    payload: ToolCall {
      tool_name, input,
      output: <attachment.tool_result.body>,
      duration_ms, outcome,
    },
  }

assistant tool_use of subagent_type Agent →
  Envelope {
    kind: AgentCall,
    payload: AgentCall {
      agent_type, description, prompt, team_name,
      child_event_ids, result, outcome,
    },
  }
```

**No new `Kind` variants required for the conversation corpus.**
The schema absorbs it as-is.

### 1.4 Fixture paths (small / medium / large)

For unit tests + parser fuzzing:

| Size | Path | Lines | Why |
|---|---|---:|---|
| Tiny | `C:/Users/Bolado/.claude/projects/e--HiveLLM-Rulebook/ab2e4403-…05f.jsonl` | 12 | Full turn lifecycle in 12 records |
| Small | `C:/Users/Bolado/.claude/projects/e--HiveLLM-Vectorizer/<any>.jsonl` | ~hundreds | Single-session, mixed tool calls |
| Medium | `C:/Users/Bolado/.claude/projects/e--HiveLLM-Cortex/0059828f-…857.jsonl` | ~thousands | 1.4 MB; classifier+tool-heavy |
| Large | `C:/Users/Bolado/.claude/projects/e--UzEngine/<largest>.jsonl` | ~tens of thousands | 1.8 GB project, stress test the parser |

## 2. `C:\Users\Bolado\.claude\` — sidecar artifacts

| Path | Format | Bytes | Indexable? | Suggested Cortex kind |
|---|---|---:|---|---|
| `history.jsonl` | JSONL `{display, pastedContents, timestamp, project, sessionId}` | 660 KB | ✅ | `Turn.user_message` (synthesized; no assistant_message) |
| `todos/<uuid>-agent-<uuid>.json` | JSON array `[{content, status, activeForm}]` | ~1.6 MB across hundreds of files | ✅ | `Memory` (op=write, memory_type="todo") **or** `Artifact` (artifact_type="snippet") |
| `plans/*.md` | Markdown | 64 KB | ✅ | `Artifact` (artifact_type="snippet", language="markdown") |
| `sessions/<port>.json` | JSON | 9 KB | ⚠️ low value | Skip (VS Code state) |
| `settings.json` | JSON | 1.6 KB | ✅ once | `Memory` (memory_type="settings") |
| `mcp.json`, `mcp-needs-auth-cache.json` | JSON | 1.8 KB | ⚠️ contains creds — redact | `Memory` (memory_type="mcp_config") if redacted |
| `.credentials.json` | encrypted | 471 B | ❌ never | excluded |
| `shell-snapshots/snapshot-bash-*.sh` | bash | 7 MB across 1 600+ files | ⚠️ low value, high volume | Skip in v1; revisit if env diff matters |
| `cache/`, `debug/`, `telemetry/`, `backups/`, `downloads/`, `paste-cache/`, `file-history/`, `ide/`, `session-env/`, `plugins/`, `statsig/`, `stats-cache.json` | varies | varies | ❌ | exclude (ephemeral / harness-internal) |

### 2.1 `~/.codex/` parallel corpus

| Path | Format | Bytes | Status |
|---|---|---:|---|
| `~/.codex/history.jsonl` | JSONL | small | ✅ index alongside `~/.claude/history.jsonl`; tag `tool: "openai-codex"` |
| `~/.codex/sessions/` | unknown | small/empty | ✅ if non-empty, same Turn ingestion path with `tool: "openai-codex"` |
| `~/.codex/auth.json`, `config.toml`, `state_5.sqlite*` | misc | small | ❌ excluded (creds + state DB) |

## 3. Volume estimates after parsing

Rough budget for the full ingest, assuming 800 records/JSONL average,
30 % of records map to Turn or ToolCall envelopes (the rest are
`attachment` subtypes we drop or fold into ToolCall.output):

| Stage | Output | Estimate |
|---|---|---|
| JSONL parse | parsed lines | ~7.9 M records |
| Envelope emit | canonical events | ~2.4 M envelopes |
| Vectorizer chunks | chunked + embedded | ~6 M chunks (≈ 24 GB FP32 raw / 6 GB PQ) |
| Meili docs | indexed documents | ~2.4 M docs (≈ 1–2 GB index) |
| Nexus nodes / edges | graph | ~5 M nodes + ~12 M edges |

These numbers drive the Phase 1 sizing in
[05-implementation-plan.md](./05-implementation-plan.md). PQ-only
warm tier on Vectorizer is mandatory; FP32-only would not fit on the
dev machine.

## 4. PII / secret handling

The corpus contains:

- the user's email (`andrehr5315@gmail.com`)
- absolute file paths revealing local layout
- bearer tokens / API keys leaked into `tool_result` bodies (if the
  user pasted any during a session)
- repository contents the user later removed from disk

Every envelope MUST pass through `cortex_core::redact()` before
publish. We extend the redaction patterns (Phase 1 of the plan) to
cover Anthropic `sk-ant-…`, OpenAI `sk-…`, GitHub `ghp_…`, AWS
`AKIA…`, Google `AIza…`, and JWT-shaped strings; all already cheap
to add.

## 5. Update cadence

`~/.claude/projects/<project>/<session>.jsonl` is **append-only while
the session is live** and **immutable after Stop hook fires**.
Detection strategy:

1. **Backfill** — one-shot walker run from `cortex-claude-archive
   bootstrap`, writes a checkpoint after each session file.
2. **Live tail** — watcher (notify-rs) on `~/.claude/projects/`,
   incremental fsync-aware reader that emits new envelopes as they
   land. Sessions appear once per `claude` invocation; the watcher
   wakes per-file, not per-record.
3. **Resume** — checkpoint stores `(session_id, last_record_uuid)`;
   on restart we skip everything ≤ that uuid.

This is the same backfill-then-tail shape `cortex-bootstrap`
already uses for git history; we copy the pattern, not the code.
