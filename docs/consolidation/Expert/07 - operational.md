# Expert System Operational Guide

## Docker Deployment (Future)

No official Docker image yet; users currently run natively.

**Planned (P2):**
```dockerfile
FROM nvidia/cuda:12.1.1-runtime-ubuntu22.04
COPY --from=builder /usr/local/bin/expert-cli /usr/local/bin/
ENV EXPERT_HOME=/data/expert
EXPOSE 8080
CMD ["expert-cli", "serve", "--port", "8080"]
```

## Ports & Services

**Current (Local CLI):**
- No network services; all operations local
- `~/.expert/` directory isolated per user

**Future (P3):**
- **HTTP API:** Port 8080 (inference, expert management)
- **gRPC API:** Port 50051 (streaming inference)
- **Status/Metrics:** Port 9090 (Prometheus metrics)

## Environment Variables

### Training (Python)
```bash
EXPERT_CUDA_VISIBLE_DEVICES=0         # GPU selection
EXPERT_DATA_DIR=./datasets            # Dataset location
EXPERT_HF_TOKEN=hf_...                # Hugging Face access
OPENAI_API_KEY=sk-...                 # GPT-4o for synthetic data
DEEPSEEK_API_KEY=sk-...               # DeepSeek Chat
ANTHROPIC_API_KEY=sk-...              # Claude
```

### Inference (Rust)
```bash
EXPERT_HOME=~/.expert                 # Registry + model cache location
EXPERT_DEVICE=cuda                    # cuda or cpu
EXPERT_LOG_LEVEL=info                 # debug, info, warn, error
EXPERT_VRAM_BUDGET_GB=15              # Max VRAM for inference
```

## Configuration Files

### manifest.json (Expert Package)
Located in expert root, defines training + routing + versioning:

```json
{
  "name": "expert-neo4j",
  "version": "0.0.1",
  "base_model": { "name": "Qwen3-0.6B" },
  "adapter": { "type": "lora", "rank": 16 },
  "training": { "config": { "lr": 5e-5 } },
  "routing": { "keywords": ["cypher", "neo4j"] }
}
```

### expert-registry.json (User Installation State)
Located at `~/.expert/expert-registry.json`:

```json
{
  "version": "1.0",
  "base_models": [...],
  "experts": [...]
}
```

## Logging & Telemetry

**Log locations:**
- `~/.expert/logs/training.log` - Expert training traces
- `~/.expert/logs/inference.log` - Inference events (prompt, experts, latency)

**Telemetry captured per job:**
```json
{
  "job_id": "uuid",
  "router_latency_ms": 15,
  "expert_load_latency_ms": 8,
  "inference_latency_ms": 1250,
  "experts_used": ["json-parser", "english"],
  "vram_peak_mb": 850,
  "tokens_generated": 420,
  "success": true
}
```

## VRAM Budgeting

**Allocation per session:**

| Component | Typical | Max |
|-----------|---------|-----|
| Base model (INT4) | 0.5 GB | 1.2 GB |
| System overhead | 0.1 GB | 0.2 GB |
| 10 experts (25MB avg) | 0.25 GB | 0.8 GB |
| KV cache (32k context) | 0.3 GB | 2.0 GB |
| **Total** | **~1.2 GB** | **~4.2 GB** |

**With 8GB VRAM:** Comfortably fits base + 4-6 experts + 64k context  
**With 16GB VRAM:** Base + 10 experts + 128k context + some parallelism

## Maintenance & Updates

### Updating Expert-CLI
```bash
# Check version
expert-cli --version

# Binary update (user downloads new release)
# Replace ~/.expert/bin/expert-cli
```

### Upgrading Expert Packages
```bash
expert-cli install git+https://github.com/user/expert-neo4j.git#v0.1.0
# Auto-detects version from tag, updates registry
```

### Cleaning Up Old Experts
```bash
expert-cli prune --keep-recent 2
# Removes all but 2 most recent versions per expert
```

### Cache Management
```bash
expert-cli cache clear
# Removes hot cache (SSD), keeps installed experts
```

## Troubleshooting

| Issue | Check |
|-------|-------|
| VRAM OOM | Reduce expert count, increase page size |
| Slow inference | Check GPU utilization (nvidia-smi) |
| Expert load fails | Verify git+https access, signature validation |
| Router timeout | Check Vectorizer/FAISS availability |
