---
description: Search local Cortex records.
argument-hint: <topic>
---

Call `cortex_query` with `repository` from `[cortex] id` in cortex.toml, `query` from the requested topic, and `limit: 10`. Only literal local FTS is implemented. Report matching record IDs, revisions, authority, title and source. Do not promise vector, code-index or session results.
