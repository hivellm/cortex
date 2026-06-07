#!/usr/bin/env node
// Install the cortex MCP server at user scope in ~/.claude.json so every
// Claude Code project picks it up without per-project opt-in. Idempotent;
// running twice is safe.
//
// Usage:
//   node install-mcp.js            install (or refresh) the entry
//   node install-mcp.js --remove   delete the cortex entry
//   node install-mcp.js --print    print the resolved entry without writing

"use strict";

const fs = require("fs");
const path = require("path");

const home = process.env.USERPROFILE || process.env.HOME;
if (!home) {
  console.error("error: cannot resolve home directory (USERPROFILE/HOME unset)");
  process.exit(2);
}

const target = path.join(home, ".claude.json");
const flag = process.argv[2] || "";
const remove = flag === "--remove" || flag === "--uninstall";
const printOnly = flag === "--print";

const entry = {
  type: "stdio",
  command: "cortex-mcp-server",
  args: ["serve"],
  env: {
    CORTEX_API_URL: process.env.CORTEX_API_URL || "http://127.0.0.1:15011",
    CORTEX_ADAPTER_SOCK:
      process.env.CORTEX_ADAPTER_SOCK || "~/.cortex/adapter-claude.sock",
  },
};

if (printOnly) {
  console.log(JSON.stringify(entry, null, 2));
  process.exit(0);
}

if (!fs.existsSync(target)) {
  console.error("error: " + target + " does not exist");
  console.error("       open Claude Code at least once before running this installer");
  process.exit(2);
}

const ts = new Date().toISOString().replace(/[:.]/g, "-");
const backup = target + ".bak." + ts;
fs.copyFileSync(target, backup);

const cfg = JSON.parse(fs.readFileSync(target, "utf8"));
cfg.mcpServers = cfg.mcpServers || {};

const had = Boolean(cfg.mcpServers.cortex);
if (remove) {
  if (!had) {
    console.log("nothing to remove: mcpServers.cortex is not set");
    process.exit(0);
  }
  delete cfg.mcpServers.cortex;
} else {
  cfg.mcpServers.cortex = entry;
}

const tmp = target + ".tmp." + process.pid;
fs.writeFileSync(tmp, JSON.stringify(cfg, null, 2));
JSON.parse(fs.readFileSync(tmp, "utf8"));
fs.renameSync(tmp, target);

console.log("backup: " + backup);
if (remove) {
  console.log("removed mcpServers.cortex from " + target);
} else {
  console.log(
    (had ? "refreshed" : "installed") + " mcpServers.cortex at " + target,
  );
  console.log(JSON.stringify(entry, null, 2));
  console.log("");
  console.log("next: run /clear in Claude Code (or reopen the IDE) so the");
  console.log("      mcp__cortex__* tools become available in every project.");
}
