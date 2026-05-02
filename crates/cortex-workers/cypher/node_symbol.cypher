UNWIND $rows AS row
CREATE (n:Symbol { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
