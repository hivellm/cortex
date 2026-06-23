# 34. Classification vs PII redaction: redaction removes secrets at ingestion; classification gates visibility per principal; they compose sequentially, never substitute

**Status**: proposed
**Date**: 2026-06-23
**Related Tasks**: phase21_data-classification-access-control

## Context

Cortex already ships ingestion-time PII redaction (DEC-017 / phase15e): secrets (API keys, SSNs, tokens) are replaced with `[REDACTED:class]` tokens before the payload reaches any index. Data classification (phase21) labels facts with a sensitivity level + compartments so the retrieval engine can gate visibility per principal. The two mechanisms overlap in intent (protect sensitive data) but differ in mechanism (irreversible payload mutation vs reversible visibility gate). Without a clear boundary, operators may mistakenly assume that classifying a fact makes it safe to store unredacted secrets.

## Decision

Redaction and classification are orthogonal, sequential, and non-substitutable. Order: redaction FIRST (at adapter/ingestion time, before Synap publish), then classification stamping (at bootstrap walker / classifier worker, applied to the already-redacted payload). A classified fact still undergoes redaction; a redacted fact still receives a classification. They compose: a fact containing salary tables is both redacted (salary numbers replaced) AND classified (`confidential + [hr]`). They do NOT substitute: classifying a fact `restricted` does NOT make it safe to store an unredacted API key in its payload — the redaction pass must still run. A classification-bearing rule may ALSO trigger a redaction signal: the content detector that marks a body as `customer_pii` may simultaneously flag the literal SSN for redaction (passed as a hint to the redaction pass). The two passes share a signal (`pii_risk`) but execute independently. Operators MUST NOT disable the redaction pass on the assumption that classification makes secrets safe to store.

## Alternatives Considered

- Classification replaces redaction — rejected: classification is a visibility gate, not a secret-erasure mechanism; a `restricted` fact with an unredacted API key is still a secret stored in plaintext in Meili / Vectorizer / Nexus; classification cannot prevent a storage breach.
- Redaction is scoped to the classification level (only redact `confidential+`) — rejected: secrets (API keys, tokens, PII) are dangerous regardless of the fact's classification level; a `public` fact containing an API key must still be redacted.
- Run classification before redaction so the classifier can see the raw payload — rejected: the classifier then has access to unredacted secrets; the classifier output (summary, sensitivity signals) should be derived from the redacted payload to avoid leaking secrets into classifier logs.

## Consequences

Pros: clear non-overlapping responsibilities; redaction provides storage safety independent of classification; classification provides access control independent of what was redacted; composing them gives defense-in-depth at both the storage and retrieval layers; the ordering rule (redact first, then classify) is simple and auditable. Cons: operators must understand two mechanisms; classifying a fact does not remove the need to audit the redaction pass for completeness (the redaction-coverage doctor still runs independently).
