# phase4i — Jaccard probe with one-empty-pair silencing keeps lanes honest
**Source**: manual
**Date**: 2026-04-28
**Related Task**: phase4i_doctor_query_overlap_mode
**Tags**: phase4i, cortex-ops, doctor, probe, jaccard
When the doctor's probe mode computes pairwise Jaccards between Vec/Meili/Nexus search results, the threshold check must silence pairs where one lane returned zero results — otherwise an empty-corpus deployment surface trips the flag on every query and the operator becomes blind to real semantic drift. The rule that fires the flag is `obs.a_size > 0 && obs.b_size > 0 && obs.jaccard < threshold` (in `cortex_ops::probe::pair_below`); both-empty pairs default to Jaccard=1.0 by convention so they never fail. Empty-lane reporting belongs to the partition-coverage doctor (phase4d/h), not the overlap probe.

The Live impls each absorb their own transport errors and return an empty `Vec<String>` instead of propagating — this matches the QueryProbe contract that one bad lane should not poison the whole probe run. Result paths come from per-lane fields in this priority: hit.path → hit.id (for Meili / Vectorizer fall-back); Nexus uses `a.path` from a Cypher `CONTAINS` substring match on `Artifact.body` (escape `\\` and `"` in the query literal — basic but enough for the one-shot probe).

`run_query_probes` returns `Vec<QueryReport>`; the CLI then OR's the existing coverage `failed` flag with `any.below_threshold` so a single below-threshold query fails the doctor exit code.