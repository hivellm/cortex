#![cfg(feature = "claude-archive")]
//! Phase11i §5.4 — corrupt JSONL line failure-mode IT.
//!
//! A session file that interleaves valid records with malformed lines
//! must:
//! - never panic the watcher;
//! - count the bad lines toward `envelopes_dropped` so the dashboard
//!   surfaces a regression in upstream JSONL emission;
//! - keep emitting the *valid* records around the malformed lines.
//!
//! The IT writes a small synthetic session, runs `tick_once`, and
//! asserts the report carries a non-zero drop count plus at least
//! one emitted envelope. Runs unconditionally (no env gate) because
//! the corpus is tiny and the cost is sub-second.

use std::sync::Arc;

use cortex_workers::claude_archive::tail::{TailState, TailWatcher};
use cortex_workers::claude_archive::{StdoutEmitter, WalkConfig};

fn write_corrupt_session(path: &std::path::Path) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).expect("create jsonl");
    // 1) Valid user record.
    writeln!(
        f,
        r#"{{"type":"user","sessionId":"sess-1","uuid":"u1","timestamp":"2026-04-20T17:47:59.616Z","cwd":"/tmp","gitBranch":"main","version":"2.1.0","entrypoint":"cli","userType":"external","message":{{"role":"user","content":"hi"}}}}"#
    )
    .unwrap();
    // 2) Valid paired assistant.
    writeln!(
        f,
        r#"{{"type":"assistant","sessionId":"sess-1","uuid":"a1","parentUuid":"u1","timestamp":"2026-04-20T17:47:59.700Z","cwd":"/tmp","gitBranch":"main","version":"2.1.0","message":{{"model":"claude-opus-4-7","id":"msg_1","type":"message","role":"assistant","content":[{{"type":"text","text":"ok"}}],"usage":{{"input_tokens":4,"output_tokens":8}}}},"requestId":"req_1"}}"#
    )
    .unwrap();
    // 3) Garbage — not even close to JSON.
    writeln!(f, "this is not valid json at all }} {{ ::: corrupted").unwrap();
    // 4) Half-finished JSON.
    writeln!(f, r#"{{"type":"user","sessionId":"sess-1","uu"#).unwrap();
    // 5) Object missing the `type` field — typeless.
    writeln!(f, r#"{{"sessionId":"sess-1","uuid":"orphan"}}"#).unwrap();
    // 6) Another valid pair so the watcher can still emit.
    writeln!(
        f,
        r#"{{"type":"user","sessionId":"sess-1","uuid":"u2","timestamp":"2026-04-20T17:48:00.000Z","cwd":"/tmp","gitBranch":"main","version":"2.1.0","entrypoint":"cli","userType":"external","message":{{"role":"user","content":"again"}}}}"#
    )
    .unwrap();
    writeln!(
        f,
        r#"{{"type":"assistant","sessionId":"sess-1","uuid":"a2","parentUuid":"u2","timestamp":"2026-04-20T17:48:00.100Z","cwd":"/tmp","gitBranch":"main","version":"2.1.0","message":{{"model":"claude-opus-4-7","id":"msg_2","type":"message","role":"assistant","content":[{{"type":"text","text":"ok2"}}],"usage":{{"input_tokens":4,"output_tokens":8}}}},"requestId":"req_2"}}"#
    )
    .unwrap();
}

#[test]
fn corrupt_lines_are_dropped_and_counted_without_panic() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("projects").join("corrupt-project");
    std::fs::create_dir_all(&project).expect("mkdir project");
    let session = project.join("corrupt-session.jsonl");
    write_corrupt_session(&session);

    let cfg = WalkConfig {
        root: tmp.path().to_path_buf(),
        projects_only: true,
        sidecars: false,
        codex: false,
    };
    let state = Arc::new(TailState::new(0));
    let mut watcher = TailWatcher::new(cfg, Box::new(StdoutEmitter::new()), state);
    let report = watcher.tick_once().expect("tick_once must not panic");

    // The two valid pairs map to two Turn envelopes — the mapper
    // pairs user↔assistant by parentUuid.
    assert!(
        report.emitted >= 2,
        "expected ≥2 valid envelopes, got {}",
        report.emitted
    );
    // The malformed + typeless lines (3 of them — the garbage line,
    // the half-JSON, and the typeless object) must land in the drop
    // counter via the §5.4 wiring.
    assert!(
        report.dropped >= 3,
        "corrupt lines must increment envelopes_dropped; got dropped={}, errors={:?}",
        report.dropped,
        report.errors
    );
    // The watcher must NEVER surface a corrupt-line as a hard error
    // — the reader warns + drops, the tick stays healthy.
    assert!(
        report.errors.is_empty(),
        "corrupt-line path must not surface tick errors: {:?}",
        report.errors
    );
}

#[test]
fn second_tick_on_unchanged_corrupt_file_re_emits_nothing() {
    // The cursor short-circuits when (mtime, len) match, so a
    // corrupt file does NOT inflate the drop counter on every tick.
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("projects").join("corrupt-project-2");
    std::fs::create_dir_all(&project).expect("mkdir project");
    let session = project.join("corrupt-session.jsonl");
    write_corrupt_session(&session);

    let cfg = WalkConfig {
        root: tmp.path().to_path_buf(),
        projects_only: true,
        sidecars: false,
        codex: false,
    };
    let state = Arc::new(TailState::new(0));
    let mut watcher = TailWatcher::new(cfg, Box::new(StdoutEmitter::new()), state);
    let _first = watcher.tick_once().expect("first tick");
    let second = watcher.tick_once().expect("second tick");
    assert_eq!(second.emitted, 0, "unchanged file must not re-emit");
    assert_eq!(
        second.dropped, 0,
        "unchanged corrupt file must not re-count drops"
    );
    assert!(second.errors.is_empty(), "{:?}", second.errors);
}