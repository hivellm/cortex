## 1. Durable audit trail
- [ ] 1.1 Persist, for every `cortex_pre_thinking` call: `query_id`, a summary of the assembled bundle's sections/content (extend `count_sections()` in `crates/cortex-pre-thinking/src/formatter.rs`, which already returns non-zero per-section counts — the durable record needs at least this plus enough per-item identifiers to be useful), and the timestamp
- [ ] 1.2 Where determinable from the session's subsequent turns (tool calls, file edits, decisions recorded), link the `query_id` to the downstream action(s) that followed it in the same session — a best-effort heuristic (e.g. "next N tool calls in this session after this query_id") is an acceptable first version
- [ ] 1.3 Expose the persisted trail in a new or extended dashboard view so an operator can browse "what did Cortex tell the agent, and what did the agent do next?"
- [ ] 1.4 Define and compute a "bundle utilization" heuristic (did any file/decision/law cited in the bundle get referenced again in the following turns?) and surface it as a metric

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 2.1 Update or create documentation covering the implementation — specifically `docs/specs/12-pre-thinking-injection.md`, describing the durable trail contract (retention beyond the in-memory ring buffer) and the new dashboard view
- [ ] 2.2 Write tests covering the new behavior (persistence beyond 1024 entries, query_id lookup, downstream-turn linking heuristic, utilization metric computation)
- [ ] 2.3 Run tests and confirm they pass
