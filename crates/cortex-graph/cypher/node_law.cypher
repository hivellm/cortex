UNWIND $rows AS row
MERGE (n:Law { id: row.key })
SET n += row.props
