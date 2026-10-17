> **SUPERSEDED 2026-10-16** by `phase34_daily-agent-memory-recovery` (canonical, Codex-coordinated; see docs/analysis/daily-agent-memory-2026-10/05-joint-decision.md). Do not execute this checklist. Kept for reference only. Claude's prototype work for these items lives unmerged on branch `phase34-daily-memory` (worktree E:/HiveLLM/Cortex-phase34) as candidate input for the canonical task.

## 1. Implementation
- [ ] 1.1 §1.1 Reinstall current cortex-adapter-claude, cortex-hook and cortex-mcp-server binaries from source; record versions
- [ ] 1.2 §1.2 No-admin adapter autostart via Startup-folder .vbs (same pattern as the consolidator); start it now; document in scripts/setup
- [ ] 1.3 §1.3 Silent-death alarm: hook surfaces one notice per session when the adapter pipe is unreachable; /v1/health degrades when no event ingested within a configured window
- [ ] 1.4 §1.4 Live trace: a real prompt flows hook -> adapter -> ingestion -> Synap -> classifier -> Meili/Vectorizer; record event id and index receipts
- [ ] 1.5 §2.1 Disable retention.memory_consolidate in default_jobs and the live cron_jobs row
- [ ] 1.6 §2.2 Restore the ten merged consolidated_*.md auto-memory files into one-fact files with backups and rebuilt MEMORY.md index lines
- [ ] 1.7 §2.3 SessionStart synchronous in the live global hook config and the packaged plugin shim (F-007)
- [ ] 1.8 §3.1 Mount feedback on the shared file-backed MetadataStore (F-004)
- [ ] 1.9 §3.2 Capture provenance from client/hook context: agent, session, turn; stop hard-coding tool=claude-code (F-005)
- [ ] 1.10 §3.3 Active-lessons bundle section sourced from structured state (status active, scope repo+global), similarity orders only (F-003)
- [ ] 1.11 §3.4 Corpus-revision cache key so a new capture or supersession invalidates cached context (F-006)
- [ ] 1.12 §3.5 Persist delivery evidence: lesson ids injected per prompt
- [ ] 1.13 §4.1 Jev backend at capture time for lesson/decision candidates: durable, scope, contradiction; pinned jev-1.13.0; sanitized egress; static fallback (F-011..F-019 scoped)
- [ ] 1.14 §5.1 Stop the 30 s full archive reload into the keyword lane (F-002)
- [ ] 1.15 §5.2 Reduce claude-archive full-tree walk cost; make GPU reranker opt-in; remeasure idle CPU/memory
- [ ] 1.16 §6.1 Seeded end-to-end acceptance test on the real install (05-execution-plan success definition)
- [ ] 1.17 §6.2 Update or create documentation covering the implementation
- [ ] 1.18 §6.3 Write tests covering the new behavior
- [ ] 1.19 §6.4 Run tests and confirm they pass

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior
- [ ] 2.3 Run tests and confirm they pass
