/* Mock data for the Cortex dashboard */

const MOCK = (() => {
  const now = new Date();
  const t = (mins) => {
    const d = new Date(now.getTime() - mins * 60000);
    return d.toTimeString().slice(0, 8);
  };

  const repos = [
    { id: "vectorizer", name: "Vectorizer", events: 12834, color: "amber" },
    { id: "nexus", name: "Nexus", events: 8201, color: "blue" },
    { id: "synap", name: "Synap", events: 5402, color: "green" },
    { id: "rulebook", name: "Rulebook", events: 4118, color: "purple" },
    { id: "expert", name: "Expert", events: 2103, color: "amber" },
    { id: "cortex", name: "Cortex", events: 9722, color: "amber" }
  ];

  const models = ["claude-opus-4-7", "claude-sonnet-4-5", "claude-haiku-4-5", "gpt-5-codex", "gemini-3-pro"];
  const tools = ["claude-code", "cursor", "codex", "gemini"];

  const events = [
    { id: "01HXY8M0", t: t(0.2), kind: "tool_call", title: "Edit", detail: "src/index/hnsw/configurator.rs · +18 / −7", session: "ses_8a3f", model: "claude-opus-4-7", repo: "Vectorizer", duration: "118 ms" },
    { id: "01HXY8M1", t: t(0.4), kind: "turn", title: "User prompt", detail: "Refactor the HNSW configurator to extract the bench harness…", session: "ses_8a3f", model: "claude-opus-4-7", repo: "Vectorizer", duration: "—" },
    { id: "01HXY8L9", t: t(1.1), kind: "law_violation", title: "LAW-007 · Never bypass pre-commit hooks", detail: "git commit --no-verify intercepted in PreToolUse", session: "ses_8a3f", model: "claude-opus-4-7", repo: "Vectorizer", duration: "blocked" },
    { id: "01HXY8L8", t: t(1.4), kind: "tool_call", title: "Bash", detail: "cargo check --workspace", session: "ses_8a3f", model: "claude-opus-4-7", repo: "Vectorizer", duration: "2.41 s" },
    { id: "01HXY8L7", t: t(2.0), kind: "agent_call", title: "Task: code-reviewer", detail: "Review HNSW configurator extraction (8 files)", session: "ses_8a3f", model: "claude-opus-4-7", repo: "Vectorizer", duration: "12.8 s" },
    { id: "01HXY8L5", t: t(2.6), kind: "memory", title: "Project memory updated", detail: "CLAUDE.md · added section 'HNSW bench harness'", session: "ses_8a3f", model: "claude-opus-4-7", repo: "Vectorizer", duration: "—" },
    { id: "01HXY8L4", t: t(3.2), kind: "tool_call", title: "Read", detail: "specs/06-embedder.md · 12 KB", session: "ses_8a3f", model: "claude-opus-4-7", repo: "Vectorizer", duration: "8 ms" },
    { id: "01HXY8L2", t: t(4.1), kind: "decision", title: "DEC-2026-014 · Symbol-level chunking via Tree-sitter", detail: "Promoted from analysis ANL-031; supersedes DEC-2026-008", session: "ses_8a3f", model: "claude-opus-4-7", repo: "Vectorizer", duration: "—" },
    { id: "01HXY8L0", t: t(5.0), kind: "tool_call", title: "Grep", detail: "pattern: 'HnswIndex::with' · 14 matches across 6 files", session: "ses_7c92", model: "claude-sonnet-4-5", repo: "Nexus", duration: "32 ms" },
    { id: "01HXY8K8", t: t(6.7), kind: "law_violation", title: "LAW-012 · HNSW recall benchmark must run before merge", detail: "PostToolUse observation · evidence: missing bench artifact", session: "ses_7c92", model: "claude-sonnet-4-5", repo: "Vectorizer", duration: "annotated" },
    { id: "01HXY8K6", t: t(8.3), kind: "tool_call", title: "Write", detail: "tests/hnsw_recall_floor.rs · new file (84 lines)", session: "ses_7c92", model: "claude-sonnet-4-5", repo: "Vectorizer", duration: "47 ms" },
    { id: "01HXY8K3", t: t(11.0), kind: "turn", title: "Pre-thinking bundle injected", detail: "3 decisions · 2 analyses · 7 similar turns · 1 active law", session: "ses_5b21", model: "gpt-5-codex", repo: "Cortex", duration: "62 ms" },
    { id: "01HXY8K1", t: t(13.5), kind: "tool_call", title: "Edit", detail: "cortex-workers/src/classifier.rs · +42 / −18", session: "ses_5b21", model: "gpt-5-codex", repo: "Cortex", duration: "94 ms" },
    { id: "01HXY8JZ", t: t(15.2), kind: "memory", title: "Reference memory captured", detail: "Haiku batch=32 keeps P95 under 2.8s on M2 Pro", session: "ses_5b21", model: "gpt-5-codex", repo: "Cortex", duration: "—" },
    { id: "01HXY8JX", t: t(18.0), kind: "agent_call", title: "Task: deep-analysis judge", detail: "Round 3/3 · panel verdict reached (3-1)", session: "ses_5b21", model: "claude-opus-4-7", repo: "Cortex", duration: "47 s" },
    { id: "01HXY8JT", t: t(22.4), kind: "tool_call", title: "Bash", detail: "pnpm test --filter @cortex/dashboard", session: "ses_4a09", model: "claude-haiku-4-5", repo: "Cortex", duration: "8.2 s" },
    { id: "01HXY8JR", t: t(26.1), kind: "decision", title: "DEC-2026-013 · Adopt Meilisearch as Lexum stand-in", detail: "Migration is a client swap; revisit when Lexum hits parity", session: "ses_4a09", model: "claude-opus-4-7", repo: "Cortex", duration: "—" },
    { id: "01HXY8JN", t: t(31.0), kind: "tool_call", title: "Edit", detail: "docker-compose.yml · added meilisearch service", session: "ses_4a09", model: "claude-opus-4-7", repo: "Cortex", duration: "22 ms" },
    { id: "01HXY8JK", t: t(35.5), kind: "law_violation", title: "LAW-003 · No raw SQL in handlers", detail: "PostToolUse · 1 occurrence in api/handlers/decisions.rs", session: "ses_3f88", model: "gemini-3-pro", repo: "Cortex", duration: "annotated" },
    { id: "01HXY8JG", t: t(42.0), kind: "turn", title: "User prompt", detail: "Why is recall dropping above 1M vectors?", session: "ses_3f88", model: "gemini-3-pro", repo: "Vectorizer", duration: "—" }
  ];

  const decisions = [
    {
      id: "DEC-2026-014",
      title: "Symbol-level chunking via Tree-sitter for code artifacts",
      status: "active",
      author: "claude-opus-4-7",
      sourceAnalysis: "ANL-031",
      rationale: "Fixed-size windows degrade retrieval precision on multi-symbol files. Tree-sitter symbol boundaries align with how engineers reason about code — function/struct/class scopes — and improved top-5 precision from 0.61 to 0.78 on the Vectorizer golden set.",
      tags: ["embedding", "code", "retrieval"],
      cites: ["spec 06", "ANL-031", "DEC-2026-008"],
      supersedes: "DEC-2026-008",
      occurredAt: "2 days ago",
      chain: [
        { id: "DEC-2025-219", title: "Fixed-size 1024-token windows", state: "old" },
        { id: "DEC-2026-008", title: "512-token sliding windows w/ 64 overlap", state: "old" },
        { id: "DEC-2026-014", title: "Tree-sitter symbol-level chunking", state: "current" }
      ]
    },
    {
      id: "DEC-2026-013",
      title: "Adopt Meilisearch as the full-text engine until Lexum reaches parity",
      status: "active",
      author: "andre@hivellm",
      sourceAnalysis: "ANL-029",
      rationale: "Lexum is not production-ready for the v1 throughput targets. Meilisearch ships typo-tolerant BM25 today; the migration is a client swap (spec 08, Decision 2). Cost of adoption is bounded — index format is reproducible from raw events.",
      tags: ["fulltext", "infra"],
      cites: ["spec 08", "ANL-029"],
      supersedes: null,
      occurredAt: "5 days ago",
      chain: null
    },
    {
      id: "DEC-2026-011",
      title: "Classify events with Claude Haiku 4.5 via the Claude Code CLI in v1",
      status: "active",
      author: "claude-opus-4-7",
      sourceAnalysis: "ANL-024",
      rationale: "We already have ample Haiku quota; this eliminates training, GPU dependency, and model-serving from v1 scope. Iterate on the prompt, not on adapter weights. SDK path is a per-worker config flip when CLI overhead matters.",
      tags: ["classifier", "haiku"],
      cites: ["spec 05", "architecture §5.2.1"],
      supersedes: "DEC-2026-002",
      occurredAt: "9 days ago",
      chain: null
    },
    {
      id: "DEC-2026-008",
      title: "512-token sliding windows with 64-token overlap",
      status: "superseded",
      supersededBy: "DEC-2026-014",
      author: "claude-sonnet-4-5",
      rationale: "Initial chunking heuristic for code; replaced once Tree-sitter benchmarks landed.",
      tags: ["embedding"],
      cites: ["spec 06"],
      occurredAt: "3 weeks ago",
      chain: null
    }
  ];

  const laws = [
    { id: "LAW-001", title: "Diagnostic-first: tsc/cargo check before tests", severity: "warn", scope: "git, build", applies: 4321, violations7d: 12, rate: 0.28, blocked: false, detector: "hook:diagnostic_before_tests", remediation: "Run `tsc --noEmit` or `cargo check` before invoking tests." },
    { id: "LAW-003", title: "No raw SQL in HTTP handlers", severity: "warn", scope: "api, db", applies: 1208, violations7d: 7, rate: 0.58, blocked: false, detector: "ast:sql_in_handler", remediation: "Move queries to a repository module; handlers compose, not query." },
    { id: "LAW-007", title: "Never bypass pre-commit hooks (`--no-verify`)", severity: "critical", scope: "git, commit", applies: 9821, violations7d: 3, rate: 0.03, blocked: true, detector: "hook:pre_commit_no_skip", remediation: "Fix the hook failure; do not pass --no-verify without explicit human authorization." },
    { id: "LAW-009", title: "Sequential editing — one file at a time", severity: "info", scope: "edit", applies: 8211, violations7d: 41, rate: 0.50, blocked: false, detector: "session:edit_concurrency", remediation: "Decompose multi-file tasks; chain edits sequentially." },
    { id: "LAW-012", title: "HNSW recall benchmark must run before merge", severity: "critical", scope: "vectorizer, ci", applies: 188, violations7d: 1, rate: 0.53, blocked: true, detector: "ci:hnsw_recall_artifact", remediation: "Run `cargo bench --bench hnsw_recall_floor` and attach artifact to PR." },
    { id: "LAW-014", title: "Decisions must cite at least one prior turn or analysis", severity: "warn", scope: "decision", applies: 312, violations7d: 4, rate: 1.28, blocked: false, detector: "graph:decision_has_cites", remediation: "Reference the analysis or turn the decision derives from." },
    { id: "LAW-018", title: "Secrets must not appear in tool-call inputs", severity: "critical", scope: "redaction", applies: 24021, violations7d: 0, rate: 0.00, blocked: true, detector: "regex:secrets_v2", remediation: "Static redactor catches these at the edge; investigate any breach." },
    { id: "LAW-021", title: "Knowledge capture at end-of-task", severity: "info", scope: "workflow", applies: 412, violations7d: 22, rate: 5.34, blocked: false, detector: "session:no_kb_capture", remediation: "Call `rulebook_knowledge_add` before closing the session." }
  ];

  const violations = [
    { id: "VIO-9281", lawId: "LAW-007", at: t(1.1), session: "ses_8a3f", model: "claude-opus-4-7", repo: "Vectorizer", action: "blocked", evidence: "git commit -m 'wip refactor' --no-verify", remediation: "Tool call intercepted in PreToolUse; user prompted to fix the failing hook." },
    { id: "VIO-9244", lawId: "LAW-007", at: "yesterday 14:22", session: "ses_5b21", model: "gpt-5-codex", repo: "Cortex", action: "blocked", evidence: "git commit --no-verify -m 'fix(deps): bump axum'", remediation: "Blocked. Hook output revealed unformatted Rust file; reformatted then re-committed." },
    { id: "VIO-9201", lawId: "LAW-007", at: "2d ago 09:41", session: "ses_3f88", model: "gemini-3-pro", repo: "Vectorizer", action: "annotated", evidence: "git push --no-verify origin feat/hnsw-bench", remediation: "User-overridden block; trust score for (gemini-3-pro, Vectorizer) decremented by 0.04." }
  ];

  const memories = [
    { kind: "project", title: "HNSW bench harness location", excerpt: "The recall benchmark lives at `vectorizer/benches/hnsw_recall_floor.rs` and is gated behind the `bench` feature. CI invokes it via `cargo bench --bench hnsw_recall_floor`.", repo: "Vectorizer", topics: ["hnsw", "ci"], updated: "2 days ago" },
    { kind: "reference", title: "Haiku batch=32 latency budget", excerpt: "Single Haiku batch of 32 events keeps P95 under 2.8s on M2 Pro; a pool of 25 workers exceeds the 500 eps target with headroom.", repo: "Cortex", topics: ["classifier", "perf"], updated: "5 days ago" },
    { kind: "feedback", title: "Pre-thinking bundle byte budget", excerpt: "A 4KB byte budget keeps Claude Code happy; budgets above 8KB visibly degrade output quality and increase repetition.", repo: "Cortex", topics: ["pre-thinking", "ux"], updated: "1 week ago" },
    { kind: "user", title: "André prefers diff-style edit summaries", excerpt: "When summarizing a change set, lead with the diff stats (+/- lines per file) before any prose. Group by repository.", repo: "*", topics: ["preferences"], updated: "2 weeks ago" },
    { kind: "project", title: "Synap stream naming convention", excerpt: "All Cortex streams are prefixed `cortex.events.*`; bootstrap traffic goes to `cortex.events.bootstrap` so it can be paused independently.", repo: "Cortex", topics: ["synap", "infra"], updated: "3 days ago" },
    { kind: "reference", title: "Tree-sitter grammars in scope", excerpt: "Top-5 langs covered for symbol-level chunking: Rust, TypeScript, Python, Go, JavaScript. Others fall back to 512-token windows.", repo: "Cortex", topics: ["embedding", "code"], updated: "2 days ago" },
    { kind: "project", title: "Trust-score recompute schedule", excerpt: "Nightly job at 03:15 UTC. Reads from `cortex.events.violations` rolling 30-day window. Output stored in Synap KV under `trust:(model, repo)`.", repo: "Cortex", topics: ["governance", "trust"], updated: "4 days ago" },
    { kind: "feedback", title: "Cursor `_cortex_context.md` is acceptable", excerpt: "Workspace-side artifact for Cursor capture is OK provided users get an opt-out flag (`--no-workspace-write`).", repo: "*", topics: ["cursor", "adapters"], updated: "1 week ago" }
  ];

  const analyses = [
    {
      id: "ANL-031",
      title: "Why does HNSW recall drop above 1M vectors?",
      status: "concluded",
      panel: ["claude-opus-4-7", "gpt-5-codex", "gemini-3-pro"],
      judge: "claude-opus-4-7",
      rounds: 3,
      durationS: 47,
      verdict: "Recall degrades because ef_construction defaults underweight the long-tail of low-degree nodes once vector count exceeds 1M. Adopt Tree-sitter symbol-level chunking AND raise ef_construction floor to 200 for collections > 500k. Promoted to DEC-2026-014.",
      decisionId: "DEC-2026-014",
      occurredAt: "2 days ago"
    },
    {
      id: "ANL-029",
      title: "Lexum vs Meilisearch as the v1 full-text engine",
      status: "concluded",
      panel: ["claude-opus-4-7", "claude-sonnet-4-5"],
      judge: "andre@hivellm",
      rounds: 2,
      durationS: 32,
      verdict: "Lexum is not production-ready for v1 throughput; Meilisearch is a 1-week integration with a clean migration path. Adopt Meilisearch now; revisit at Phase-4 hardening.",
      decisionId: "DEC-2026-013",
      occurredAt: "5 days ago"
    },
    {
      id: "ANL-027",
      title: "Should bootstrap re-classify on schema_version bumps?",
      status: "concluded",
      panel: ["claude-opus-4-7", "gpt-5-codex"],
      judge: "claude-opus-4-7",
      rounds: 2,
      durationS: 21,
      verdict: "Yes for severity & redaction_suggestions fields (cheap to recompute); no for embeddings (cost-prohibitive — keep schema_version in cache key, lazy migrate on read).",
      decisionId: "DEC-2026-010",
      occurredAt: "1 week ago"
    }
  ];

  const toolUsage = [
    { tool: "Read", icon: "R", calls: 12480, avgMs: 8, errRate: 0.001, share: 0.32 },
    { tool: "Edit", icon: "E", calls: 8204, avgMs: 94, errRate: 0.012, share: 0.21 },
    { tool: "Grep", icon: "G", calls: 5120, avgMs: 32, errRate: 0.003, share: 0.13 },
    { tool: "Bash", icon: "$", calls: 3892, avgMs: 1840, errRate: 0.087, share: 0.10 },
    { tool: "Write", icon: "W", calls: 2740, avgMs: 47, errRate: 0.005, share: 0.07 },
    { tool: "Task", icon: "T", calls: 1611, avgMs: 12800, errRate: 0.041, share: 0.04 },
    { tool: "WebFetch", icon: "↗", calls: 982, avgMs: 2210, errRate: 0.122, share: 0.025 },
    { tool: "Glob", icon: "*", calls: 711, avgMs: 14, errRate: 0.000, share: 0.018 }
  ];

  // Graph nodes — laid out manually for the explorer view
  const graph = {
    nodes: [
      { id: "ses", label: "Session", x: 50, y: 50, kind: "session" },
      { id: "t1", label: "Turn", x: 200, y: 80, kind: "turn" },
      { id: "t2", label: "Turn", x: 200, y: 180, kind: "turn" },
      { id: "tc1", label: "Edit", x: 360, y: 60, kind: "tool_call" },
      { id: "tc2", label: "Bash", x: 360, y: 130, kind: "tool_call" },
      { id: "tc3", label: "Read", x: 360, y: 220, kind: "tool_call" },
      { id: "art1", label: "configurator.rs", x: 540, y: 60, kind: "artifact" },
      { id: "art2", label: "spec-06.md", x: 540, y: 220, kind: "artifact" },
      { id: "dec1", label: "DEC-2026-014", x: 720, y: 110, kind: "decision" },
      { id: "law1", label: "LAW-007", x: 540, y: 310, kind: "law" },
      { id: "vio1", label: "VIO-9281", x: 360, y: 310, kind: "violation" },
      { id: "anl1", label: "ANL-031", x: 720, y: 220, kind: "analysis" }
    ],
    edges: [
      { from: "ses", to: "t1", label: "CONTAINS" },
      { from: "ses", to: "t2", label: "CONTAINS" },
      { from: "t1", to: "tc1", label: "INVOKED" },
      { from: "t1", to: "tc2", label: "INVOKED" },
      { from: "t2", to: "tc3", label: "INVOKED" },
      { from: "tc1", to: "art1", label: "WROTE" },
      { from: "tc3", to: "art2", label: "READ" },
      { from: "art1", to: "dec1", label: "REFERENCES" },
      { from: "anl1", to: "dec1", label: "PRODUCED" },
      { from: "vio1", to: "law1", label: "OF" },
      { from: "tc2", to: "vio1", label: "OBSERVED_IN" }
    ]
  };

  return { repos, models, tools, events, decisions, laws, violations, memories, analyses, toolUsage, graph };
})();
