#!/usr/bin/env bash
# cortex-post-tool — PostToolUse shim. Spec 10.
set -u

# Cortex sub-workers (classifier-cli, …) export CORTEX_ADAPTER_DISABLE=1
# so this shim short-circuits and the subprocess hooks never reach the
# bus — see crates/cortex-classifier/src/haiku_cli.rs for the rationale.
if [ "${CORTEX_ADAPTER_DISABLE:-0}" = "1" ]; then echo "{}"; exit 0; fi

# Polyglot: on Windows shells the daemon binds a named pipe and `nc -U`
# is unavailable, so re-exec the .ps1 sibling via pwsh. On Linux/macOS
# the case falls through and the Unix-socket path below runs unchanged.
case "${OSTYPE:-}" in
    msys*|cygwin*|win32*)
        exec pwsh -NoProfile -File "$(dirname "$0")/cortex-post-tool.ps1"
        ;;
esac

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
