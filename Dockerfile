# syntax=docker/dockerfile:1.7

# Phase10k+ — single multi-stage image that builds every cortex
# service binary in one pass, then forks into per-binary runtime
# stages docker-compose can target via `target:`.
#
# Build once with `docker compose build`; the BuildKit cache mounts
# below keep the cargo registry / git index / target dir warm across
# rebuilds so iteration is fast.

# -----------------------------------------------------------------
# Builder — Debian Trixie dev image with Rust installed via rustup.
# Using dhi.io/debian-base:trixie-dev keeps the builder and the
# runtime stage on the same libc / libssl ABI, so a binary built
# here always matches what the runtime layer ships.
# -----------------------------------------------------------------
FROM dhi.io/debian-base:trixie-dev AS builder

# Phase11e hotfix — git SHA / dirty flag forwarded from the host so
# `cortex-build`'s `emit_version_env` stamps real values in the
# `/healthz` version block. Without these, the build context lacks
# `.git/` and every container reports `sha=unknown`. Defaults stay
# `unknown` / `false` so a bare `docker build` still works.
ARG CORTEX_GIT_SHA=unknown
ARG CORTEX_GIT_DIRTY=false

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    RUST_VERSION=1.93.1 \
    CORTEX_GIT_SHA_OVERRIDE=$CORTEX_GIT_SHA \
    CORTEX_GIT_DIRTY_OVERRIDE=$CORTEX_GIT_DIRTY

# Native deps the workspace pulls in via build.rs / sys crates:
# - pkg-config + libssl-dev → reqwest's TLS via native-tls (some
#   transitive deps still link OpenSSL).
# - protobuf-compiler        → tonic / prost build scripts.
# - cmake + git              → zstd-sys, parquet, rusqlite bundled.
# - gcc + g++ + make         → cc-rs default toolchain for every
#   *-sys crate. (Previously this list included `clang`, but the
#   `dhi.io/debian-base:trixie-dev` mirror temporarily ships
#   `clang:amd64=1:19.0-63dhi0` which transitively pulls
#   `libobjc-14-dev` and forces a downgrade of
#   `gcc-14-base:amd64=14.2.0-19` against the already-installed
#   `14.2.0-19dhi0`. The workspace builds with gcc fine; clang was
#   redundant.)
# - curl                     → rustup bootstrap.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates curl \
        protobuf-compiler cmake git \
        gcc g++ make \
 && rm -rf /var/lib/apt/lists/* \
 && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain "$RUST_VERSION" --profile minimal \
 && rustc --version

WORKDIR /usr/src/cortex

# Pull the manifest set first so the cargo cache layer can be reused
# when only source changes. Keep this list tight — every Cargo.toml
# in the workspace is needed for `cargo fetch`.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY docs ./docs
COPY .rulebook ./.rulebook

# Build every binary the compose stack uses in a single cargo
# invocation. BuildKit cache mounts keep the registry + target dir
# warm across rebuilds.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/src/cortex/target \
    cargo build --release \
        --bin cortex-api \
        --bin cortex-ingestion \
        --bin cortex-adapter-claude \
        --bin cortex-classifier-worker \
        --bin cortex-embedder-worker \
        --bin cortex-fulltext-worker \
        --bin cortex-graph-worker \
        --bin cortex-ops \
        --bin cortex-consolidator \
        --bin cortex-mcp-server \
 && cargo build --release \
        --features cortex-workers/claude-archive \
        -p cortex-workers \
        --bin cortex-claude-archive \
 && mkdir -p /out \
 && cp target/release/cortex-api \
       target/release/cortex-ingestion \
       target/release/cortex-adapter-claude \
       target/release/cortex-classifier-worker \
       target/release/cortex-embedder-worker \
       target/release/cortex-fulltext-worker \
       target/release/cortex-graph-worker \
       target/release/cortex-ops \
       target/release/cortex-consolidator \
       target/release/cortex-mcp-server \
       target/release/cortex-claude-archive \
       /out/

# -----------------------------------------------------------------
# Common runtime — same dhi.io trixie base as the builder so libc
# and libssl ABIs match. `-dev` keeps shell tooling for in-container
# debugging while still being slimmer than the full distro image.
# -----------------------------------------------------------------
FROM dhi.io/debian-base:trixie-dev AS runtime-base
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates curl libssl3 \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /opt/cortex

# -----------------------------------------------------------------
# Per-binary leaf stages — docker-compose targets these by name.
# -----------------------------------------------------------------
FROM runtime-base AS cortex-api
COPY --from=builder /out/cortex-api /usr/local/bin/cortex-api
COPY --from=builder /out/cortex-ops /usr/local/bin/cortex-ops
# phase11x — bundle `cortex-consolidator` so the cron's
# `retention.consolidator_nightly` (`cortex-consolidator nightly`)
# actually finds the binary on `$PATH`. Pre-fix the cron failed
# with `sh: line 1: cortex-consolidator: command not found`.
COPY --from=builder /out/cortex-consolidator /usr/local/bin/cortex-consolidator
EXPOSE 17000
HEALTHCHECK --interval=10s --timeout=3s --start-period=20s --retries=6 \
    CMD curl -fsS http://127.0.0.1:17000/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cortex-api"]

FROM runtime-base AS cortex-ingestion
COPY --from=builder /out/cortex-ingestion /usr/local/bin/cortex-ingestion
EXPOSE 17010
HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=6 \
    CMD curl -fsS http://127.0.0.1:17010/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cortex-ingestion"]

FROM runtime-base AS cortex-adapter
COPY --from=builder /out/cortex-adapter-claude /usr/local/bin/cortex-adapter-claude
EXPOSE 17011
HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=6 \
    CMD curl -fsS http://127.0.0.1:17011/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cortex-adapter-claude"]

FROM runtime-base AS cortex-classifier-worker
COPY --from=builder /out/cortex-classifier-worker /usr/local/bin/cortex-classifier-worker
EXPOSE 17021
HEALTHCHECK --interval=10s --timeout=3s --start-period=20s --retries=6 \
    CMD curl -fsS http://127.0.0.1:17021/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cortex-classifier-worker"]

FROM runtime-base AS cortex-embedder-worker
COPY --from=builder /out/cortex-embedder-worker /usr/local/bin/cortex-embedder-worker
EXPOSE 17022
HEALTHCHECK --interval=10s --timeout=3s --start-period=20s --retries=6 \
    CMD curl -fsS http://127.0.0.1:17022/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cortex-embedder-worker"]

FROM runtime-base AS cortex-fulltext-worker
COPY --from=builder /out/cortex-fulltext-worker /usr/local/bin/cortex-fulltext-worker
EXPOSE 17023
HEALTHCHECK --interval=10s --timeout=3s --start-period=20s --retries=6 \
    CMD curl -fsS http://127.0.0.1:17023/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cortex-fulltext-worker"]

FROM runtime-base AS cortex-graph-worker
COPY --from=builder /out/cortex-graph-worker /usr/local/bin/cortex-graph-worker
# The graph-worker reads cypher templates from disk at startup. Ship
# them with the image and point the binary at the in-image path via
# `CORTEX_GRAPH_CYPHER_DIR` (set by docker-compose).
COPY crates/cortex-workers/cypher /opt/cortex/cypher
EXPOSE 17024
HEALTHCHECK --interval=10s --timeout=3s --start-period=20s --retries=6 \
    CMD curl -fsS http://127.0.0.1:17024/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cortex-graph-worker"]

# phase11i §5.1 — long-running watcher that tails the Claude Code
# JSONL conversation archive (`~/.claude/projects/`) and emits one
# canonical envelope per turn / tool_call / agent_call. The data
# path is bind-mounted read-only by docker-compose. Health endpoint
# `:17030/healthz` lands in §5.2; the EXPOSE here keeps the image
# forward-compatible.
FROM runtime-base AS cortex-claude-archive
COPY --from=builder /out/cortex-claude-archive /usr/local/bin/cortex-claude-archive
EXPOSE 17030
HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=6 \
    CMD curl -fsS http://127.0.0.1:17030/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cortex-claude-archive"]
CMD ["tail", "--root", "/data/claude-projects", "--projects-only", "--sink", "archive", "--archive-root", "/var/lib/cortex/archive"]

# phase14a §3.4 — long-running daemon that subscribes to the Synap
# trigger stream (`cortex.consolidator.triggers`) and dispatches each
# trigger to the matching grain (Session / Topic / DecisionTrace).
# The daemon does not own an HTTP surface itself — its health view
# lives on cortex-api's `/v1/health/consolidator` (phase14a §4.1).
# The container-side healthcheck therefore probes process liveness;
# operators read run quality through the cortex-api endpoint.
FROM runtime-base AS cortex-consolidator
COPY --from=builder /out/cortex-consolidator /usr/local/bin/cortex-consolidator
RUN apt-get update && apt-get install -y --no-install-recommends procps \
    && rm -rf /var/lib/apt/lists/*
HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=6 \
    CMD pgrep -f cortex-consolidator >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cortex-consolidator"]
CMD ["daemon"]
