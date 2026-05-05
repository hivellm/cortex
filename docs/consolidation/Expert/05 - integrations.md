# Expert System Integrations

## HiveLLM Project Relationships

### Cortex (Consumer)
**Role:** Expert serves as specialized inference backend for Cortex's document classification, routing, and reasoning tasks.

**Integration points:**
- Cortex's task router calls Expert.infer() to classify documents
- Expert's router selects appropriate domain experts (JSON, Neo4j, SQL, etc.)
- Results fed back to Cortex for pipeline orchestration
- Future: Cortex indexes experts via Vectorizer for semantic selection

### Nexus (Graph Database)
**Role:** Expert generates Cypher queries and performs graph analytics.

**Integration:**
- Neo4j expert trained on Cypher syntax + Nexus schema
- Expert outputs Cypher → Nexus executes → results returned to caller
- Multi-hop reasoning via expert composition (graph.expert + reasoning.expert)

### Vectorizer (Semantic Search)
**Role:** Expert index stored in Vectorizer for ANN-based expert discovery.

**Integration:**
- Each expert's manifest + capabilities embedded
- Router queries Vectorizer: "find experts for 'parse JSON documents'"
- Vectorizer returns top-K semantic matches (cosine similarity)
- Reduces router latency from O(n) to O(log n) for large expert catalogs

### Synap (Task Synthesis)
**Role:** Expert can be trained/fine-tuned via Synap's synthetic data generation.

**Integration:**
- Synap generates domain-specific examples (e.g., Cypher patterns)
- Expert trains on Synap-generated JSONL datasets
- Synap validates expert quality before marketplace publication

### Lexum (Search)
**Potential future:**
- Expert catalog searchable via Lexum
- Full-text search on expert descriptions, capabilities, benchmarks

## External Dependencies

### Hugging Face Hub (hf-hub crate)
- Model downloads (Qwen3-0.6B from HF Model Hub)
- SafeTensors loading for weights
- API token required for gated models

### PyTorch/PEFT Stack
- Training pipeline only (training environment)
- Runtime has no Python dependency (pure Rust inference)

### LLM Providers (for synthetic data)
- DeepSeek Chat API
- OpenAI GPT-4o
- Anthropic Claude
- Provides training datasets without manual labeling

## External Consumption (SDKs & Bindings)

### Python PyO3 Bindings (Future)
```python
from expert_rs import ExpertEngine
engine = ExpertEngine()
```

### Node.js NAPI Bindings (Future)
```javascript
const { ExpertEngine } = require('@hivellm/expert-rs');
```

### REST API (Future P3)
```
POST /infer
{
  "prompt": "...",
  "experts": ["json-parser", "english"],
  "temperature": 0.5
}
```

### gRPC API (Future P3)
Native async streaming via tonic framework.

## Storage Integration

### Git Distribution
- Each expert is a Git repository
- No centralized registry (marketplace is Git index)
- Supports forking, versioning, branching
- Install command: `expert-cli install git+https://github.com/user/expert-xyz.git#v1.0.0`

### Local Filesystem
- Expert registry: `~/.expert/expert-registry.json`
- Model cache: `~/.expert/models/`
- Expert store: `~/.expert/experts/`
- Compatible with multi-user systems (per-user installation)

## Security Integration

### Ed25519 Signatures
- Expert packages signed by publisher's private key
- Verification before installation prevents tampering
- Public key stored in manifest or fetched from publisher

### Integrity Checks
- SHA256 hash of manifest + weights
- Compatibility matrix checked at install time
- VRAM budget enforcement prevents OOM attacks
