//! Phase11p §4.1 — live-session IT.
//!
//! Builds a temp parquet archive (zstd-NDJSON in the canonical
//! `events/year=YYYY/month=MM/day=DD/hour=HH/raw-NNNNN.parquet`
//! layout), seeds 30 envelopes (10 user-side Turn + 10 paired-side
//! Turn + 10 ToolCall) under one synthetic `session_id`, runs
//! `LiveSessionSource::fetch` and asserts the returned
//! `SessionInput` shape:
//!
//! - `session_id` matches the seed,
//! - `envelopes.len() == 30`,
//! - every seeded `event_id` is reachable through
//!   `input.envelopes.iter()`,
//! - the envelopes are sorted by `occurred_at`.
//!
//! Gated on `CORTEX_CONSOLIDATOR_LIVE_IT=1`. Default `cargo test`
//! returns early so the suite stays fast on CI.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;

use cortex_core::events::{Context, Envelope, Kind, Stream, ToolCall, Turn};
use cortex_workers::consolidator::source::LiveSessionSource;

fn it_enabled() -> bool {
    std::env::var("CORTEX_CONSOLIDATOR_LIVE_IT").as_deref() == Ok("1")
}

fn ctx() -> Context {
    Context {
        repo: Some("cortex".into()),
        branch: None,
        commit: None,
        cwd: None,
        user: None,
        platform: "linux".into(),
        ide: None,
        extras: BTreeMap::new(),
    }
}

fn turn_user(event_id: &str, session_id: &str, ts: &str, idx: u8) -> Envelope {
    Envelope {
        event_id: event_id.into(),
        schema_version: "1".into(),
        occurred_at: ts.into(),
        ingested_at: None,
        session_id: session_id.into(),
        stream: Stream::Live,
        tool: "claude-code".into(),
        model: None,
        kind: Kind::Turn,
        context: ctx(),
        payload: serde_json::to_value(Turn {
            user_message: format!("user msg {idx}"),
            assistant_message: None,
            tokens: None,
            tool_call_event_ids: vec![],
        })
        .unwrap(),
        redactions: vec![],
        content_hash: format!("sha256:{:0>64}", idx),
        parent_event_id: None,
    }
}

fn turn_paired(event_id: &str, session_id: &str, ts: &str, idx: u8) -> Envelope {
    Envelope {
        event_id: event_id.into(),
        schema_version: "1".into(),
        occurred_at: ts.into(),
        ingested_at: None,
        session_id: session_id.into(),
        stream: Stream::Live,
        tool: "claude-code".into(),
        model: None,
        kind: Kind::Turn,
        context: ctx(),
        payload: serde_json::to_value(Turn {
            user_message: format!("user msg {idx}"),
            assistant_message: Some(format!("reply {idx}")),
            tokens: None,
            tool_call_event_ids: vec![],
        })
        .unwrap(),
        redactions: vec![],
        content_hash: format!("sha256:{:0>64}", idx + 100),
        parent_event_id: None,
    }
}

fn tool_call(event_id: &str, session_id: &str, ts: &str, idx: u8) -> Envelope {
    Envelope {
        event_id: event_id.into(),
        schema_version: "1".into(),
        occurred_at: ts.into(),
        ingested_at: None,
        session_id: session_id.into(),
        stream: Stream::Live,
        tool: "claude-code".into(),
        model: None,
        kind: Kind::ToolCall,
        context: ctx(),
        payload: serde_json::to_value(ToolCall {
            tool_name: "Bash".into(),
            input: serde_json::json!({"command": format!("ls /tmp/{idx}")}),
            output: None,
            duration_ms: Some(10),
            touched: vec![],
            outcome: "success".into(),
        })
        .unwrap(),
        redactions: vec![],
        content_hash: format!("sha256:{:0>64}", idx + 200),
        parent_event_id: None,
    }
}

fn write_archive(root: &Path, envelopes: &[Envelope]) {
    let dir = root.join("events/year=2026/month=04/day=26/hour=19");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("raw-00000.parquet");
    let file = File::create(&path).unwrap();
    let mut enc = zstd::stream::write::Encoder::new(file, 3).unwrap();
    for env in envelopes {
        let line = serde_json::to_string(env).unwrap();
        enc.write_all(line.as_bytes()).unwrap();
        enc.write_all(b"\n").unwrap();
    }
    enc.finish().unwrap();
}

#[test]
fn live_session_source_returns_full_30_envelope_set() {
    if !it_enabled() {
        eprintln!("skipping consolidator_live_session_it (CORTEX_CONSOLIDATOR_LIVE_IT != 1)");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let session_id = "01HXSESSPHASE11PIT0000000000";

    let mut envelopes: Vec<Envelope> = Vec::with_capacity(30);
    let mut expected_ids: HashSet<String> = HashSet::new();
    for i in 0..10 {
        let id = format!("E_TURN_USR_{i:02}");
        envelopes.push(turn_user(&id, session_id, &format!("2026-04-26T19:{:02}:00Z", i), i));
        expected_ids.insert(id);
    }
    for i in 0..10 {
        let id = format!("E_TURN_PAIR_{i:02}");
        envelopes.push(turn_paired(
            &id,
            session_id,
            &format!("2026-04-26T19:{:02}:00Z", 10 + i),
            i,
        ));
        expected_ids.insert(id);
    }
    for i in 0..10 {
        let id = format!("E_TOOLCALL_{i:02}");
        envelopes.push(tool_call(
            &id,
            session_id,
            &format!("2026-04-26T19:{:02}:00Z", 20 + i),
            i,
        ));
        expected_ids.insert(id);
    }
    write_archive(dir.path(), &envelopes);

    let source = LiveSessionSource::new(dir.path());
    let input = source.fetch(session_id).expect("fetch");
    assert_eq!(input.session_id, session_id);
    assert_eq!(input.envelopes.len(), 30);
    let got_ids: HashSet<String> = input.envelopes.iter().map(|e| e.event_id.clone()).collect();
    assert_eq!(got_ids, expected_ids);

    // Sorted by occurred_at.
    let mut prev = String::new();
    for env in &input.envelopes {
        assert!(env.occurred_at >= prev, "out of order: {} after {prev}", env.occurred_at);
        prev = env.occurred_at.clone();
    }
}
