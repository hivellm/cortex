UNWIND $rows AS row
CREATE (n:Law { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
