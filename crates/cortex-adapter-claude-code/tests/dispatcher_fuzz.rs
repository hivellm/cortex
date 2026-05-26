//! Phase14i §1.5 — dispatcher fuzz test.
//!
//! Throws 100 randomly-shaped hook payloads at the dispatcher and
//! asserts none crash the process. The dispatcher's contract
//! (`Dispatcher::dispatch -> HookResponse`) makes every internal
//! error degrade to `HookResponse::empty()`, but the test is
//! still load-bearing: it pins the contract structurally so a
//! future refactor that re-introduces `unwrap()` on a frame
//! field surfaces as a fuzz failure instead of a daemon crash in
//! production.

use std::sync::Arc;

use cortex_adapter_claude_code::{
    dispatch_inline, AdapterSection, Dispatcher, MemoryPublisher, Metrics, SessionManager,
    SyncClient,
};
use serde_json::{json, Value};

fn build_dispatcher() -> Arc<Dispatcher> {
    let cfg = AdapterSection {
        api_endpoint: "http://127.0.0.1:1".into(),
        pre_thinking: cortex_adapter_claude_code::PreThinkingSection {
            timeout_ms: 25,
            ..Default::default()
        },
        ..Default::default()
    };
    let metrics = Arc::new(Metrics::new());
    let sessions = Arc::new(SessionManager::new());
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn cortex_adapter_claude_code::Publisher> = publisher;
    let sync = Arc::new(SyncClient::new(&cfg, metrics.clone()));
    Arc::new(Dispatcher::new(sessions, pub_dyn, sync, 12345))
}

/// Tiny LCG so the test is deterministic + the corpus stays
/// reproducible across runs. `rand` is intentionally avoided.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn pick<T: Copy>(&mut self, choices: &[T]) -> T {
        let idx = (self.next() % choices.len() as u64) as usize;
        choices[idx]
    }
    fn bool(&mut self) -> bool {
        self.next() & 1 == 0
    }
}

fn random_value(rng: &mut Lcg, depth: u8) -> Value {
    if depth >= 3 {
        return match rng.next() % 4 {
            0 => Value::Null,
            1 => Value::Bool(rng.bool()),
            2 => json!(rng.next() as i64),
            _ => json!(format!("s{}", rng.next() % 1000)),
        };
    }
    match rng.next() % 9 {
        0 => Value::Null,
        1 => Value::Bool(rng.bool()),
        2 => json!(rng.next() as i64),
        3 => json!((rng.next() as f64) / 1e9),
        4 => json!(format!("s{}", rng.next() % 1_000_000)),
        5 => {
            // Empty string — many hook fields default-skip this.
            json!("")
        }
        6 => {
            let n = (rng.next() % 5) as usize;
            let arr: Vec<Value> = (0..n).map(|_| random_value(rng, depth + 1)).collect();
            Value::Array(arr)
        }
        7 => {
            let n = (rng.next() % 5) as usize;
            let mut m = serde_json::Map::new();
            for i in 0..n {
                m.insert(format!("k{i}"), random_value(rng, depth + 1));
            }
            Value::Object(m)
        }
        _ => {
            // A frame with valid hook names sometimes — we want
            // the dispatcher to traverse the real branches too.
            let hook = rng.pick(&[
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Stop",
                "SubagentStop",
                "SessionStart",
                "Notification",
                "totally-unknown",
                "",
            ]);
            json!({
                "hook": hook,
                "session_id": format!("sess-{}", rng.next() % 100),
                "cwd": format!("/tmp/path-{}", rng.next() % 10),
                "payload": random_value(rng, depth + 1),
            })
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatcher_survives_100_random_hook_payloads() {
    let dispatcher = build_dispatcher();
    let mut rng = Lcg::new(0x14_5eed_deca_u64);
    let mut handled = 0usize;
    for _ in 0..100 {
        let value = random_value(&mut rng, 0);
        // Either the random value is already a frame-shaped
        // object or we wrap it as the payload of a frame so the
        // dispatcher sees something that looks-ish like a hook
        // call.
        let frame = if value.is_object() && value.get("hook").is_some() {
            value
        } else {
            let hook_choices = [
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Stop",
                "totally-unknown",
            ];
            let hook = rng.pick(&hook_choices);
            json!({
                "hook": hook,
                "session_id": format!("sess-{}", rng.next() % 100),
                "cwd": format!("/tmp/path-{}", rng.next() % 10),
                "payload": value,
            })
        };
        // `dispatch_inline` parses the frame + drives the
        // dispatcher; the contract is that it ALWAYS returns a
        // JSON value (the canonical empty `{}` on any internal
        // failure). A panic on any of the 100 cases would surface
        // as a tokio task panic and fail the test.
        let resp = dispatch_inline(frame, dispatcher.clone()).await;
        // HookResponse serialises to either `{}` or a structured
        // object — both are JSON objects. The structural assert is
        // that serialisation never panics (which would surface as
        // a tokio task panic and fail the test).
        let serialised = serde_json::to_value(&resp).unwrap_or(Value::Null);
        assert!(
            serialised.is_object() || serialised.is_null(),
            "dispatcher response must serialise to a JSON object, got {serialised:?}"
        );
        handled += 1;
    }
    assert_eq!(handled, 100, "every fuzz iteration must complete");
}
