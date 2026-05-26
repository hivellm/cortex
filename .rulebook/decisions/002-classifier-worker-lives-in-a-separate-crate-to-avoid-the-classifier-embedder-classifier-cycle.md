# 2. Classifier worker lives in a separate crate to avoid the classifier -> embedder -> classifier cycle

**Status**: proposed
**Date**: 2026-04-27
**Related Tasks**: phase1_classifier_worker

## Context

Spec 05 (cortex-classifier) shipped the classifier library but explicitly deferred the "worker binary wiring" follow-up. The worker has to publish on `cortex.events.enriched` using the EnrichedEvent struct that `cortex-embedder` defines (and which `cortex-graph` and `cortex-fulltext` also re-export). Putting the worker binary inside the cortex-classifier crate would force cortex-classifier to depend on cortex-embedder, while cortex-embedder already depends on cortex-classifier — a hard cargo cycle.</context>
<parameter name="decision">Introduce a standalone `cortex-classifier-worker` crate that depends on cortex-classifier (for the Classifier stack types) and cortex-embedder (for EnrichedEvent). The library half exports the consumer/publisher abstractions and the Worker loop; the [[bin]] entry composes a ClassifierStack and runs the pool until ctrl-c.

## Decision

_No decision recorded._

## Alternatives Considered

- Move EnrichedEvent down into cortex-core so the classifier library can return it without depending on cortex-embedder. Rejected for this pass because it would touch embedder, graph, and fulltext at the same time and we need the bridge online first.
- Define a parallel EnrichedEvent struct in cortex-classifier with identical Serde shape. Rejected — keeping two structs in sync invites drift and the very first schema change would silently break the wire format.

## Consequences

Pros: cortex-classifier stays a pure library; the worker can evolve its dependencies (Synap SDK version, retry logic, room-creation, metrics) without touching the lib. Cons: the workspace gains one more crate to compile; operators have one more binary to launch (`cortex-classifier-worker` joins embedder/graph/fulltext as a sibling worker). The boot order does not matter because the publisher auto-creates Synap rooms on the first "Room not found" error.
