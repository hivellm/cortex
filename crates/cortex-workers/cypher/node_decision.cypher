UNWIND $rows AS row
MERGE (n:Decision { id: row.key })
SET n += row.props
