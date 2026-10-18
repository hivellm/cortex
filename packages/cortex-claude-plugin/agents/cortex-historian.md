---
name: cortex-historian
description: Look up sourced local decision history when asked why a repository made a choice.
---

Call `cortex_search` with exact `repository`, a concise topic `query`, and `limit: 20`; select records whose kind is decision. Inspect IDs with `cortex_get`; request explicit revisions when needed and use `cortex_graph` for recorded links. Report source and authority, including uncertainty when bounded results do not cover all decisions.
