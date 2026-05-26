#!/usr/bin/env bash
# Phase11w §8.1 — Cortex OpenCode integration installer.
#
# Verifies `opencode` is on PATH, ensures the daemon HTTP listener
# binding is configured, and prints next-step instructions. Idempotent
# — re-running never overwrites files that already exist.

set -euo pipefail

CORTEX_REPO_ROOT="${CORTEX_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
CORTEX_ADAPTER_HTTP_BIND_DEFAULT="127.0.0.1:17004"

err() { printf "\033[31merror\033[0m: %s\n" "$1" >&2; }
info() { printf "\033[32mok\033[0m: %s\n" "$1"; }
warn() { printf "\033[33mwarn\033[0m: %s\n" "$1"; }

# 1. Verify required binaries.
if ! command -v opencode >/dev/null 2>&1; then
  err "opencode CLI not found on PATH. Install from https://opencode.ai/docs/install"
  exit 1
fi
info "opencode CLI: $(command -v opencode)"

if ! command -v cortex-adapter-claude >/dev/null 2>&1; then
  warn "cortex-adapter-claude binary not on PATH; build it with 'cargo build --release -p cortex-adapter-claude-code'"
fi

# 2. Confirm the project config exists.
if [ ! -f "${CORTEX_REPO_ROOT}/opencode.json" ]; then
  err "opencode.json missing at ${CORTEX_REPO_ROOT}. Re-run 'rulebook task apply phase11w_opencode-adapter'."
  exit 1
fi
info "opencode.json present"

# 3. Confirm the agents + commands directories ship.
for d in .opencode/agents .opencode/commands; do
  if [ ! -d "${CORTEX_REPO_ROOT}/${d}" ]; then
    err "${d} missing under ${CORTEX_REPO_ROOT}"
    exit 1
  fi
done
info ".opencode/{agents,commands} present"

# 4. Resolve the HTTP bind the plugin will POST to.
http_bind="${CORTEX_ADAPTER_HTTP_BIND:-$CORTEX_ADAPTER_HTTP_BIND_DEFAULT}"
info "daemon http bind: ${http_bind}"

cat <<EOF

Next steps:
  1. Start the Cortex daemon with the HTTP transport:
       CORTEX_ADAPTER_HTTP_BIND=${http_bind} cortex-adapter-claude daemon
  2. From this repo, launch OpenCode:
       opencode
  3. Inside the OpenCode TUI, verify the cortex tools are listed:
       /mcp list
  4. Smoke-test the plugin: submit a prompt and confirm the assistant
     receives a pre-thinking bundle (look for the "## active work" or
     "## consolidations" sections in the assistant's reasoning context).

Uninstall: scripts/install-opencode.sh --uninstall
EOF

if [ "${1:-}" = "--uninstall" ]; then
  warn "uninstall flag set; this script does not delete .opencode/ or opencode.json on its own"
  warn "remove them manually: rm -r .opencode opencode.json"
fi
