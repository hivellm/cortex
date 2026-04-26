# cortex-classifier

> Spec: [`docs/specs/05-classifier.md`](../../docs/specs/05-classifier.md)

Cortex's enrichment layer. Takes a redacted event and returns
`ClassifierOutput` with topics, severity, PII risk, redaction suggestions,
and (for oversized payloads) a summary. Backed by **Claude Haiku** through
the Claude Code CLI by default, with a deterministic static fallback so
the pipeline never blocks on the model.

## Composition

The default stack is a chain of decorators:

```
StaticClassifier  ◄──  HaikuCli (or HaikuSdk)  ◄──  Cached  ◄──  Budgeted
                                                                  ▲
                                                                  └─ entry point
```

- **`Budgeted`** — daily/monthly USD budget per `ClassifierMode`.
  Budget-exhausted calls degrade to the static classifier.
- **`Cached`** — content-hash keyed cache; identical events skip the model.
- **`HaikuCli`** — invokes the Claude Code CLI with the prompt template
  in [`prompts/`](prompts/) and parses the JSON response.
- **`StaticClassifier`** — regex-driven, offline, used as the floor.

`ClassifierStack` wires these together; callers pick a stack
configuration and treat it as a single `Classifier` trait object.

## Usage

```toml
[dependencies]
cortex-classifier = { path = "../cortex-classifier" }
```

```rust
use cortex_classifier::{ClassifierStack, EnrichmentInput};

let stack = ClassifierStack::default_haiku_cli()?;
let out   = stack.classify(EnrichmentInput::from_envelope(&envelope)?).await?;
println!("topics={:?} severity={:?} pii_risk={:?}",
    out.topics, out.severity, out.pii_risk);
```

The crate is `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Vocabularies

`prompt::TOPIC_VOCAB_V1` is the frozen topic set the model is constrained
to. Severity and PII-risk levels are mirrored from `cortex_core::vocab`.
Adding or renaming a topic is a versioned change (`*_V2`); the v1 vocab
stays callable for back-compat with previously cached entries.

## Pricing & budget

`ClassifierSpend` and `PricingTable` track per-mode token usage and
cost. The default pricing matches Anthropic's published Haiku rates;
override with a custom table if you proxy through a different gateway.

## Configuration

| Variable                       | Default                       | Notes                                       |
|--------------------------------|-------------------------------|---------------------------------------------|
| `CORTEX_CLASSIFIER_MODE`       | `haiku-cli`                   | `haiku-cli`, `haiku-sdk`, or `static`.      |
| `CORTEX_CLASSIFIER_BUDGET_USD` | `1.00`                        | Soft daily budget per mode.                 |
| `CORTEX_CLASSIFIER_CACHE_DIR`  | `./data/classifier-cache`     | On-disk cache backing `ClassifierCache`.    |
| `ANTHROPIC_API_KEY`            | _required for haiku-cli_      | Read by the Claude Code CLI itself.         |

## Testing

```bash
cargo test -p cortex-classifier
```

Tests cover the static fallback, cache behavior, budget exhaustion, and
prompt-template rendering. The Haiku adapter is covered with a fake
process driver to keep the suite hermetic.
