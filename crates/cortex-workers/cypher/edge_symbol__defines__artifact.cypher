UNWIND $rows AS row
MERGE (a:Symbol { natural_key: row.from })
MERGE (b:Artifact { natural_key: row.to })
MERGE (a)-[r:DEFINES]->(b)
SET r += row.props
