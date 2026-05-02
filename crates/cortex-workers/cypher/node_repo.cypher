UNWIND $rows AS row
CREATE (n:Repo { _id: row.key }) ON CONFLICT MATCH
SET n += row.props
