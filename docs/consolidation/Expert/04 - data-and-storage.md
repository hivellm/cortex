# Expert System Data & Storage

## Base Model Storage

**Location**: `~/.expert/models/Qwen3-0.6B/`

**Files:**
- `config.json` - Model configuration (hidden size, layers, attention heads, etc.)
- `generation_config.json` - Inference defaults (temperature, top_p, etc.)
- `tokenizer.json` + `tokenizer_config.json` - BPE tokenizer (150k vocab)
- `vocab.json` - Token embeddings
- `special_tokens_map.json` - Special token mappings
- `added_tokens.json` - Custom tokens

**Size:** ~0.5GB (INT4 quantized)

## Expert Packages (.expert Format)

**Location**: `~/.expert/experts/<expert-name>/`

**Structure:**
```
expert-neo4j-v0.0.1.expert (tar.gz)
├── manifest.json (expert metadata, training config, routing rules)
├── qwen3-06b/adapter_model.safetensors (LoRA weights)
└── signature.ed25519 (publisher signature)
```

**Size:** 5-80 MB per expert (LoRA rank-dependent)

## Storage Directories

| Path | Purpose | Size |
|------|---------|------|
| `~/.expert/models/` | Base model (Qwen3-0.6B) | 0.5-1.2 GB |
| `~/.expert/experts/` | Installed expert packages | 50 MB - 800 MB (10 experts) |
| `~/.expert/cache/` | Hot expert weights (LRU) | Up to 1 GB |
| `~/.expert/logs/` | Telemetry and inference logs | Growing over time |

## Datasets (Training)

**Format**: JSONL (one example per line)

**Entry structure:**
```json
{
  "prompt": "Generate a Cypher query that finds all people",
  "response": "MATCH (p:Person) RETURN p",
  "metadata": {
    "domain": "neo4j",
    "difficulty": "easy",
    "source": "gpt-4o"
  }
}
```

**Storage**: `experts/<expert-name>/datasets/data.jsonl`

**Quality controls:**
- Deduplication (exact + fuzzy)
- Format validation (SQL syntax check, JSON schema validation)
- Diversity threshold (0.75-0.95 embedding similarity cutoff)

## Registry State (expert-registry.json)

**Scope**: Local installation tracking (per-user)

**Versioning:**
- Base model hash + RoPE scaling method tracked
- Compatibility constraints enforced at install time
- Incompatibilities list prevents problematic expert combinations

**Updates:**
- Written atomically after successful install/uninstall
- Last updated timestamp recorded

## Cache Behavior

**Hot Cache (in-VRAM):**
- Base model: always resident
- Experts: LRU eviction when VRAM budget exceeded
- KV cache: per-job isolation, freed after inference

**Cold Cache (SSD):**
- Expert weights pre-decompressed once, then memory-mapped
- Load latency: 50-200ms (SSD I/O + mmap setup)

## Quantization Specifics

**INT4 (preferred):**
- Group size: 128
- Requires per-group scales
- ~0.3-0.4GB VRAM for base model

**INT8:**
- Per-channel or per-token quantization
- ~0.5-0.6GB VRAM
- Better quality for reasoning-heavy tasks

Both use safetensors format with safetensors library for atomic loading.
