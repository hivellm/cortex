## 1. Meili helper
- [ ] 1.1 In `crates/cortex-fulltext/src/meili_client.rs`: `prune_document_body(index, batch: &[PruneOp]) -> Result<TaskUid>`
- [ ] 1.2 `PruneOp { event_id, summary_capped: String, pruned_at: String }` produces an `update_documents` payload
- [ ] 1.3 Helper `await_task_terminal(uid, timeout) -> TaskState` (uses Meili SDK `tasks.get`)

## 2. Pruner runner
- [ ] 2.1 NEW `crates/cortex-retention/src/meili_prune.rs`
- [ ] 2.2 `enumerate_prunable(index, now, after_days, batch_size)` queries Meili with `filter = "occurred_at < <cutoff> AND pruned != true"` and `limit = batch_size`
- [ ] 2.3 Per batch: build `PruneOp[]`, cap each `summary` to 4 KB, send to Meili, await terminal, record metrics
- [ ] 2.4 Stop on first hard error; partial batches before the failure are durable (Meili tasks are atomic)
- [ ] 2.5 Run across both `cortex_turns` and `cortex_tool_calls` per invocation

## 3. Idempotence
- [ ] 3.1 Pruned docs carry `pruned = true`; the enumerator excludes them, so re-running is a no-op
- [ ] 3.2 `--rebuild` flag re-prunes (useful if the cap policy changes); clears `pruned` first then re-applies

## 4. CLI / wiring
- [ ] 4.1 `cortex-retention meili-prune [--time-travel RFC3339] [--dry-run] [--index cortex_turns|cortex_tool_calls|all] [--rebuild]`
- [ ] 4.2 `cortex.toml [retention.meili]` (`prune_after_days = 90`, `summary_cap_bytes = 4096`, `batch = 1000`)
- [ ] 4.3 Advisory lock keyed `("meili-prune")`

## 5. Compatibility test
- [ ] 5.1 Integration test: ingest a turn → prune it → run a keyword query that previously matched the body → assert the query still ranks the doc on its `summary`
- [ ] 5.2 Negative test: a prune operation MUST NOT delete docs, only blank fields and set `pruned`

## 6. Spec / docs
- [ ] 6.1 Add §"Meili pruning" to `docs/specs/19-retention.md`
- [ ] 6.2 Update `docs/specs/08-fulltext-indexer.md` to reference the pruner contract

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
