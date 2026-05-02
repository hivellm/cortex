// Phase11k §6.1 — code_callers: walk one CALLS hop into a target
// symbol so the pre-thinking renderer can answer "who calls X?"
// without an extra Synap round-trip. Parameters:
//   $target_id  — Nexus reserved `_id` of the called symbol
//   $limit      — max callers to return (default 25 enforced by caller)
MATCH (caller:Symbol)-[r:CALLS]->(callee:Symbol {_id: $target_id})
RETURN caller._id AS caller_id,
       caller.name AS caller_name,
       caller.qualified_name AS caller_qualified_name,
       r.tier AS tier,
       r.source_line AS source_line
LIMIT $limit
