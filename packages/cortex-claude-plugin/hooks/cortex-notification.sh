#!/usr/bin/env bash
# Cortex Notification hook launcher, shared by the Claude Code plugin and the standalone
# install. The native cortex-hook owns bounded I/O, session identity, on-demand
# runtime start and the once-per-session notice. Without a binary this drains
# stdin, replies with an empty JSON object and exits 0, never blocking.
set -u

EVENT="Notification"
SYNCHRONOUS=0

if [ "${CORTEX_ADAPTER_DISABLE:-0}" = "1" ]; then cat >/dev/null; printf '{}\n'; exit 0; fi

# Only absolute overrides are honoured: a relative path would resolve against
# the checkout, letting a repository ship the binary every event is handed to.
absolute() { case "$1" in /* | [A-Za-z]:[\\/]*) return 0 ;; *) return 1 ;; esac; }

BIN=""
OPTION_DIR="${CLAUDE_PLUGIN_OPTION_BIN_DIR:-}"
OPTION_DIR="${OPTION_DIR%/}"
OPTION_DIR="${OPTION_DIR%\\}"
if [ -n "${CORTEX_HOOK_BIN:-}" ] && absolute "$CORTEX_HOOK_BIN"; then
    BIN="$CORTEX_HOOK_BIN"
elif [ -n "$OPTION_DIR" ] && absolute "$OPTION_DIR"; then
    BIN="$OPTION_DIR/cortex-hook"
    [ -f "$BIN" ] || BIN="$OPTION_DIR/cortex-hook.exe"
fi
# An explicit override never falls back to another binary.
if [ -n "$BIN" ] && [ ! -f "$BIN" ]; then cat >/dev/null; printf '{}\n'; exit 0; fi
if [ -z "$BIN" ] && command -v cortex-hook >/dev/null 2>&1; then
    BIN="cortex-hook"
fi
if [ -z "$BIN" ]; then
    # A GUI-launched client may not have the install directory on PATH.
    for cand in "${HOME:-}/.cortex/bin/cortex-hook" "${HOME:-}/.cortex/bin/cortex-hook.exe" \
        "${HOME:-}/.cortex/cortex-hook" "${HOME:-}/.cortex/cortex-hook.exe"; do
        if [ -f "$cand" ]; then BIN="$cand"; break; fi
    done
fi
if [ -z "$BIN" ]; then cat >/dev/null; printf '{}\n'; exit 0; fi

if [ -n "${CLAUDE_PLUGIN_ROOT:-}" ]; then
    # Lets cortex-hook skip events that legacy settings hooks already deliver.
    CORTEX_HOOK_ORIGIN=plugin
    export CORTEX_HOOK_ORIGIN
    if [ -n "${CLAUDE_PLUGIN_OPTION_API_URL:-}" ]; then
        CORTEX_API_URL="$CLAUDE_PLUGIN_OPTION_API_URL"
        export CORTEX_API_URL
    fi
fi

exec "$BIN" "$EVENT" --fire-forget
