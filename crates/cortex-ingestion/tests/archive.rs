//! Integration tests for `cortex_ingestion::archive`.

use cortex_ingestion::archive::{read_archive_file, InMemoryArchive, NdJsonZstdArchive};
use cortex_ingestion::ArchiveWriter;
use serde_json::{json, Value};
use tempfile::TempDir;

fn envelope() -> Value {
    json!({
        "event_id": "01HXYZABCDEF0123456789ABCD",
        "schema_version": "1",
        "occurred_at": "2026-04-17T12:34:56.789Z",
        "session_id": "01HXYZABCDEF0123456789ABCE",
        "stream": "live",
        "tool": "claude-code",
        "kind": "turn",
        "context": { "platform": "linux" },
        "payload": { "user_message": "hi" },
        "content_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    })
}

#[test]
fn ndjson_zst_round_trip() {
    let tmp = TempDir::new().unwrap();
    let archive = NdJsonZstdArchive::new(tmp.path().to_path_buf(), 6);
    archive.write("raw", &envelope()).unwrap();
    archive.write("raw", &envelope()).unwrap();
    archive.flush().unwrap();
    let paths = archive.open_paths();
    assert_eq!(paths.len(), 1);
    drop(archive);
    let read = read_archive_file(&paths[0]).unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0]["event_id"], "01HXYZABCDEF0123456789ABCD");
    assert!(read[0].get("_archived_at").is_some());
}

#[test]
fn separates_streams() {
    let tmp = TempDir::new().unwrap();
    let archive = NdJsonZstdArchive::new(tmp.path().to_path_buf(), 6);
    archive.write("raw", &envelope()).unwrap();
    archive.write("bootstrap", &envelope()).unwrap();
    archive.flush().unwrap();
    assert_eq!(archive.open_paths().len(), 2);
}

#[test]
fn in_memory_archive_captures_rows() {
    let a = InMemoryArchive::default();
    a.write("raw", &envelope()).unwrap();
    assert_eq!(a.rows().len(), 1);
}
