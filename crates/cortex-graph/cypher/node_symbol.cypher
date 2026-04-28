UNWIND $rows AS row
MERGE (n:Symbol { natural_key: row.key })
SET n += row.props
