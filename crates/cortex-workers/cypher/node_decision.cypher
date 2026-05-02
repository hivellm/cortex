UNWIND $rows AS row
CREATE (n:Decision { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
