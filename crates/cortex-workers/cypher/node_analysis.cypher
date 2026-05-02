UNWIND $rows AS row
CREATE (n:Analysis { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
