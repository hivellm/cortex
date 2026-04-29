UNWIND $rows AS row
MERGE (a:ToolCall { id: row.from })
MERGE (b:Artifact { natural_key: row.to })
MERGE (a)-[r:TOUCHED]->(b)
SET r += row.props
