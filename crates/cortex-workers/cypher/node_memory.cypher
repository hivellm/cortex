UNWIND $rows AS row
CREATE (n:Memory { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
