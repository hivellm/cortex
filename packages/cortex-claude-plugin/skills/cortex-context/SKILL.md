---
name: cortex-context
description: Recall cited local context before repository work.
---

# cortex-context

Call `cortex_pre_thinking` with the actual absolute `cwd` and a concise `task`, or `cortex_memory_read` with the exact repository ID and `query`. Read the repository ID from `[cortex] id` in cortex.toml. Never guess it. Context is bounded; use `cortex_search` and `cortex_get` for omitted sources. Preserve authority, ID, revision and source when reporting. Delivery does not prove model use. No legacy code-index or session lookup is available.
