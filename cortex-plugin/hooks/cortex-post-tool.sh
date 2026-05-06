#!/usr/bin/env bash
# cortex-post-tool — PostToolUse shim. Spec 10.
set -u

# Cortex sub-workers (classifier-cli, …) export CORTEX_ADAPTER_DISABLE=1
# so this shim short-circuits and the subprocess hooks never reach the
# bus — see crates/cortex-classifier/src/haiku_cli.rs for the rationale.
if [ "${CORTEX_ADAPTER_DISABLE:-0}" = "1" ]; then echo "{}"; exit 0; fi

SOCK="${CORTEX_ADAPTER_SOCK:-$HOME/.cortex/adapter-claude.sock}"
INPUT="$(cat || true)"
PAYLOAD=$(printf '{"hook":"PostToolUse","session_id":"%s","cwd":"%s","payload":%s}' \
    "${CLAUDE_SESSION_ID:-}" "$PWD" "${INPUT:-{}}")
if command -v nc >/dev/null 2>&1 && [ -S "$SOCK" ]; then
    RESPONSE=$(printf '%s\n' "$PAYLOAD" | nc -U -w1 "$SOCK" 2>/dev/null || true)
elif command -v socat >/dev/null 2>&1 && [ -S "$SOCK" ]; then
    RESPONSE=$(printf '%s\n' "$PAYLOAD" | socat - "UNIX-CONNECT:$SOCK" 2>/dev/null || true)
else
    RESPONSE=""
fi
printf '%s\n' "${RESPONSE:-{}}"
exit 0
