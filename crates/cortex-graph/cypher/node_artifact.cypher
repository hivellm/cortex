UNWIND $rows AS row
MERGE (n:Artifact { natural_key: row.key })
SET n += row.props
