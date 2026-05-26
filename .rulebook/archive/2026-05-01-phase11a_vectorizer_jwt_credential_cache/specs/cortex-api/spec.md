# cortex-api — vectorizer JWT credential cache

## ADDED Requirements

### Requirement: Authenticated probe at boot
The `cortex-api` daemon SHALL run an authenticated round-trip against the Vectorizer when `CORTEX_VECTORIZER_USER` + `CORTEX_VECTORIZER_PASSWORD` (or the `_EMBEDDER_*` aliases, or `_API_KEY`) are set, before declaring the live vector lane ready.

#### Scenario: Authenticated probe succeeds → live lane wired
Given `CORTEX_VECTORIZER_URL` is set
And `CORTEX_VECTORIZER_USER` and `CORTEX_VECTORIZER_PASSWORD` are set to valid credentials
When `cortex-api` boots
Then `VectorizerLane::with_login(...)` mints a JWT
And `probe_authenticated()` returns `Ok(())`
And the live lane is installed in the orchestrator

#### Scenario: Authenticated probe returns 401 once → refresh and retry
Given creds are cached on the lane
And the first authenticated probe call returns HTTP 401
When the boot path observes the 401
Then it calls `refresh_token()` once
And retries the authenticated probe
And on success the live lane is installed

#### Scenario: Authenticated probe still 401 after refresh → fall back loudly
Given creds are cached on the lane
And both the initial probe and the post-refresh retry return HTTP 401
When the boot path exhausts the retry
Then it logs `ERROR` with the resolved URL and the username
And the orchestrator falls back to `MemoryVectorLane`
And the daemon stays up

### Requirement: Boot warning when URL is set but creds are not
The `cortex-api` daemon SHALL emit a `WARN`-level log at boot when `CORTEX_VECTORIZER_URL` is set but no credentials are configured, naming every env key it checked.

#### Scenario: Anonymous boot against an authenticated server
Given `CORTEX_VECTORIZER_URL` is set
And no `_API_KEY`, `_USER`, `_PASSWORD`, `_EMBEDDER_VECTORIZER_USER`, `_EMBEDDER_VECTORIZER_PASSWORD` env values are present
When `cortex-api` boots
Then a `WARN` log records "every authenticated search will return 401"
And the log lists the env keys checked
And the live lane is wired in anonymous mode (current behaviour preserved)

### Requirement: Optional periodic JWT warmup
The `cortex-api` daemon SHALL support an optional periodic JWT refresh, gated on `CORTEX_VECTORIZER_JWT_WARMUP_SECS`.

#### Scenario: Warmup disabled by default
Given `CORTEX_VECTORIZER_JWT_WARMUP_SECS` is unset or `0`
When `cortex-api` boots
Then no warmup task is spawned
And refresh continues to be triggered reactively on 401 in `vectorizer_lane.rs::search`

#### Scenario: Warmup enabled
Given `CORTEX_VECTORIZER_JWT_WARMUP_SECS=3000` (50 min)
And creds are cached on the lane
When `cortex-api` boots
Then a `tokio::spawn`-ed task calls `VectorizerLane::refresh_token()` every 3000 seconds
And the task exits cleanly on the daemon's shutdown signal
