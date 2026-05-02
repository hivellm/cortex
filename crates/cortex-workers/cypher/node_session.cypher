UNWIND $rows AS row
CREATE (n:Session { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
