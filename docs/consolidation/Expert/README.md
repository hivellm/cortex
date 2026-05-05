# Expert System Consolidation Knowledge Base

This directory contains structured institutional knowledge about the HiveLLM Expert project, designed for ingestion into Cortex's knowledge base.

## Files

1. **01 - overview.md** (Purpose, role in HiveLLM, maturity, tech stack, hardware)
   - What Expert is, where it fits, current status, core dependencies

2. **02 - architecture.md** (Six core components, data flow, performance bottlenecks)
   - Base Model, Experts (adapters), Router, Runtime, Marketplace, Orchestrator
   - How they interact, typical latencies, optimization points

3. **03 - public-surface.md** (CLI commands, Python/Node bindings, manifest/registry formats)
   - User-facing APIs: dataset generation, training, packaging, installation, inference
   - Future REST/gRPC/bindings planned for P3

4. **04 - data-and-storage.md** (File layouts, quantization, cache behavior, datasets)
   - Where models/experts live, registry format, training JSONL structure
   - VRAM allocation, hot/cold cache strategies

5. **05 - integrations.md** (Hive ecosystem relationships, SDKs, Git distribution, security)
   - How Expert integrates with Cortex, Nexus, Vectorizer, Synap, Lexum
   - External consumption patterns (Python PyO3, Node NAPI, REST, gRPC)

6. **06 - decisions-and-rationale.md** (9 major design choices with tradeoffs)
   - Why not MoE? Why Qwen3-0.6B? Why Git-based? Why Rust runtime?
   - Why LoRA-first? Why 10 experts max? Why paged KV?

7. **07 - operational.md** (Docker, ports, env vars, config files, logging, VRAM budgeting, troubleshooting)
   - How to deploy, configure, monitor, maintain Expert in production

8. **08 - cortex-relevance.md** (What Cortex should ingest, integration architecture, example flows)
   - Expert catalog metadata, inference telemetry, benchmarks, routing traces
   - Task→Expert routing loop, data pipeline, ingestible artifacts

9. **09 - open-questions.md** (10 design gaps, known limitations, blockers for Cortex)
   - Multi-expert composition semantics, router optimization, marketplace indexing
   - What's needed before Cortex can reliably integrate

## Usage

**For Cortex ingestion:**
- Ingest files in order (01-09) to build complete context
- Focus on 08 (cortex-relevance) for integration planning
- Refer to 09 (open-questions) for dependency gaps

**For project handoff:**
- Use 01-02 for onboarding new team members
- Use 06 for architectural decision justification
- Use 07 for operational runbooks

**For feature planning:**
- Use 09 to identify what must be resolved before X integration
- Use 05 to understand ecosystem dependencies
- Use 03 to scope public API contracts

## Quick Facts

- **Status:** CLI implementation phase (15% overall, design 100%)
- **Base Model:** Qwen3-0.6B (INT4, 0.3-0.6GB VRAM)
- **Max Experts:** 10 per inference
- **Expert Types:** LoRA (primary), DoRA, IA³, soft-prompts
- **Distribution:** Git repositories (no NPM/PyPI)
- **Training:** Python + PyTorch/PEFT; Runtime: Rust + Candle
- **Context:** 120k-200k tokens (RoPE YaRN scaling)
- **VRAM budget:** ~1.2GB typical (8GB min, 16GB recommended)
- **Router latency:** 15-50ms (heuristics + embeddings + mini-policy)
- **Inference latency:** 500ms-10s depending on sequence length (RTX 4090)

## Next Steps for Cortex

1. **Expert indexing:** Ingest `~/.expert/expert-registry.json` into Cortex doc store
2. **Telemetry streaming:** Subscribe to Expert inference logs for feedback loop
3. **Router transparency:** Get decision traces ("why these experts?") for auditing
4. **Validation plugins:** Define post-inference validation per domain
5. **Feedback mechanism:** Send Cortex validation results back to Expert router
