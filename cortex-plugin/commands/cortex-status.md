---
description: Show Cortex daemon health (pid, queue depth, recent publisher errors, overflow WAL bytes).
---

Call the `cortex_status` MCP tool with no arguments and render
the response as a compact dashboard:

```text
Cortex daemon
  pid:           <pid>
  uptime:        <hh:mm:ss>
  queue depth:   <n> / <queue_bounded>
  WAL bytes:     <bytes>
  publisher errors (last 5):
    - <ts> <status> <detail>
    ...
```

If `cortex_status` returns `isError: true` with
`reason: api_unreachable`, surface `Cortex daemon is offline.
Run `cortex-adapter-claude status` for diagnostics.` and stop.
