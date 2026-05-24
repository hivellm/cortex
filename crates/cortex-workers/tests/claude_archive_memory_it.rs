#![cfg(feature = "claude-archive")]
//! Phase11i §5.3 — RSS hard cap IT.
//!
//! Drives the watcher's polling loop against a synthetic 100k-event
//! corpus and asserts the resident-set-size stays under the spec's
//! 512 MiB ceiling. This catches the "dedupe-set unbounded growth"
//! and "every tick re-parses every byte and never frees" classes of
//! regression — both of which are silent on the unit-test path
//! because the in-memory fixtures the unit tests use are 3-5
//! records.
//!
//! Gated behind `CORTEX_ARCHIVE_MEMORY_IT=1` because synthesising a
//! 100k-event session takes ~20s and the memory probe needs a
//! single-tenant process. Default `cargo test` runs print a "gate
//! not set" notice and exit cleanly so the suite stays fast.
//!
//! The IT writes one synthetic JSONL session under a tempdir matching
//! the `~/.claude/projects/<project>/<session>.jsonl` layout the
//! walker expects, runs `TailWatcher::tick_once` once, samples RSS,
//! and asserts `rss < 512 MiB`. Running the tick once (rather than
//! looping) keeps the IT deterministic and bounds the runtime.

use std::path::PathBuf;
use std::sync::Arc;

use cortex_workers::claude_archive::tail::{TailState, TailWatcher};
use cortex_workers::claude_archive::{StdoutEmitter, WalkConfig};

const ENV_GATE: &str = "CORTEX_ARCHIVE_MEMORY_IT";
const TARGET_RECORDS: usize = 100_000;
const RSS_CAP_BYTES: u64 = 512 * 1024 * 1024;

fn synth_user_assistant_pair(session: &str, idx: usize) -> (String, String) {
    let user_uuid = format!("u{idx:08}");
    let assistant_uuid = format!("a{idx:08}");
    let ts = format!("2026-04-{:02}T12:00:00.000Z", (idx % 28) + 1);
    let user = format!(
        r#"{{"type":"user","sessionId":"{session}","uuid":"{user_uuid}","timestamp":"{ts}","cwd":"/tmp","gitBranch":"main","version":"2.1.0","entrypoint":"cli","userType":"external","message":{{"role":"user","content":"hello idx={idx}"}}}}"#
    );
    let assistant = format!(
        r#"{{"type":"assistant","sessionId":"{session}","uuid":"{assistant_uuid}","parentUuid":"{user_uuid}","timestamp":"{ts}","cwd":"/tmp","gitBranch":"main","version":"2.1.0","message":{{"model":"claude-opus-4-7","id":"msg_{idx}","type":"message","role":"assistant","content":[{{"type":"text","text":"reply idx={idx}"}}],"usage":{{"input_tokens":4,"output_tokens":8}}}},"requestId":"req_{idx}"}}"#
    );
    (user, assistant)
}

fn write_synthetic_session(target: &std::path::Path, pair_count: usize) {
    use std::io::Write;
    let f = std::fs::File::create(target).expect("create synthetic jsonl");
    let mut w = std::io::BufWriter::with_capacity(4 * 1024 * 1024, f);
    let session = "synth-100k";
    for idx in 0..pair_count {
        let (user, assistant) = synth_user_assistant_pair(session, idx);
        writeln!(w, "{user}").unwrap();
        writeln!(w, "{assistant}").unwrap();
    }
    w.flush().unwrap();
}

fn sample_self_rss() -> u64 {
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::new().with_memory(),
    );
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}

#[test]
fn watcher_rss_stays_under_512_mib_on_100k_event_corpus() {
    if std::env::var(ENV_GATE).ok().as_deref() != Some("1") {
        eprintln!(
            "{ENV_GATE} not set — skipping memory IT (synthesises ~50 MB of JSONL + boots a tail watcher; gate it on for the §5.3 acceptance run)."
        );
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let projects = tmp.path().join("projects").join("synth-100k-project");
    std::fs::create_dir_all(&projects).expect("mkdir projects");
    let session_path = projects.join("synth-100k.jsonl");
    let pair_count = TARGET_RECORDS / 2; // 100k records = 50k user/assistant pairs.
    eprintln!(
        "synthesising {pair_count} pairs ({} records) under {}",
        pair_count * 2,
        session_path.display()
    );
    write_synthetic_session(&session_path, pair_count);
    let synth_size = std::fs::metadata(&session_path)
        .map(|m| m.len())
        .unwrap_or(0);
    eprintln!("synthetic session size = {synth_size} bytes");

    let cfg = WalkConfig {
        root: tmp.path().to_path_buf(),
        projects_only: true,
        sidecars: false,
        codex: false,
    };
    let state = Arc::new(TailState::new(0));
    let mut watcher = TailWatcher::new(cfg, Box::new(StdoutEmitter::new()), state);

    let baseline_rss = sample_self_rss();
    eprintln!("baseline_rss_bytes = {baseline_rss}");

    let report = watcher.tick_once().expect("tick_once");
    eprintln!(
        "tick: emitted={} dropped={} files_seen={} errors={}",
        report.emitted,
        report.dropped,
        report.files_seen,
        report.errors.len()
    );
    assert!(report.errors.is_empty(), "tick errors: {:?}", report.errors);
    assert!(report.emitted > 0, "no envelopes emitted");

    let after_rss = sample_self_rss();
    eprintln!("after_tick_rss_bytes = {after_rss}");
    assert!(
        after_rss < RSS_CAP_BYTES,
        "RSS {after_rss} (= {} MiB) exceeds the {} MiB cap",
        after_rss / (1024 * 1024),
        RSS_CAP_BYTES / (1024 * 1024)
    );

    // Run a second tick — the cursor should short-circuit (file
    // unchanged), so RSS must not grow more than a few KiB. The cap
    // here is intentionally loose: process allocators round up,
    // tokio's pool keeps thread RSS, etc.
    let second = watcher.tick_once().expect("second tick");
    let stable_rss = sample_self_rss();
    eprintln!(
        "stable_rss_bytes = {stable_rss} second_tick_emitted = {}",
        second.emitted
    );
    assert_eq!(
        second.emitted, 0,
        "second tick must not re-emit unchanged files"
    );
    assert!(
        stable_rss < RSS_CAP_BYTES,
        "RSS {stable_rss} (= {} MiB) drifted past the {} MiB cap on the second tick",
        stable_rss / (1024 * 1024),
        RSS_CAP_BYTES / (1024 * 1024)
    );
}

#[test]
fn synth_helpers_produce_parseable_records() {
    // Lightweight (non-gated) sanity check so the synth helpers
    // stay honest even when CORTEX_ARCHIVE_MEMORY_IT is unset.
    let (user, assistant) = synth_user_assistant_pair("s-test", 7);
    let v_user: serde_json::Value = serde_json::from_str(&user).expect("user parses");
    let v_assistant: serde_json::Value =
        serde_json::from_str(&assistant).expect("assistant parses");
    assert_eq!(v_user["type"], "user");
    assert_eq!(v_assistant["type"], "assistant");
    assert_eq!(v_assistant["parentUuid"], v_user["uuid"]);
}

#[test]
fn rss_cap_constant_matches_phase11i_spec() {
    // 512 MiB; surface as bytes so the assertion is unambiguous.
    assert_eq!(RSS_CAP_BYTES, 512 * 1024 * 1024);
}

// Suppress unused warnings on `PathBuf` import on some toolchains
// when the gated test path is the only consumer.
fn _path_typecheck() -> Option<PathBuf> {
    None
}
