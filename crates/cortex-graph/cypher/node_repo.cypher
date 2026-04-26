UNWIND $rows AS row
MERGE (n:Repo { name: row.key })
SET n += row.props
