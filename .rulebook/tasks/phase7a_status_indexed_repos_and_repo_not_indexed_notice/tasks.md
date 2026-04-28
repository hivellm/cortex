## 1. Implementation
- [ ] 1.1 Add `indexed_repos()` snapshot helper to `MemoryKeywordLane`
- [ ] 1.2 Wire the keyword-lane snapshot into `ApiState` and extend `StatusBody` with `indexed_repos`
- [ ] 1.3 Add `Notice` struct to `QueryResponse` and stamp `repo_not_indexed` when scope misses
- [ ] 1.4 Propagate the notice through the pre-thinking MCP shim as `reason: "repo_not_indexed"`
- [ ] 1.5 Document first-time indexing path in `README.md`

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
