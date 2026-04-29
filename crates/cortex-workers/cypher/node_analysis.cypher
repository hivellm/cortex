UNWIND $rows AS row
MERGE (n:Analysis { id: row.key })
SET n += row.props
