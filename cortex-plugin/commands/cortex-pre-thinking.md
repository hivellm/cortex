---
description: Manually trigger a pre-thinking bundle for the current turn (debug aid — the spec-10 hook does this automatically).
---

Call the `cortex.pre_thinking` MCP tool:

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

Render the bundle verbatim — it's already a deterministic
Markdown block with a leading `<!-- cortex: ... query_id=... -->`
comment. **Do not paraphrase or trim.** The whole point of this
slash command is for the user to see exactly what the spec-10
hook would inject.

If the bundle is empty (Cortex returned no relevant context),
render:

```text
Pre-thinking bundle was empty for this scope. Possible reasons:
  - cortex-api hasn't indexed this repo yet
  - the prompt is too generic (try a more specific verb)
  - the bundle's budget kicked in and stripped everything
```

If the tool returns `isError: true`, surface the `reason` as a
single line and stop.
