# Cortex System Analysis — GLM-5.1 Rework Report

**Analyst:** z-ai/glm-5.1  
**Date:** 2026-05-06  
**Scope:** Full codebase (10 crates, ~70K+ lines of Rust)

---

## Overall Assessment

Cortex is a **well-architected system** with strong foundations. The codebase demonstrates:

- **No `unsafe` code** — `#![forbid(unsafe_code)]` enforced in every crate
- **No TODO/FIXME/HACK/STUB markers** in production code
- **Strong documentation** with spec cross-references throughout
- **Clean module boundaries** with no circular dependencies
- **Comprehensive docker-compose** with health checks and proper service ordering

The issues identified are **incremental improvements**, not fundamental design flaws. The system is production-viable with the critical fixes applied.

---

## Findings Summary

| Severity | Count | Key Themes |
|----------|-------|------------|
| **Critical** | 2 | Missing schemas/indexes causing silent validation failures |
| **High** | 9 | Type safety gaps, race conditions, memory leaks, code duplication |
| **Medium** | 19 | Architecture drift, error handling gaps, dead code |
| **Low** | 17 | Code quality, test coverage, dependency optimization |

---

## Top 5 Immediate Actions

1. **Create missing JSON schemas** for `knowledge` and `learning` event types (F-001) — validation silently rejects these events today
2. **Add Meilisearch index definitions** for `consolidations` and `topic_cards` (F-002) — bootstrap cannot configure these indexes
3. **Extract Synap worker infrastructure** into shared module (F-003) — eliminates ~1,500 lines of duplication and drift risk
4. **Fix `vocab.rs` KIND_IDS** to include all 12 kinds (F-007) — vocabulary lookups fail for 4 kinds
5. **Fix `record_cron_run` race condition** (F-008) — TOCTOU bug on concurrent cron execution

---

## Documents

| Document | Description |
|----------|-------------|
| [`findings.md`](findings.md) | 47 detailed findings with evidence, impact, and recommendations |
| [`execution-plan.md`](execution-plan.md) | 7-phase remediation plan with tasks, dependencies, and estimates |

---

## Estimated Rework Effort

| Phase | Focus | Duration |
|-------|-------|----------|
| Phase 1 | Critical correctness fixes | 1-2 days |
| Phase 2 | Type safety & validation | 1-2 weeks |
| Phase 3 | Worker infrastructure consolidation | 1-2 weeks |
| Phase 4 | Reliability & concurrency | 1 week |
| Phase 5 | Architecture improvements | 2-4 weeks |
| Phase 6 | Test coverage | 2-3 weeks |
| Phase 7 | Code quality & cleanup | Ongoing |

**Total**: 8-14 weeks for complete rework

---

## Crates Analyzed

| Crate | Lines | Finding Count | Health |
|-------|-------|---------------|--------|
| `cortex-core` | ~2,400 | 13 | Needs schema fixes |
| `cortex-workers` | ~70,000+ | 15 | Duplication + reliability |
| `cortex-api` | ~15,000+ | 7 | Architecture drift |
| `cortex-storage` | ~3,500 | 8 | Race conditions + god object |
| `cortex-adapter-claude-code` | ~2,000 | 2 | Production panics |
| `cortex-pre-thinking` | ~800 | 1 | Unused cap parameter |
| `cortex-mcp-server` | ~1,500 | 1 | Silent timeout fallback |
| `cortex-health` | ~400 | 0 | Clean |
| `cortex-build` | ~300 | 0 | Clean |
| `cortex-cli` | ~5,000+ | 0 | Clean (test data only) |
