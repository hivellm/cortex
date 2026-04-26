UNWIND $rows AS row
MERGE (a:LawViolation { id: row.from })
MERGE (b:Law { id: row.to })
MERGE (a)-[r:OF]->(b)
SET r += row.props
