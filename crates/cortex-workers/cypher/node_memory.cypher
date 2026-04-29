UNWIND $rows AS row
MERGE (n:Memory { id: row.key })
SET n += row.props
