UNWIND $rows AS row
MERGE (a:Turn { id: row.from })
MERGE (b:Decision { id: row.to })
MERGE (a)-[r:LINKED_TO]->(b)
SET r += row.props
