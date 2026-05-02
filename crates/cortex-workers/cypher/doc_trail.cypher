// Phase11k §6.1 — doc_trail: walks the CITES chain rooted at an
// ADR / Decision / Analysis so the pre-thinking renderer can show
// the design ancestry behind a target node. Parameters:
//   $root_id  — Nexus reserved `_id` of the root node
//   $depth    — max chain depth (default 4 enforced by caller)
//   $limit    — max paths to return
MATCH path = (root {_id: $root_id})-[:CITES*1..4]->(target)
WHERE all(n IN nodes(path) WHERE n._id IS NOT NULL)
RETURN [n IN nodes(path) | n._id] AS path_ids,
       [n IN nodes(path) | labels(n)[0]] AS path_labels,
       length(path) AS path_length
ORDER BY path_length ASC
LIMIT $limit
