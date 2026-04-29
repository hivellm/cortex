//! Integration tests for `cortex_workers::embedder::identity`.

use cortex_workers::embedder::dedup_key;

#[test]
fn dedup_key_is_deterministic() {
    let a = dedup_key("evt_1", 0, "abc");
    let b = dedup_key("evt_1", 0, "abc");
    assert_eq!(a, b);
}

#[test]
fn dedup_key_differs_on_ordinal() {
    let a = dedup_key("evt_1", 0, "abc");
    let b = dedup_key("evt_1", 1, "abc");
    assert_ne!(a, b);
}
