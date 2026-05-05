# TransmutationLite — Integrations

## Position in HiveLLM Ecosystem

```
┌─────────────────────────────────────────────────────────┐
│                   HiveLLM Ecosystem                      │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  Classify (LLM Classification)                          │
│    ↓                                                    │
│    └─→ Transmutation Lite (TypeScript, lightweight)    │
│           ↓                                             │
│         Markdown + Metadata                             │
│           ↓                                             │
│    Vectorizer (Vector embeddings)                       │
│           ↓                                             │
│    Nexus (Graph database + vector search)              │
│                                                         │
│  Full Transmutation (Rust, production RAG)            │
│    └─→ Advanced features (OCR, audio, video)           │
│           ↓                                             │
│    Vectorizer / Nexus (alternative pipeline)           │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

## Relationship to Full Transmutation

| Aspect | TransmutationLite | Transmutation (Rust) |
|--------|-------------------|---------------------|
| **Purpose** | Classification workflows | RAG pipelines |
| **Precision** | Basic (untested) | 80%+ (tested) |
| **Performance** | Moderate (Node.js) | 98x faster than Docling |
| **Memory** | Moderate (~100–200 MB) | Very low (~20 MB) |
| **OCR** | ❌ None | ✅ Tesseract |
| **Audio/Video** | ❌ None | ✅ Whisper |
| **Archives** | ❌ None | ✅ ZIP, TAR |
| **Integration** | ✅ Easy (npm) | Moderate (CLI/FFI) |
| **Status** | ✅ Production Ready | ✅ Production Ready |

## Integration with Classify

TransmutationLite is the **default document converter** for HiveLLM Classify.

### Usage Flow

```typescript
import { convert } from '@hivellm/transmutation-lite';
import { ClassifyClient } from '@hivellm/classify';

// 1. Convert document to Markdown
const conversionResult = await convert('./contract.pdf');

// 2. Classify the Markdown content
const classifier = new ClassifyClient({
  provider: 'deepseek',
  apiKey: process.env.DEEPSEEK_API_KEY,
});

const classificationResult = await classifier.classifyText(
  conversionResult.markdown
);

console.log('Domain:', classificationResult.classification.domain);
console.log('Type:', classificationResult.classification.doc_type);
```

### Why TransmutationLite for Classify?

1. **Easy integration**: npm dependency; no external tools
2. **Node.js native**: Runs in Node.js process; no subprocess calls
3. **Lightweight**: No OCR, audio, video overhead
4. **Good enough**: Classification doesn't require 98x precision
5. **Fast development**: Faster than Rust integration for prototyping

## Relationship to Vectorizer

- **Input**: Markdown from TransmutationLite (or Transmutation)
- **Output**: Vector embeddings
- **Integration**: Vectorizer accepts Markdown text; compatible with TransmutationLite output

## Relationship to Nexus

- **Input**: Metadata + Embeddings (from Vectorizer)
- **Storage**: Vector graph database
- **Compatibility**: TransmutationLite metadata includes format, pageCount, author, title

Example flow:
```
PDF → TransmutationLite → Markdown + Metadata
                          ↓
                       Vectorizer (embed text)
                          ↓
                       Nexus (store + search)
```

## Relationship to Other HiveLLM Projects

### Lexum (Search)
- **NOT used**: Meilisearch used directly instead (for Classify indexing)
- **Compatible**: Transmutation Lite output (Markdown) can feed Lexum if needed

### Synap (Sync/Orchestration)
- **NOT integrated yet**: Future consideration
- **Potential**: Synap could trigger batch conversions via TransmutationLite CLI

### Expert (Fine-tuning)
- **NOT integrated yet**: Future consideration
- **Potential**: Converted documents could feed Expert training pipelines

### Rulebook (Task Management)
- **No dependency**: TransmutationLite is independent of Rulebook
- **Used by**: Rulebook could orchestrate Cortex (which uses Transmutation Lite)

## npm Package Distribution

- **Package name**: `@hivehub/transmutation-lite`
- **Status**: Ready for publication; not yet published to npm
- **Repository**: https://github.com/hivellm/transmutation-lite
- **License**: MIT
- **Node requirement**: ≥18.0.0

## Docker & Deployment

**NOT containerized**: TransmutationLite is a library, not a service.
- Install via npm in Node.js applications
- Alternative: Transmutation Rust binary can be containerized

## When to Use TransmutationLite vs. Full Transmutation

| Scenario | Use |
|----------|-----|
| Document classification (Classify) | TransmutationLite ✅ |
| Quick previews / prototyping | TransmutationLite ✅ |
| Production RAG pipelines | Transmutation (Rust) ✅ |
| High-precision document processing | Transmutation (Rust) ✅ |
| OCR required | Transmutation (Rust) ✅ |
| Audio/video processing | Transmutation (Rust) ✅ |
| Node.js-only environment | TransmutationLite ✅ |
| Rust toolchain available | Either (prefer Rust for perf) |
