# Extraction contract gate: confidence lives in EdgeOp.props, not a struct field
**Source**: manual
**Date**: 2026-06-24
**Related Task**: phase23c_ua-extraction-contract
**Tags**: graph, extraction-contract, reconciliation, hallucination, confidence
## Phase23c extraction contract — implementation notes

**EdgeConfidence is in props, not a struct field.** `EdgeOp` does NOT have a `confidence: EdgeConfidence` field. `with_confidence()` writes `props["confidence"] = "extracted"|"inferred"|"ambiguous"`. To read it back: `edge.props.get("confidence").and_then(|v| v.as_str())`.

**NodeOp has no confidence concept.** Gate code can't use "is this node Extracted?" for nodes. Instead: check if the node's label is a code label (Symbol/Artifact/ExternalPackage/UnresolvedImport) AND if its natural-key is in the FactSet. If FactSet is empty (non-code context), skip the check entirely.

**read_confidence_node trap**: an early version used the label to infer "confidence" for nodes (Symbol → Extracted, others → Inferred). This was wrong — it made the gate skip hallucinated Symbol nodes from Phase 2. The correct approach: check ALL code-label nodes against the fact set when `!facts.is_empty()`.

**FactSet.is_empty() guards the gate.** For Turn/Decision/Topic events where no static analysis ran, FactSet is empty. Passing empty FactSet to apply_gate means the endpoint check fires for 0 facts, rejecting all edges. The `!facts.is_empty()` guard prevents this — non-code events pass through unchanged.

**Significance filter uses NodeOp.props.** `props["line_count"]` (u64) and `props["exported"]` (bool). If `line_count` is absent → keep (conservative). Only drop when line_count IS known AND < threshold AND not exported. Tree-sitter analyzers don't currently set these; filter only fires in tests or when we add line tracking to the analyzers.

**Import edge types**: `"IMPORTS"` (classifier, Inferred) and `"IMPORTS_FILE"` (tree-sitter, Extracted) are both import edges. Use `is_import_edge()` to group them.

**Clippy: collapsible-if.** Two nested `if` blocks → clippy `-D warnings` fails. Collapse to `if A && B { ... }` form.
