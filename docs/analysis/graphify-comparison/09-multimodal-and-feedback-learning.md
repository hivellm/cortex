# 09 — Multi-modal ingestion + query-log learning — **LOW-MED**

Two loosely-related "edges of the corpus" ideas grouped together.

## 9a. Multi-modal ingestion

**graphify:** beyond code+docs it ingests **papers (PDF), images (vision LLM), audio/video (Whisper)**, and remote sources (arXiv, GitHub, tweets, webpages). Notable trick: **video/audio transcription is seeded with the top god nodes** from the code graph, focusing the transcript on domain terms. Remote fetch is SSRF-guarded with byte caps (`security.py`).

**Cortex today:** no image/video/PDF ingestion (grep: none). Cortex ingests code + docs (markdown/text) + session events. Multi-modal is out of its current scope.

**Recommendation:** **Low priority for Cortex's core use case** (live coding-session memory). Worth it only if Cortex expands to "ingest the whole knowledge base incl. design videos/whiteboards/papers." If pursued: add ingestion *sources* that emit the same envelope shape (a transcript/caption is just text with provenance), and reuse the existing redaction + content-hash path. The **god-node-seeded transcription** idea is cheap and clever if video ever lands. Adopt graphify's SSRF/byte-cap hardening for any remote fetch.

## 9b. Query-log learning

**graphify** (`querylog.py`): append-only JSONL of every query (kind, question, corpus, nodes_returned, duration_ms, optional full response), fail-silent, opt-in. The intended signal: most-queried concepts → surface as god nodes; common follow-ups; latency hotspots; low-result queries → coverage gaps.

**Cortex today:** **already has a feedback channel** — `feedback_record` / `feedback_signals` (`crates/cortex-storage/src/metadata.rs`, exposed via `crates/cortex-mcp-server/src/tools.rs`, tested in `feedback_signals_it.rs`) and the relevance eval harness (`crates/cortex-eval`). So Cortex is *ahead* here in having explicit feedback, but it captures **rated/explicit** signals more than passive **every-query telemetry**.

**Recommendation:** **Mostly already covered.** The incremental idea worth borrowing: log *passive* query telemetry (every `cortex_query`/pre-thinking call: intent, hit count, latency, whether the caller followed up) — not just explicit feedback — and mine it for (a) zero/low-result queries → coverage gaps to bootstrap, (b) most-queried entities → candidates to promote/summarize, (c) latency hotspots. This likely fits the existing audit/feedback tables; it's analytics on data Cortex largely already emits, not new infra.

## Effort / impact

- **9a multimodal:** Impact LOW for current scope (HIGH only if scope expands), Effort HIGH per modality. Defer unless a concrete need appears.
- **9b passive query telemetry:** Impact LOW-MED (coverage-gap + promotion signals), Effort LOW (extend existing feedback/audit logging + an analysis query). Reasonable quick win.
