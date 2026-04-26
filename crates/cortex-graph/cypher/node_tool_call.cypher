UNWIND $rows AS row
MERGE (n:ToolCall { id: row.key })
SET n += row.props
