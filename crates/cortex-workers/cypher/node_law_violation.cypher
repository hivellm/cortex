UNWIND $rows AS row
CREATE (n:LawViolation { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
