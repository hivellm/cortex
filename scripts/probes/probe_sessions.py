import duckdb, os
root = os.path.expanduser('~/.cortex/archive/events')
root = root.replace(chr(92), '/')
g = root + '/year=*/month=*/day=*/hour=*/raw-*.parquet'
con = duckdb.connect()
try:
    con.execute(f"SELECT DISTINCT session_id FROM read_parquet('{g}', union_by_name=true) WHERE session_id IS NOT NULL ORDER BY session_id")
    rows = con.fetchall()
    print(f'distinct session_ids with non-null: {len(rows)}')
    for r in rows[:20]:
        print(r[0])
    con.execute(f"SELECT COUNT(*), COUNT(session_id) FROM read_parquet('{g}', union_by_name=true)")
    print('total/with_session:', con.fetchone())
except Exception as e:
    print('error:', e)
    try:
        con.execute(f"DESCRIBE SELECT * FROM read_parquet('{g}', union_by_name=true) LIMIT 1")
        print('schema:', con.fetchall())
    except Exception as e2:
        print('schema-err:', e2)
