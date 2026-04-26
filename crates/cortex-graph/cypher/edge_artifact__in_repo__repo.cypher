UNWIND $rows AS row
MERGE (a:Artifact { natural_key: row.from })
MERGE (b:Repo { name: row.to })
MERGE (a)-[r:IN_REPO]->(b)
SET r += row.props
