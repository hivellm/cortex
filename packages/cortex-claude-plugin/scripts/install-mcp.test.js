"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

function printedEntry(apiUrl) {
  const result = spawnSync(process.execPath, [path.join(__dirname, "install-mcp.js"), "--print"], {
    env: { ...process.env, CORTEX_API_URL: apiUrl },
    encoding: "utf8",
    timeout: 2000,
  });
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout);
}

test("installer standalone command matches packaged MCP configuration", () => {
  const packaged = JSON.parse(fs.readFileSync(path.join(__dirname, "../.mcp.json"), "utf8"));
  assert.deepEqual(printedEntry(""), packaged.mcpServers.cortex);
});

test("standalone entry ignores legacy daemon configuration", () => {
  assert.deepEqual(printedEntry("http://127.0.0.1:18000"), { type: "stdio", command: "cortex", args: ["mcp"] });
});
