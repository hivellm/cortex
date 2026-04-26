UNWIND $rows AS row
MERGE (n:LawViolation { id: row.key })
SET n += row.props
