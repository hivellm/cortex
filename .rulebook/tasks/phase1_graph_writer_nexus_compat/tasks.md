## 1. Diagnosis (already captured in proposal.md)
- [x] 1.1 Reproduce empty Nexus + writer-claims-success against the live stack
- [x] 1.2 Identify which Cypher shapes Nexus 1.15 actually persists vs silently drops
- [x] 1.3 Decide on the per-row interpolated-literals fix path

## 2. Cypher rendering helpers
- [ ] 2.1 Add `cypher_str_escape(&str)` to crates/cortex-graph/src/cypher.rs (handles `\`, `'`, control chars; refuses raw newlines inside literals)
- [ ] 2.2 Add `render_node_merge(label, key_field, key, props)` that returns a complete Cypher statement string with all values inline-escaped
- [ ] 2.3 Add `render_edge_merge(from_label, from_key_field, from_key, edge_type, to_label, to_key_field, to_key, props)` doing the same for edges
- [ ] 2.4 Unit tests: every Value variant round-trips through the renderer without breaking out of the literal

## 3. Writer rewrite
- [ ] 3.1 Replace `run_write_tx` in nexus_client.rs to call render_node_merge per node and render_edge_merge per edge
- [ ] 3.2 Each rendered statement ends with `RETURN count(*) AS written` so the writer can assert >0
- [ ] 3.3 Treat `written == 0` as `GraphClientError::Other(...)` so the worker emits a real failure instead of a silent success
- [ ] 3.4 Drop the `templates: &CypherTemplates` parameter from the live path; keep the registry available for read queries / future tasks

## 4. Cypher template files
- [ ] 4.1 Mark the existing `.cypher` files as deprecated (or move to `cypher/_legacy/`) until a Nexus version that supports UNWIND-write ships
- [ ] 4.2 Add a top-of-file note in cypher/README.md explaining why the runtime no longer reads them

## 5. Tests
- [ ] 5.1 Update unit tests in nexus_client.rs / writer.rs to assert the new per-row Cypher strings
- [ ] 5.2 Update cortex-graph integration test (`tests/worker.rs`, `tests/mapper.rs`) to expect per-row writes
- [ ] 5.3 Add an integration probe (gated on a live-Nexus env flag) that asserts a node + edge actually round-trip

## 6. End-to-end
- [ ] 6.1 Restart cortex-graph-worker against the live stack
- [ ] 6.2 Re-run cortex-bootstrap on the Cortex repo
- [ ] 6.3 Verify `MATCH (n) RETURN labels(n), count(n)` shows non-empty per-label rows
- [ ] 6.4 Verify `MATCH ()-[r]->() RETURN type(r), count(r)` shows expected edge counts

## 7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 7.1 Update or create documentation covering the implementation (docs/specs/07-graph-writer.md §Compat note + cortex-graph README)
- [ ] 7.2 Write tests covering the new behavior
- [ ] 7.3 Run tests and confirm they pass
