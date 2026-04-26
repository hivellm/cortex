UNWIND $rows AS row
MERGE (a:Decision { id: row.from })
MERGE (b:Decision { id: row.to })
MERGE (a)-[r:SUPERSEDES]->(b)
SET r += row.props
