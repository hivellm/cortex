# Expert System — Cortex Integration Points

## What Cortex Should Ingest from Expert

### 1. Expert Catalog & Metadata
**What:** Manifest + capabilities for all installed/available experts

**File:** `.expert/expert-registry.json` (local) + metadata from HF Hub (remote available)

**Use in Cortex:**
- Index expert names, versions, capabilities in Cortex document store
- Enable Cortex to query Expert for "all JSON parsing experts"
- Link Task→Expert mappings (task classification → select expert)

**Format to ingest:**
```json
{
  "expert": "expert-neo4j",
  "version": "0.0.1",
  "capabilities": ["query:cypher", "tech:neo4j"],
  "routing_keywords": ["cypher", "match", "neo4j"],
  "domain": "graph-databases",
  "accuracy_benchmark": 0.95
}
```

### 2. Expert Inference Telemetry
**What:** Per-job metrics (latency, success rate, experts used)

**Source:** `~/.expert/logs/inference.log` + structured JSON events

**Use in Cortex:**
- Track expert effectiveness over time
- Identify failing expert+task combinations
- Route future similar tasks to high-performing experts
- Feedback loop: Task Classification → Expert Selection → Validation → Learning

**Schema:**
```json
{
  "timestamp": "2025-11-03T12:00:00Z",
  "task_id": "cortex-classification-123",
  "input": { "prompt": "...", "body": "..." },
  "experts_selected": ["json-parser", "english"],
  "inference_latency_ms": 1250,
  "success": true,
  "output_valid": true,
  "accuracy": 0.98
}
```

### 3. Expert Benchmarks & Validation Results
**What:** Performance metrics on standard datasets

**Source:** `experts/<name>/benchmarks/results.json` (after expert training)

**Use in Cortex:**
- Compare expert quality before installation
- Predict which expert set will work best for incoming document
- Track expert degradation over time (detect retraining needs)

**Metrics tracked:**
- Accuracy (domain-specific F1, BLEU, etc.)
- Latency (p50, p95, p99)
- VRAM footprint
- Token generation quality (repetition, coherence)

### 4. Expert Routing Decisions
**What:** When Expert router selects experts, return decision trace

**Source:** Internal to Expert, surfaced via API

**Use in Cortex:**
- Audit trail: "why did Expert pick experts X, Y, Z for this prompt?"
- Learn from routing mistakes
- Correlate with document classification errors

**Decision trace:**
```json
{
  "prompt": "Find all Neo4j nodes",
  "heuristics": { "format": "cypher", "confidence": 0.92 },
  "embeddings_match": [
    { "expert": "neo4j-expert", "score": 0.94 },
    { "expert": "graph-reasoning", "score": 0.87 }
  ],
  "final_selection": ["neo4j-expert", "graph-reasoning"],
  "reasoning": "High heuristic + embedding agreement on domain"
}
```

## Integration Architecture

### Flow: Cortex → Expert → Feedback

```
Cortex receives document
    ↓
Classify task (routing)
    ↓
Query Expert API: "infer(prompt, body, experts=auto)"
    ↓
Expert router selects adapters
    ↓
Expert inference runs
    ↓
Result + telemetry returned to Cortex
    ↓
Cortex validates output (schema, semantics)
    ↓
Success/failure logged back to Expert
    ↓
Expert updates historical success rates
    ↓
Future similar tasks route to high-performing experts
```

### Data Pipeline

1. **Training phase**: Generate synthetic datasets (Synap), train experts (Expert)
2. **Installation**: User installs expert packages via `expert-cli install`
3. **Indexing**: Cortex reads `~/.expert/expert-registry.json`, indexes in doc store
4. **Routing**: Cortex task classifier determines domain, suggests expert set
5. **Inference**: Cortex calls Expert.infer(), gets result + telemetry
6. **Validation**: Cortex post-processes, validates output quality
7. **Feedback**: Cortex records success/failure, feeds back to Expert router cache
8. **Learning**: Over time, router learns which expert sets work best

## Key Ingestible Artifacts

| Artifact | Location | Refresh | Size |
|----------|----------|---------|------|
| Expert registry | `~/.expert/expert-registry.json` | On install | <10 KB |
| Manifests | `~/.expert/experts/*/manifest.json` | Per expert | ~5 KB each |
| Inference logs | `~/.expert/logs/inference.log` | Streaming | Growing |
| Benchmarks | `experts/*/benchmarks/results.json` | After training | ~100 KB each |
| Telemetry | Expert API /metrics | Real-time | Streaming |

## Example: Document Classification → Expert Selection

**Cortex scenario:**

1. User uploads JSON document to classify
2. Cortex classifier identifies: "JSON structure analysis" + "Neo4j schema inference"
3. Queries Expert catalog: "give me experts for JSON + graph domains"
4. Finds: `expert-json-parser` + `expert-neo4j`
5. Calls: `Expert.infer(prompt, body, experts=['expert-json-parser', 'expert-neo4j'])`
6. Expert returns: classification results + routing decision trace
7. Cortex logs: success + telemetry in Cortex knowledge base
8. Future JSON+Neo4j documents automatically route to same expert pair
