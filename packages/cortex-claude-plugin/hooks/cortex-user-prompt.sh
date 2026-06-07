#!/usr/bin/env bash
# cortex-user-prompt — UserPromptSubmit shim. Spec 10.
set -u

if [ "${CORTEX_ADAPTER_DISABLE:-0}" = "1" ]; then echo "{}"; exit 0; fi

EVENT="UserPromptSubmit"
SYNCHRONOUS=1

BIN="${CORTEX_HOOK_BIN:-}"
if [ -z "${BIN}" ] && command -v cortex-hook >/dev/null 2>&1; then
    BIN="cortex-hook"
fi
if [ -z "${BIN}" ]; then
    for cand in "$HOME/.cortex/cortex-hook" "$HOME/.cortex/cortex-hook.exe"; do
        if [ -x "$cand" ]; then BIN="$cand"; break; fi
    done
fi
if [ -n "${BIN}" ]; then
    if [ "${SYNCHRONOUS}" = "1" ]; then
        exec "${BIN}" "${EVENT}"
    else
        exec "${BIN}" "${EVENT}" --fire-forget
    fi
fi

SOCK="${CORTEX_ADAPTER_SOCK:-$HOME/.cortex/adapter-claude.sock}"
INPUT="$(cat || true)"
PAYLOAD=$(printf '{"hook":"%s","session_id":"%s","cwd":"%s","payload":%s}' \
    "${EVENT}" "${CLAUDE_SESSION_ID:-}" "$PWD" "${INPUT:-{}}")
if command -v nc >/dev/null 2>&1 && [ -S "$SOCK" ]; then
    RESPONSE=$(printf '%s\n' "$PAYLOAD" | nc -U -w1 "$SOCK" 2>/dev/null || true)
elif command -v socat >/dev/null 2>&1 && [ -S "$SOCK" ]; then
    RESPONSE=$(printf '%s\n' "$PAYLOAD" | socat - "UNIX-CONNECT:$SOCK" 2>/dev/null || true)
else
    RESPONSE=""
fi
printf '%s\n' "${RESPONSE:-{}}"
exit 0
