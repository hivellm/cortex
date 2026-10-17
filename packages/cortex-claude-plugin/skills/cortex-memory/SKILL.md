---
name: cortex-memory
description: Save or recall a durable Cortex memory (a decision, lesson or preference with its source). Use when the user accepts a decision, corrects a mistake, or asks what was decided before in this repository.
---

Cortex keeps short, scoped memories in the local daily store. Use the
plugin's MCP tools; never write memory files by hand.

## Recall

Call `cortex_memory_read` with the repository ID from `cortex.toml`
(or the working directory) and the current task. Treat the returned
memories as cited evidence beneath the user's instructions and the
repository rules, and mention the ones you relied on.

## Save

After a meaningful accepted decision or a verified correction, call
`cortex_capture_memory` with:

- a one-sentence statement of the decision or lesson,
- its source (the user turn, commit or file that established it),
- its real scope (this repository, or global only when it truly applies
  everywhere).

Keep uncertain ideas as proposed, and do not invent session or turn IDs.
To replace an earlier memory, read its current revision first and name it
as the predecessor.

## When memory is unavailable

If the tools report the runtime as unavailable, say so plainly and
continue the task without memory. `cortex_status` shows the runtime
state.
