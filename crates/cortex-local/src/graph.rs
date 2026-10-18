//! Bounded breadth-first traversal; repository scope is never widened.
use crate::{files, index, model::*};
use rusqlite::{params, Connection};
use std::collections::{HashSet, VecDeque};

pub(crate) fn query(connection: &Connection, query: GraphQuery) -> Result<GraphResult> {
    files::validate_scope(&query.repository)?;
    files::validate_id(&query.id)?;
    if query.depth > MAX_GRAPH_DEPTH || query.limit == 0 || query.limit > MAX_GRAPH_RESULTS {
        return Err(LocalError::Invalid(
            "graph depth must be 0..5, result limit 1..200".into(),
        ));
    }
    let Some(start) = index::get(connection, &query.repository, &query.id, None)? else {
        return Ok(GraphResult::default());
    };
    if !start.recall_eligible() {
        return Ok(GraphResult::default());
    }
    let mut result = GraphResult {
        records: vec![start],
        ..Default::default()
    };
    let mut seen = HashSet::from([query.id.clone()]);
    let mut queue = VecDeque::from([(query.id, 0)]);
    while let Some((id, depth)) = queue.pop_front() {
        if depth >= query.depth {
            continue;
        }
        let mut statement = connection.prepare(
            "SELECT source_id,target_id,relation FROM edges e
             JOIN current_records s ON s.repository=e.repository AND s.id=e.source_id AND s.active=1
             JOIN current_records t ON t.repository=e.repository AND t.id=e.target_id AND t.active=1
             WHERE e.repository=?1 AND (e.source_id=?2 OR e.target_id=?2)
             ORDER BY source_id,target_id,relation LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![query.repository, id, MAX_GRAPH_RESULTS as i64 + 1],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        for (position, row) in rows.enumerate() {
            if position >= MAX_GRAPH_RESULTS || result.edges.len() >= MAX_GRAPH_RESULTS {
                result.truncated = true;
                break;
            }
            let (source_id, target_id, relation) = row?;
            let neighbor = if source_id == id {
                &target_id
            } else {
                &source_id
            };
            if !seen.contains(neighbor) {
                let Some(record) = index::get(connection, &query.repository, neighbor, None)?
                else {
                    continue;
                };
                if !record.recall_eligible() {
                    continue;
                }
                if result.records.len() >= query.limit {
                    result.truncated = true;
                    continue;
                }
                result.records.push(record);
                seen.insert(neighbor.clone());
                queue.push_back((neighbor.clone(), depth + 1));
            }
            let edge = GraphEdge {
                source_id,
                target_id,
                relation: serde_json::from_str(&relation)?,
            };
            if !result.edges.contains(&edge) {
                result.edges.push(edge);
            }
        }
    }
    Ok(result)
}
