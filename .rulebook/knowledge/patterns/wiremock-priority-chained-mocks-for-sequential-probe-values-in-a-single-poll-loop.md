# wiremock priority-chained mocks to simulate a sequence of probe values across polls

**Category**: testing
**Tags**: wiremock, integration-test, synap, cortex-workers, stream.stats, mock-sequencing

## Description

When a test needs the SAME endpoint to answer differently across successive
calls (e.g. a `stream.stats` probe that must first report a HIGHER
`total_published` then a LOWER one to exercise a decrease-detection heal),
`wiremock` 0.6 resolves ties between equal-priority mocks by MOUNT ORDER: the
first-mounted matching mock wins until its `up_to_n_times(N)` quota is
exhausted, then the next-mounted matching mock takes over. Mount the
"first answer" with `.up_to_n_times(1)` and the "later answer" unbounded,
both using the exact same matcher (`body_string_contains("stream.stats")`).
No custom fake-server state machine is needed — this is the same pattern
already used by the room-not-found → get_or_create → retry test earlier in
the same file (`consume_room_not_found_redeclares_and_retries_within_one_poll`).

## Example

    Mock::given(method("POST")).and(path("/api/v1/command"))
        .and(body_string_contains("stream.stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(/* published */ 5)))
        .up_to_n_times(1)
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/api/v1/command"))
        .and(body_string_contains("stream.stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(/* published */ 1)))
        .mount(&server).await;
    // poll 1 sees published=5 (remembers baseline), poll 2 sees published=1 (decrease → heal)

## When to Use

Integration tests against a `wiremock::MockServer` where the code under test
polls the SAME endpoint repeatedly across a `while`/loop-driven consumer and
the test needs to assert behavior that only differs on the Nth call
(sequential probe values, retry-then-succeed, degrade-then-recover).

## When NOT to Use

When the differing calls carry a distinguishable request body (e.g. a
different `from_offset` value) — in that case a plain, unbounded
`body_string_contains` match per distinct body value is simpler and needs no
`up_to_n_times` bookkeeping.
