# Expert System Public Surface

## CLI Commands (expert-cli)

### Dataset Operations
```bash
expert-cli dataset generate --manifest manifest.json --output data.jsonl
expert-cli dataset validate --dataset data.jsonl
```
Synthetic data generation from LLM providers (DeepSeek, Claude, GPT-4o).

### Training Pipeline
```bash
expert-cli train --manifest manifest.json --dataset data.jsonl --output weights/
expert-cli validate --expert weights/adapter --dataset test.jsonl
```
Configurable LoRA/DoRA/IA³ training with checkpointing every 250 steps.

### Expert Packaging
```bash
expert-cli package --manifest manifest.json --weights weights/adapter --output expert.v1.0.0
expert-cli sign --expert expert.v1.0.0
```
Creates .expert tar.gz files with Ed25519 signatures.

### Installation & Discovery
```bash
expert-cli install git+https://github.com/hivellm/expert-neo4j.git#v0.0.1
expert-cli install /path/to/expert.v1.0.0
expert-cli list [--verbose] [--base-model Qwen3-0.6B]
```
Git-based or local package installation with registry tracking.

### Inference
```bash
expert-cli chat --experts graph,json --prompt "Find nodes" --max-tokens 50 --device cuda
```
One-shot inference mode for scripting, automatic adapter discovery.

## Python Bindings (Future)

```python
from expert import ExpertEngine

engine = ExpertEngine(base_model='qwen3-0.6b-int4')
result = await engine.infer(
    prompt="Parse this JSON",
    body=json_doc,
    experts=['json-parser', 'english']
)
print(result.output, result.latency_ms)
```

## Node.js Bindings (Future)

Via napi-rs (NAPI), single Rust binary callable from TypeScript:

```javascript
const { ExpertEngine } = require('@hivellm/expert');

const engine = new ExpertEngine({
  baseModel: 'qwen3-0.6b-int4'
});

const result = await engine.infer({
  prompt: "Generate Rust code",
  experts: ['rust', 'async-patterns']
});
```

## Manifest Format (manifest.json)

Core configuration for expert packages:

```json
{
  "name": "expert-neo4j",
  "version": "0.0.1",
  "base_model": { "name": "Qwen3-0.6B", "rope_scaling": "yarn-128k" },
  "adapter": { "type": "lora", "rank": 16, "alpha": 16 },
  "training": {
    "config": { "lr": 5e-5, "temp": 0.7, "dropout": 0.1 },
    "dataset": { "generation": { "domain": "neo4j", "count": 8000 } }
  },
  "routing": {
    "keywords": ["cypher", "match", "neo4j"],
    "exclude_keywords": ["what is", "explain"],
    "priority": 0.85
  },
  "capabilities": ["query:cypher", "tech:neo4j"]
}
```

## Registry Format (expert-registry.json)

Located at `~/.expert/expert-registry.json`, tracks installed experts:

```json
{
  "version": "1.0",
  "last_updated": "2025-11-03T12:00:00Z",
  "base_models": [{ "name": "Qwen3-0.6B", "sha256": "...", "quantization": "int4" }],
  "experts": [
    {
      "name": "expert-neo4j",
      "version": "0.0.1",
      "base_model": "Qwen3-0.6B",
      "path": "~/.expert/experts/expert-neo4j",
      "source": "git+https://github.com/hivellm/expert-neo4j.git#v0.0.1",
      "installed_at": "2025-11-03T12:00:00Z",
      "capabilities": ["tech:neo4j", "query:cypher"]
    }
  ]
}
```
