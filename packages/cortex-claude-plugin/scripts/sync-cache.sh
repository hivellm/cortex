#!/usr/bin/env bash
# Sync the cortex-claude-plugin source tree into Claude Code's plugin cache.
#
# Workaround for a Claude Code limitation: when a plugin marketplace
# uses `source: { type: directory, path: ... }`, `claude plugin install`
# only copies the manifest files (plugin.json, marketplace.json) into
# the cache directory at
#   ~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/
# The runtime then reads from that cache, so anything not in the
# cache (hooks.json, hooks/, skills/, agents/, commands/, .mcp.json,
# README.md) is invisible to the Claude Code loader.
#
# This script copies the missing assets into the cache so the loader
# sees the full plugin tree. Run it after every `pnpm` / `git pull`
# that changes the plugin assets, then restart Claude Code so the
# loader picks up the new files.
#
# Usage:
#   bash packages/cortex-claude-plugin/scripts/sync-cache.sh
#
# Override the cache root via CORTEX_PLUGIN_CACHE if your install
# lives elsewhere (e.g. a non-default $HOME).

set -euo pipefail

PLUGIN_NAME="hivellm-cortex"
PLUGIN_SUBDIR="cortex"
PLUGIN_VERSION="0.1.0"

# Resolve plugin source dir: this script's parent's parent.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PLUGIN_SRC="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ -n "${CORTEX_PLUGIN_CACHE:-}" ]; then
    CACHE_BASE="$CORTEX_PLUGIN_CACHE"
elif [ -n "${USERPROFILE:-}" ]; then
    # Git Bash on Windows: $USERPROFILE points at the user home.
    CACHE_BASE="${USERPROFILE//\\//}/.claude/plugins/cache"
elif [ -n "${HOME:-}" ]; then
    CACHE_BASE="$HOME/.claude/plugins/cache"
else
    echo "error: cannot resolve cache base — set CORTEX_PLUGIN_CACHE" >&2
    exit 1
fi

CACHE_DIR="$CACHE_BASE/$PLUGIN_NAME/$PLUGIN_SUBDIR/$PLUGIN_VERSION"

if [ ! -d "$CACHE_DIR" ]; then
    echo "error: cache dir does not exist — run \`claude plugin install cortex@hivellm-cortex\` first" >&2
    echo "expected: $CACHE_DIR" >&2
    exit 1
fi

echo "syncing $PLUGIN_SRC → $CACHE_DIR"

for asset in hooks skills agents commands README.md .claude-plugin .mcp.json; do
    src="$PLUGIN_SRC/$asset"
    if [ -e "$src" ]; then
        cp -r "$src" "$CACHE_DIR/"
        echo "  copied $asset"
    fi
done

echo
echo "done. Restart Claude Code so the loader picks up the new assets."
