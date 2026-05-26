## 1. Implementation
- [x] 1.1 Add `indexed_repos()` snapshot helper to `MemoryKeywordLane`
- [x] 1.2 Wire the keyword-lane snapshot into `ApiState` and extend `StatusBody` with `indexed_repos`
- [x] 1.3 Add `Notice` struct to `QueryResponse` and stamp `repo_not_indexed` when scope misses
- [x] 1.4 Propagate the notice through the pre-thinking MCP shim as `reason: "repo_not_indexed"`
- [x] 1.5 Document first-time indexing path in `README.md`

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 2.1 Update or create documentation covering the implementation
- [x] 2.2 Write tests covering the new behavior
- [x] 2.3 Run tests and confirm they pass
