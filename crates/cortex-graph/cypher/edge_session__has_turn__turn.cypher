UNWIND $rows AS row
MERGE (a:Session { id: row.from })
MERGE (b:Turn { id: row.to })
MERGE (a)-[r:HAS_TURN]->(b)
SET r += row.props
