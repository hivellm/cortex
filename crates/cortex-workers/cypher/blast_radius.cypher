// Phase11k §6.1 — blast_radius: walk IMPORTS_FILE 1..2 hops out
// from a touched artifact so the pre-thinking renderer can warn
// about downstream files an edit may break. Parameters:
//   $source_id — Nexus reserved `_id` of the touched :Artifact
//   $limit     — max downstream artifacts to return
MATCH (source:Artifact {_id: $source_id})-[:IMPORTS_FILE*1..2]->(downstream:Artifact)
WHERE downstream._id <> source._id
RETURN DISTINCT downstream._id AS downstream_id,
       downstream.repo AS repo,
       downstream.path AS path,
       downstream.content_hash AS content_hash
LIMIT $limit
