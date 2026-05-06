# 02 — Recommendations

> **Analysis ID:** REWORK-MINMAX-001 · **Date:** 2026-05-05

Actionable recommendations per finding. Each recommendation includes effort estimate, impact, and the specific fix.

---

## P0 — Ship Now (Critical severity, Low effort)

### R-001: Circuit Breaker on Pre-Thinking Fail-Open

**Finding:** F-003 · **Severity:** Critical · **Effort:** Low

**Fix — `cortex-pre-thinking/src/pipeline.rs`:**

```rust
struct PreThinkingCircuitBreaker {
    fail_open_count: AtomicU64,
    window_start: AtomicU64,
    threshold: u64,     // e.g., 5
    window_secs: u64,    // e.g., 60
}

impl PreThinkingCircuitBreaker {
    fn record(&self) -> bool {
        let now = UnixTime::now().as_secs();
        let start = self.window_start.load(Ordering::Relaxed);
        if now - start > self.window_secs {
            self.window_start.store(now, Ordering::Relaxed);
            self.fail_open_count.store(0, Ordering::Relaxed);
        }
        let prev = self.fail_open_count.fetch_add(1, Ordering::Relaxed);
        prev + 1 >= self.threshold
    }
}

// In pipeline.rs run():
let fail_open = resp_opt.is_none();
if fail_open && breaker.record() {
    metrics.incr_circuit_breaker_tripped();
    // Emit law_violation envelope to divergence lane
    // Activate degraded mode
    return PreThinkingOutput {
        bundle: DEGRADED_BUNDLE.to_string(),
        fail_open: true,
        notice: Some(Notice::CircuitBreakerTripped),
        ..
    };
}
```

**Verification:** `scripts/doctor/health.bat` shows degraded status when circuit breaker trips. Canary detects degraded mode.

---

### R-002: Mandatory Advisory on Fail-Open

**Finding:** F-004 · **Severity:** Critical · **Effort:** Low

**Fix — `cortex-pre-thinking/src/formatter.rs`:**

```rust
enum EmptyReason {
    NoResults,        // silent empty
    Timeout,          // advisory bundle
    ServiceDegraded,  // minimal bundle + laws
}

fn format_empty(intent: Intent, reason: EmptyReason) -> String {
    match reason {
        EmptyReason::NoResults => String::new(),
        EmptyReason::Timeout => format!(
            "<!-- cortex: timeout -->\
            > ⚠ Context retrieval timed out. Proceed without Cortex context."
        ),
        EmptyReason::ServiceDegraded => format!(
            "<!-- cortex: degraded -->\
            > ⚠ Cortex degraded. {n} active law(s).",
            n = laws.len()
        ),
    }
}
```

**Note:** Spec 12 Decision 4 ("silence > noise") is respected — this only fires on actual failures, not on "no results found".

**Verification:** Bundle contains `<!-- cortex: timeout -->` or `<!-- cortex: degraded -->` comment on fail-open. Unit test in `pipeline.rs` verifies degraded bundle is non-empty.

---

### R-003: Intent Mismatch Tracking

**Finding:** F-002 · **Severity:** High · **Effort:** Low

**Fix — `cortex-pre-thinking/src/metrics.rs`:**

```rust
// Add to Metrics struct:
pub intent_mismatches: Mutex<BTreeMap<String, u64>>,
pub intent_correct: Mutex<BTreeMap<String, u64>>,

// Add method:
pub fn record_intent_outcome(&self, intent: &str, correct: bool) {
    let mut map = if correct {
        self.intent_correct.lock().unwrap()
    } else {
        self.intent_mismatches.lock().unwrap()
    };
    *map.entry(intent.to_string()).or_insert(0) += 1;
}

// Compute mismatch rate:
fn intent_mismatch_rate(intent: &str) -> f64 {
    let correct = *intent_correct.get(intent).unwrap_or(&0);
    let mismatched = *intent_mismatches.get(intent).unwrap_or(&0);
    let total = correct + mismatched;
    if total == 0 { return 0.0; }
    mismatched as f64 / total as f64
}
```

**Also:** Add `intent_correctness_feedback` field to `QueryResponse` or expose via MCP tool `cortex_feedback(intent, correct: bool)`.

**Verification:** `cortex-ops intent-stats` CLI command prints per-intent mismatch rate. If any intent > 5%, flag for Haiku classifier activation.

---

## P1 — Next Sprint (High severity, Medium effort)

### R-004: Intent-Specific Byte Budgets

**Finding:** F-005 · **Severity:** High · **Effort:** Low

**Fix — `cortex-pre-thinking/src/pipeline.rs`:**

```rust
const INTENT_BUDGETS: &[(Intent, u32, FormatOptions)] = &[
    (Intent::Explain,           16*1024, FormatOptions { snippet_cap: 8,  similar_turns_cap: 0, decisions_cap: 0, laws_cap: 0, ..Default::default() }),
    (Intent::DecisionLookup,   24*1024, FormatOptions { snippet_cap: 3,  similar_turns_cap: 2, decisions_cap: 10, ..Default::default() }),
    (Intent::SimilarProblems,  40*1024, FormatOptions { snippet_cap: 3,  similar_turns_cap: 10, decisions_cap: 3, ..Default::default() }),
    (Intent::LawCheck,         12*1024, FormatOptions { snippet_cap: 0,  similar_turns_cap: 0, decisions_cap: 0, laws_cap: 20, ..Default::default() }),
    (Intent::PreChangeContext, 32*1024, FormatOptions { snippet_cap: 5,  similar_turns_cap: 5, decisions_cap: 5, laws_cap: 10, ..Default::default() }),
    (Intent::FreeSearch,      32*1024, FormatOptions::default()),
];

// In run():
let budget = INTENT_BUDGETS.iter()
    .find(|(i, _, _)| *i == intent)
    .map(|(_, bytes, opts)| (*bytes, opts.clone()))
    .unwrap_or((32*1024, FormatOptions::default()));
```

**Verification:** Metrics histogram `bundle_bytes` segmented by intent shows different distributions per intent after 2 weeks of data.

---

### R-005: Cascade Query Rewriter

**Finding:** F-006 · **Severity:** Medium · **Effort:** Medium

**Fix — `cortex-api/src/query_rewriter.rs`:**

```rust
pub enum QueryRewriter {
    Deterministic(NounPhraseStrip),
    Sonnet { client: HaikuClient, cache: Arc<DashCache<String, QueryRewrite>> },
    Cascade {
        primary: Box<dyn QueryRewriter>,
        fallback: Box<dyn QueryRewriter>,
    },
}

impl QueryRewriter {
    async fn rewrite(&self, prompt: &str, intent: Intent) -> QueryRewrite {
        let result = match self {
            QueryRewriter::Cascade { primary, fallback } => {
                let primary_result = primary.rewrite(prompt, intent).await;
                match primary_result {
                    Ok(r) if r.quality_score > 0.7 => primary_result,
                    _ => fallback.rewrite(prompt, intent).await,
                }
            },
            _ => self.rewrite_internal(prompt, intent).await,
        };
        result.unwrap_or_else(|| DeterministicRewriter.rewrite(prompt, intent))
    }
}
```

**Configuration:** `CORTEX_QUERY_REWRITER=cascade` activates cascade with Sonnet primary + deterministic fallback. Cache key: `sha256(prompt ⊕ intent)`.

**Verification:** Latency + quality tradeoff: cached Sonnet calls skip API; uncached calls fall back to deterministic with no user-visible error.

---

### R-006: Semantic Contradiction Detector for Topic Cards

**Finding:** F-007 · **Severity:** Medium · **Effort:** Medium

**Fix — `cortex-workers/src/topic_cards/contradiction.rs`:**

```rust
pub struct SemanticContradictionDetector {
    haiku: HaikuClient,
}

impl SemanticContradictionDetector {
    pub async fn detect(&self, evidence_a: &str, evidence_b: &str) -> bool {
        let prompt = format!(
            "Given evidence A: '{}' and evidence B: '{}', \
             do they contradict each other semantically? \
             Respond ONLY with YES or NO.",
            evidence_a, evidence_b
        );
        let response = self.haiku.classify(&prompt).await?;
        response.to_uppercase().starts_with("YES")
    }
}

// Use in conjunction with, not replacing, the existing 3 heuristic detectors.
// Add as 4th detector: `ContradictionKind::SemanticConflict`
```

**Verification:** Unit test with known contradictory pairs (e.g., "HNSW ef=64" vs "HNSW ef=128") returns true. Known consistent pairs return false.

---

### R-007: Ship Laws DSL v1 (Rule-Based Detectors)

**Finding:** F-008 · **Severity:** High · **Effort:** Medium

**Phased approach:**

**Phase 1 (ship now, 1 sprint):**
- Parser: Markdown + YAML frontmatter → `Law` struct
- Format: 10 required fields (`id`, `title`, `severity`, `applies_to`, `detector`, `remediation`, `introduced`, `supersedes`, `version`)
- CLI: `cortex laws lint laws/*.md`
- Blocking via `PreToolUse` hook: regex-based detectors run synchronously
- No Deno sandbox yet

**Phase 2 (next sprint):**
- Deno sandbox for scripted detectors
- Observational law capture (async)
- Trust score computation

**Sample Law file:**
```yaml
---
id: LAW-007
title: Never bypass pre-commit hooks
severity: critical
applies_to: ["git", "commit"]
detector: regex:git.*--no-verify
remediation: "Fix the hook failure; do not pass --no-verify."
introduced: 2026-04-17
version: 1
---
```

**Verification:** `cortex laws lint laws/*.md` returns 0 for valid laws. PreToolUse blocks `git commit --no-verify` in Claude Code.

---

### R-008: Deep Analysis MVP

**Finding:** F-009 · **Severity:** Medium-High · **Effort:** Medium

**MVP scope (1 sprint):**
- Trigger: `cortex analysis start "question"` CLI command
- Context retrieval: standard orchestrator query with `intent=similar_problems`
- 3-agent debate: 2 Sonnet agents debate with context as ground truth (3 fixed rounds)
- Judge: human decides (no judge agent automation)
- Output: Markdown with debate transcript + Decision draft

**NOT in MVP:**
- Judge agent automation
- Cost truncation
- Multi-round adaptive rounds
- Dashboard supersession graph

**Verification:** `cortex analysis start "Why does our HNSW recall drop above 1M vectors?"` produces a Decision record indexed in Nexus and citable from future queries.

---

## P2 — Next Quarter (Medium severity, Medium-High effort)

### R-009: Bundle Quality Tracking

**Finding:** F-015 · **Severity:** High · **Effort:** Low

**Fix — `cortex-api/src/feedback.rs`:**

```rust
// New endpoint: POST /api/feedback
struct BundleFeedback {
    query_id: String,
    intent: Intent,
    bundle_bytes: u32,
    helpful: bool,
    files_cited: Vec<PathBuf>,
    latency_ms: u64,
}

// Aggregated metrics:
// - helpful_rate per intent
// - files_cited_rate per intent
// - bundle_quality_score = helpful_rate * files_cited_rate
```

**Also add to `cortex-mcp-server`:** tool `cortex_feedback(query_id, helpful: bool)` that the model can call immediately after responding.

**Verification:** Dashboard shows per-intent quality scores. Quality score drives adaptive budget tuning.

---

### R-010: Canary Default ON in Production

**Finding:** F-014 · **Severity:** Medium-High · **Effort:** Low

**Fix — `cortex-api/src/canary.rs`:**

```rust
enum CanaryMode {
    Dev { interval_secs: 300 },
    Production { interval_secs: 60, alert_on_failure: true },
}

impl Canary {
    fn for_env() -> Self {
        match std::env::var("CORTEX_ENV").as_deref() {
            "production" => Canary::new(CanaryMode::Production),
            _ => Canary::new(CanaryMode::Dev),
        }
    }
}
```

**Also:** 2 consecutive failures → `law_violation` envelope via phase8e alert path. 5 consecutive successes → clear alert state.

**Verification:** `scripts/doctor/canary.bat` succeeds in prod. In CI, canary runs on every PR.

---

### R-011: Cross-Repo Symbol Registry

**Finding:** F-016 · **Severity:** Low-Medium · **Effort:** High

**Fix:** During bootstrap, for each symbol extracted by Tree-sitter:
1. Compute `sha256(content)`
2. Store `(content_hash, symbol_name, repo, path)` in SQLite `SymbolRegistry`
3. On graph write: if same `content_hash` exists in other repos, create `SHARED_ARTIFACT` edge

**Verification:** `cortex graph query --symbol sha256:abc123` returns all repos containing that symbol.

---

### R-012: Shared Adapter Core

**Finding:** F-010 · **Severity:** Medium · **Effort:** High

**Fix:** Extract common adapter logic into `cortex-adapter-core`:
- Event normalizer (tool_calls → Envelope)
- Redactor (static patterns)
- IPC frame builder (synap publish)
- `PreThinkingQueryFn` trait

Each adapter (Claude Code, Cursor, Codex, Gemini) implements only:
- Tool call extractor (IDE-specific)
- Hook registration (IDE-specific)

**Verification:** New adapter for Cursor can be built in 1 week reusing `cortex-adapter-core`.

---

## P3 — Backlog (Lower priority)

### R-013: Classifier Proactive Circuit Breaker

**Finding:** F-011 · **Severity:** Medium · **Effort:** Medium

Add 90% warning tier and `circuit_open` flag that pre-emptively uses static fallback before hitting the daily budget.

---

### R-014: Hot Tier TTL with Soft Delete

**Finding:** F-012 · **Severity:** Medium · **Effort:** Medium

Use `created_at` metadata + query filter to implement soft-delete TTL without SDK mutations.

---

### R-015: Parallel Repo Bootstrap

**Finding:** F-013 · **Severity:** Medium · **Effort:** Medium

Use `rayon::parallel.into_par_iter()` on the 17 repos. Add incremental bootstrap (only re-index modified files since `last_bootstrap_at`).

---

### R-016: Implicit Feedback via Tool Call Overlap

**Finding:** F-001 · **Severity:** Medium · **Effort:** Medium

Compute `Jaccard(bundle_files, touched_files)` as implicit quality signal. High overlap = positive signal; low overlap = negative signal. No user action required.

---

## Recommended Sequencing

```
Sprint 1 (2 weeks): R-001 + R-002 + R-003
  → circuit breaker + advisory (P0)
  → intent mismatch tracking (P0)

Sprint 2 (2 weeks): R-004 + R-007 + R-008
  → adaptive budgets (P1)
  → Laws DSL v1 (P1)
  → Deep Analysis MVP (P1)

Sprint 3 (2 weeks): R-005 + R-006 + R-009
  → cascade query rewriter (P2)
  → semantic contradiction detector (P2)
  → bundle quality tracking (P2)

Sprint 4+: R-010 + R-011 + R-012
  → canary prod default (P2)
  → cross-repo symbol registry (P2)
  → shared adapter core (P2)
```

**Do not start:** R-007 (Laws DSL), R-008 (Deep Analysis), R-010 (Canary) in parallel with abstraction work (Phase A from the prior rework analysis). Same org risk: patches crowd out foundations.