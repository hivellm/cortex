#!/usr/bin/env bash
# Native transport owns bounded JSON I/O, session identity and outage notices.
# No native binary: continue the prompt with an empty response. Legacy nc/socat
# forwarding cannot guarantee the same bounds or once-per-session policy.
set -u

if [ "${CORTEX_ADAPTER_DISABLE:-0}" = "1" ]; then printf '{}\n'; exit 0; fi

EVENT="SessionStart"
SYNCHRONOUS=1
BIN="${CORTEX_HOOK_BIN:-}"
if [ -z "$BIN" ] && command -v cortex-hook >/dev/null 2>&1; then
    BIN="cortex-hook"
fi
if [ -z "$BIN" ]; then
    for cand in "${HOME:-}/.cortex/cortex-hook" "${HOME:-}/.cortex/cortex-hook.exe"; do
        if [ -x "$cand" ]; then BIN="$cand"; break; fi
    done
fi
if [ -n "$BIN" ] && { [ -x "$BIN" ] || command -v "$BIN" >/dev/null 2>&1; }; then
    # SessionStart must return the daemon's actual additionalContext.
    # Do not append --fire-forget, including on resumed sessions.
    exec "$BIN" "$EVENT"
fi
printf '{}\n'
exit 0
