UNWIND $rows AS row
MERGE (n:Session { id: row.key })
SET n += row.props
