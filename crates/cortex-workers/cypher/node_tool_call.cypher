UNWIND $rows AS row
CREATE (n:ToolCall { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
