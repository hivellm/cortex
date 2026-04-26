UNWIND $rows AS row
MERGE (a:LawViolation { id: row.from })
MERGE (b:ToolCall { id: row.to })
MERGE (a)-[r:OBSERVED_IN]->(b)
SET r += row.props
