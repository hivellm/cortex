# Exit 0 as a "supervisor-friendly" disabled state under docker restart:always/unless-stopped

**Category**: architecture
**Tags**: docker, restart-policy, worker, classifier, supervisor, anti-pattern, stale-container-env

## Description

A worker binary that handles a "disabled" config by exiting cleanly (return Ok(())/exit 0) on the assumption that supervisors won't churn a clean exit. This is FALSE for docker's two most common restart policies: `restart: always` and `restart: unless-stopped` restart the container on ANY exit code — only `restart: on-failure` respects exit 0. The result is a per-restart-interval loop (RestartCount climbs forever) for a process that was meant to sit idle. systemd has the same trap unless `Restart=on-failure` is set (the default is `Restart=no`, but `Restart=always` units exhibit the identical churn). Fix: when a long-running service is intentionally disabled, PARK the process (await a shutdown signal: `tokio::signal::ctrl_c().await`, or block on a never-completing future) instead of exiting. The container/unit stays Up and quiet; `docker stop`/SIGTERM still terminates it via default signal disposition. Compounding gotcha: a container created with a stale env override (e.g. CORTEX_CLASSIFIER_MODE=disabled baked at creation) keeps that env across docker *restarts* — restarts never re-read .env or compose ${VAR} substitution. Recreating (`docker compose up -d --force-recreate <svc>`) is required to pick up a changed .env; a restart alone won't.

## Example

// WRONG — churns under restart: unless-stopped
if matches!(config.mode, Mode::Disabled) { return Ok(()); }
// RIGHT — park until a stop signal; container stays Up, no churn
if matches!(config.mode, Mode::Disabled) {
    tracing::info!("disabled — idling; send SIGINT/SIGTERM to stop");
    tokio::signal::ctrl_c().await.ok();
    return Ok(());
}
// crates/cortex-workers/src/bin/classifier-worker.rs

## When to Use

When a daemon/worker has an operator escape hatch to run in a no-op/disabled state but is launched under a process supervisor with an always-restart policy.

## When NOT to Use

If the supervisor is explicitly `restart: on-failure` (or a one-shot job), exit 0 is correct and idling would wrongly hold resources. Match the chosen disabled-state behavior to the actual restart policy.
