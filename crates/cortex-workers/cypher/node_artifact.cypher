UNWIND $rows AS row
CREATE (n:Artifact { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
