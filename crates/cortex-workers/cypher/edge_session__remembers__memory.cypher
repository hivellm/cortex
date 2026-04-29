UNWIND $rows AS row
MERGE (a:Session { id: row.from })
MERGE (b:Memory { id: row.to })
MERGE (a)-[r:REMEMBERS]->(b)
SET r += row.props
