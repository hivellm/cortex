UNWIND $rows AS row
MERGE (a:Session { id: row.from })
MERGE (b:ToolCall { id: row.to })
MERGE (a)-[r:HAS_TOOL_CALL]->(b)
SET r += row.props
