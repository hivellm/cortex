---
name: cortex-context
description: Refresh the system-prompt context bundle for the current task. Pulls active laws, recent decisions, similar past turns, and relevant snippets via the Cortex pre-thinking pipeline. Use this when the user asks to "load context", "remind me what we know about X", "before I change Y", or whenever the conversation needs grounding against Cortex's institutional memory.
---

# cortex-context

Use this skill when the user is about to make a change to code or
docs and you want focused context — laws that apply, decisions that
were made, similar turns from past sessions — without spamming the
prompt.

## When to use

- The user asks to refactor, modify, rewrite, or change something.
- The user asks "why did we pick X?" or "what laws apply here?".
- The user is starting a new task and you have low confidence about
  the scope.
- The user says "before you do that, remind yourself what we know".

Do NOT use this skill for:
- General free-text search — use `/cortex-query` or the
  `cortex.query` tool with `intent=free_search` instead.
- Auditing a past turn — use the `cortex-audit` skill.

## How to invoke

Call the `cortex.pre_thinking` MCP tool with:

```jsonc
{
  "user_prompt": "<the user's most recent prompt verbatim>",
  "cwd": "<absolute path to the user's current working directory>",
  "session_id": "<the active Claude Code session id>",
  "turn_id": "<the active turn id>",
  "budget_bytes": 32768,
  "budget_ms": 600
}
```

Always pass `cwd` — the pipeline derives the repo from the
nearest `.git/` ancestor + a per-repo `cortex.toml` override.
Without `cwd` you get repo-less results that are markedly worse.

## How to use the response

The tool returns a Markdown bundle with a fixed section order:

1. **Active laws in this scope** — load-bearing; mention every one
   the model is about to violate.
2. **Recent decisions you should know about** — cite the
   `DEC-XXXX` ids verbatim when summarising.
3. **Similar past turns** — paraphrase, do not re-quote at length.
4. **Relevant snippets** — only mention if the user is touching
   the same file.

The leading comment carries `query_id=<ULID>`; surface it when the
user asks "where did this context come from?" so they can audit
the retrieval against `cortex.events.query_audit`.

## Empty bundle

If the bundle is empty (no laws, decisions, turns, or snippets
matched), the tool returns the empty string. **Do not invent
context.** Tell the user "Cortex returned no relevant context for
this scope" and proceed with whatever the user asked.

## Failure modes

- Tool returns `isError: true` with `reason: api_unreachable` →
  `cortex-api` is down. Note it once and continue without the
  bundle; the session is not blocked.
- Tool returns `reason: rate_limited` → wait the suggested
  `retry_after_ms` before retrying.
