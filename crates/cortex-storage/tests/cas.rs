//! Integration tests for `cortex_storage::cas`.

use chrono::Utc;
use cortex_storage::{CasContentType, CasError, CasStore};
use rusqlite::params;

#[test]
fn put_and_get_round_trip() {
    let store = CasStore::open_in_memory().unwrap();
    let body = b"hello, cortex";
    let hash = store.put(body, CasContentType::Text).unwrap();
    let blob = store.get(&hash).unwrap();
    assert_eq!(blob.bytes, body);
    assert_eq!(blob.size, body.len() as u64);
    assert_eq!(blob.content_type, CasContentType::Text);
}

#[test]
fn put_is_idempotent_on_hash() {
    let store = CasStore::open_in_memory().unwrap();
    let body = b"same body";
    let h1 = store.put(body, CasContentType::Text).unwrap();
    let h2 = store.put(body, CasContentType::Text).unwrap();
    assert_eq!(h1, h2);
    let count: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM cas_blobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn refcount_lifecycle() {
    let store = CasStore::open_in_memory().unwrap();
    let h = store.put(b"refd", CasContentType::Binary).unwrap();
    assert_eq!(store.refcount(&h).unwrap(), 0);
    store.retain(&h).unwrap();
    store.retain(&h).unwrap();
    assert_eq!(store.refcount(&h).unwrap(), 2);
    store.release(&h).unwrap();
    assert_eq!(store.refcount(&h).unwrap(), 1);
    store.release(&h).unwrap();
    store.release(&h).unwrap();
    assert_eq!(store.refcount(&h).unwrap(), 0);
}

#[test]
fn vacuum_drops_only_expired_unreferenced() {
    let store = CasStore::open_in_memory().unwrap();
    let h = store.put(b"old", CasContentType::Binary).unwrap();
    store
        .conn()
        .execute(
            "UPDATE cas_blobs SET last_referenced = ?1 WHERE hash = ?2",
            params!["2000-01-01T00:00:00+00:00", h],
        )
        .unwrap();
    let dropped = store.vacuum(Utc::now()).unwrap();
    assert_eq!(dropped, 1);
    assert!(!store.contains(&h).unwrap());
}

#[test]
fn referenced_blob_survives_vacuum() {
    let store = CasStore::open_in_memory().unwrap();
    let h = store.put(b"keep", CasContentType::Binary).unwrap();
    store.retain(&h).unwrap();
    let dropped = store.vacuum(Utc::now()).unwrap();
    assert_eq!(dropped, 0);
    assert!(store.contains(&h).unwrap());
}

#[test]
fn get_missing_is_not_found() {
    let store = CasStore::open_in_memory().unwrap();
    let err = store.get("sha256:deadbeef").unwrap_err();
    matches!(err, CasError::NotFound(_));
}

#[test]
fn compression_actually_compresses_repetitive_input() {
    let store = CasStore::open_in_memory().unwrap();
    let body = vec![b'x'; 4096];
    let h = store.put(&body, CasContentType::Binary).unwrap();
    let stored: Vec<u8> = store
        .conn()
        .query_row(
            "SELECT blob FROM cas_blobs WHERE hash = ?1",
            params![h],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stored.len() < body.len() / 4);
}
