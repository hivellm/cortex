# 40 — Classification Model

> **Status:** 🟡 P1 schema shipped · **Owner:** Core team · **Depends on:** 07, 08, 30
> **Phase:** phase21_data-classification-access-control

## Goal

Every retrievable Cortex fact gains two orthogonal axes that together
define a Bell-LaPadula access lattice: a scalar **sensitivity level**
(no-read-up) and a set of **compartments** (need-to-know). The
classification model is the load-bearing primitive — the principal
store (spec 41), ACL enforcement plane (§5), and the migration path
(§2.7) all build on it.

## Scope

**In:**

- Two optional columns on every `Envelope`, `EnrichedEvent`,
  `Document` (Meili), and `ChunkMetadata` (Vectorizer): `class_level`
  + `class_compartments`.
- Classification stamper `graph/classification.rs` that idempotently
  stamps graph `NodeOp` props on every enriched event.
- Meili settings v8 — `class_level` filterable + sortable;
  `class_compartments` filterable.
- Versioned reindex aliases for the classification cut-over.
- `cortex-ops migrate-classification` CLI for the one-shot backfill.

**Out:**

- Principal store + RBAC role bindings (spec 41).
- ACL filter builder + per-lane enforcement (phase21 §5).

## ADR cross-reference

| ADR | locks |
|-----|-------|
| 032 | Merge semantics: declared floor + classifier escalation-only + explicit emitter wins. |
| 034 | Redaction × classification ordering: redaction runs first; `[REDACTED:]` tokens are still detectable for classification escalation. |

## 1. Column set

Every entity that participates in retrieval carries:

| column               | type              | null? | rule |
|----------------------|-------------------|-------|------|
| `class_level`        | `u8` (0–3)        | yes   | Absent = operator default at read time. |
| `class_compartments` | `String[]`        | yes   | Absent = no compartment restriction. |

The columns are **optional everywhere** so callers that pre-date
phase21 round-trip unchanged. `skip_serializing_if = "Option::is_none"` /
`skip_serializing_if = "Vec::is_empty"` keeps wire payloads compact.

## 2. Sensitivity level ordinal

| value | label         | meaning |
|-------|---------------|---------|
| 0     | `public`      | Anyone may read. Default when absent. |
| 1     | `internal`    | Authenticated operator only. |
| 2     | `confidential`| Named role or explicit grant required. |
| 3     | `restricted`  | Highest sensitivity; named compartment required. |

The ordinal is a `u8`; values > 3 are anomalous and flagged by the
migration scanner. The lattice predicate is `clearance_level >=
fact_level`.

## 3. Compartment vocabulary

The open, config-extensible canonical set in v1:

| label          | semantics |
|----------------|-----------|
| `financial`    | Billing, cost, pricing data. |
| `hr`           | HR records, salaries, performance data. |
| `legal`        | Privileged legal / compliance artefacts. |
| `security`     | Pentest reports, vuln data, secrets inventory. |
| `customer_pii` | Personal identifiable information belonging to customers. |

Unknown compartment labels are not rejected at ingest (open set); they
are flagged as anomalies by the migration scanner
(`ClassificationAnomaly::UnknownCompartment`). New labels are added by
extending `KNOWN_COMPARTMENTS` in
`crates/cortex-workers/src/graph/classification_migration.rs`.

## 4. Stamper rules (ADR-032 merge semantics)

The classification stamper
(`crates/cortex-workers/src/graph/classification.rs::stamp_classification_props_on_patch`)
applies the following precedence chain:

1. **Explicit emitter override wins.** An `Envelope` that carries
   `class_level` / `class_compartments` from a trusted emitter
   (e.g. a manual classification via the admin API) propagates
   verbatim; the stamper's `entry().or_insert()` idiom never
   overwrites a pre-set value.
2. **Event-level value propagated.** `EnrichedEvent.class_level` is
   used as-is if set; if `None`, the `default_level` from config is
   imputed.
3. **Classifier escalation-only.** The classifier worker may
   escalate (`level = max(declared, detected)`) and union compartments;
   it may never downgrade a level or remove a compartment.
4. **Declared config floor is inviolable.** The `[cortex.classification]`
   path rules (phase21 §3.1) produce the floor; the classifier
   cannot produce a result below it.

## 5. Backfill and migration

`cortex-ops migrate-classification` (phase21 §2.7) implements the
one-shot backfill:

```
cortex-ops migrate-classification \
  --archive-root /data/cortex/archive \
  --project cortex \
  --default-level 0 \
  [--no-dry-run] \
  [--json]
```

**Dry-run** (default): scan the archive, count events missing
`class_level`, report anomalies, exit `0` (clean) or `2` (anomalies).

**Live**: same scan + impute `class_level = <default-level>` + empty
compartments on every candidate row. Graph writes are wired in
phase21 §3.2.

## 6. Versioned reindex aliases

| constant | value |
|----------|-------|
| `MEILI_CLASSIFICATION_ALIAS` | `cortex-meili-classification-v1` |
| `VECTORIZER_CLASSIFICATION_ALIAS` | `cortex-vector-classification-v1` |

The aliases follow the phase18 §2.8 rollback contract: the `-v1`
suffix enables an `alias --revert` to the pre-classification slice
without data destruction.

## 7. Pinned tests

| file | test | what it pins |
|------|------|--------------|
| `cortex-core/src/events.rs` | `classification_fields_tests::*` | Serde round-trip + default omission for `Envelope` fields. |
| `cortex-workers/src/graph/classification.rs` | `tests::*` (7) | Stamper idempotence, default fallback, emitter-set preservation, per-node walk. |
| `cortex-workers/src/fulltext/builders.rs` | `classification_projection_tests::*` (4) | `class_level`/`class_compartments` propagation into `Document`. |
| `cortex-workers/src/embedder/chunker.rs` | `stamp_classification_tests::*` (4) | Same contract for `ChunkMetadata`. |
| `cortex-workers/src/fulltext/settings.rs` | `classification_axis_fields_are_filterable_and_class_level_sortable` | Meili settings v8 contract. |
| `cortex-workers/src/graph/reindex_alias.rs` | `classification_*` (3) | Alias naming invariants. |
| `cortex-workers/src/graph/classification_migration.rs` | `tests::*` (6) | Migration scanner: dry-run, imputation, anomaly detection. |
| `cortex-workers/src/classifier/statics.rs` | `detect_sensitivity_*` (9), `merge_sensitivity_*` (4) | Content-detection labelled fixtures; merge escalate-only contract. |
| `cortex-workers/src/classifier/statics.rs` | `redacted_token_*`, `redaction_and_classification_*` (3) | Redaction → classification ordering invariant. |
| `cortex-workers/src/classifier_worker/worker.rs` | `into_enriched_*` (7), `bootstrap_event_with_class_*` (2) | Enrichment-path merge: declared floor × detected sensitivity precedence chain. |

## 8. Assignment pipeline and merge precedence (phase21 §3)

### 8.1 Assignment pipeline

Classification is assigned to an event by composing three independent
sources, each running in a distinct stage of the ingestion pipeline:

```
Bootstrap walker (config rules)     Live canonical envelope
         │                                  │
         ▼                                  ▼
  BootstrapEvent.class_level        Envelope.class_level
  BootstrapEvent.class_compartments Envelope.class_compartments
         │                                  │
         └────────────────┬─────────────────┘
                          │
                    NormalisedEvent
                    .class_level   (= declared floor)
                    .class_compartments
                          │
              ┌───────────▼────────────┐
              │   Classifier worker    │
              │  detect_sensitivity()  │
              │  (content signals)     │
              └───────────┬────────────┘
                          │
                  merge_sensitivity(declared, detected)
                  level = max(declared, detected)
                  compartments = union
                          │
                          ▼
                   EnrichedEvent
                   .class_level
                   .class_compartments
```

### 8.2 Merge precedence (ADR-032)

Three sources compose through `merge_sensitivity` (escalate-only):

| priority | source | rule |
|----------|--------|------|
| 3 (highest) | **Explicit trusted-emitter override** | A future admin API or trusted hook that stamps `class_level` directly on an `Envelope` before ingestion. Not available in v1; reserved as the top of the precedence chain when it lands. |
| 2 | **Detected** — classifier content signals | `SENSITIVITY_RULES` keyword scan over the flat redacted payload. Only escalates; the classifier worker may never produce a result below the declared floor. |
| 1 (floor) | **Declared** — config path rules | `[cortex.classification]` glob rules → `(level, compartments)` stamped by the walker on `BootstrapEvent`; carried on `Envelope.class_level` for live events. |

The merge formula:
```
merged.level        = max(declared.level,        detected.level)
merged.compartments = union(declared.compartments, detected.compartments)
```

If the result is `(0, [])` (public + no compartments), `class_level` and
`class_compartments` are left as `None` on `EnrichedEvent` to preserve
backward compatibility ("absent = public" per the column contract in §1).

### 8.3 Redaction × classification ordering (ADR-034, phase21 §3.5)

Redaction (`cortex-core::redact`) runs in the **adapter layer** before the
event is published to Synap, which is before the classifier worker stamps
classification. The two mechanisms compose sequentially and neither
substitutes for the other:

1. **Redaction** removes the raw secret from the payload irreversibly.
2. **Classification** gates visibility of the already-redacted fact per
   principal.

`[REDACTED:<class>]` tokens that redaction leaves in the payload are
*intentionally detectable* by `detect_sensitivity` — the `\[REDACTED:`
pattern maps to `internal / customer_pii`. This ensures a fact that
contained raw PII is still classified at ≥ `internal` after redaction;
there is no silent downgrade window between the two steps.
