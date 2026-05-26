# Cypher UNWIND-write and $param-write substitution silently drop in Nexus 1.15

**Category**: code
**Tags**: nexus, cypher, unwind, merge, write-substitution, cortex-graph, silent-failure

## Description

Nexus 1.15.0 parses but does not persist any write that touches an UNWIND row, or that uses `$param` substitution inside a write clause (MERGE pattern, SET assignment). The HTTP call returns 200, `rows = []`, and no error — total silent failure. Reads with `$param` work fine; only the write substitution path is broken. cortex-graph hit this for ~1500 events: writer logged `nodes_upserted=271` per batch, but `MATCH (n)` returned 1. Workaround: render every write as a per-row Cypher statement with values escaped into the string literal (no `$param` on the write side), and use `result.rows.is_empty()` as the silent-drop signal (successful writes return `[[null]]`; dropped writes return `[]`).

## Example

// BAD: silently dropped by Nexus 1.15
//   UNWIND $rows AS row MERGE (n:Label { key: row.k }) SET n += row.props
//   MERGE (n:Label { key: $k }) SET n += $props

// GOOD: escape values into the literal, one statement per row
fn render_node_merge(node: &NodeOp) -> String {
    let key = serde_json::to_string(&node.natural_key).unwrap();
    let mut cy = format!("MERGE (n:{} {{ natural_key: {} }})", node.label, key);
    if !node.props.is_empty() {
        cy.push_str(" SET ");
        let parts: Vec<String> = node.props.iter()
            .map(|(k, v)| format!("n.{k} = {}", serde_json::to_string(v).unwrap()))
            .collect();
        cy.push_str(&parts.join(", "));
    }
    cy.push_str(" RETURN n");
    cy
}

// And: don't trust RETURN values for writes — Nexus returns [[null]]
// for successful writes. Use rows.is_empty() instead:
if result.rows.is_empty() {
    return Err("write silently dropped by Nexus");
}

## When to Use

Any code that writes to Nexus 1.15.x. Until Nexus ships a fix for UNWIND-write and write-side $param substitution, every MERGE / CREATE / SET must inline its values into the Cypher string.

## When NOT to Use

Reads. UNWIND with RETURN-only patterns and reads with $params work fine. The bug is specifically write-side substitution.
