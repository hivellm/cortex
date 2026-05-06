#!/usr/bin/env bash
# cortex-session-start — pipe Claude Code SessionStart JSON to the
# local cortex adapter daemon. Spec 10 §Hook ↔ daemon protocol.
# Never break the session: any failure prints `{}` and exits 0.
set -u

# Cortex sub-workers (classifier-cli, …) opt out via CORTEX_ADAPTER_DISABLE.
if [ "${CORTEX_ADAPTER_DISABLE:-0}" = "1" ]; then echo "{}"; exit 0; fi

SOCK="${CORTEX_ADAPTER_SOCK:-$HOME/.cortex/adapter-claude.sock}"
INPUT="$(cat || true)"
PAYLOAD=$(printf '{"hook":"SessionStart","session_id":"%s","cwd":"%s","payload":%s}' \
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
