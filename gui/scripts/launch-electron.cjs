#!/usr/bin/env node
// Spawn the Electron binary with a clean environment.
//
// `ELECTRON_RUN_AS_NODE` is set in some shells (Claude Code's bash
// env among them) which forces electron.exe into plain Node mode —
// `process.type` becomes undefined and `require("electron")` returns
// the path string instead of the API. We can't undo that with
// `cross-env ELECTRON_RUN_AS_NODE=` because Electron checks for the
// var's *presence*, not its value. This launcher actually deletes it
// from the spawned env.

const { spawn } = require("child_process");
const path = require("path");

const env = { ...process.env };
delete env.ELECTRON_RUN_AS_NODE;
env.CORTEX_GUI_DEV = env.CORTEX_GUI_DEV ?? "1";

const electronBinary = require("electron");
const args = [path.resolve(__dirname, "..")];

const child = spawn(electronBinary, args, { stdio: "inherit", env, windowsHide: false });
child.on("close", (code, signal) => {
  if (code === null) {
    console.error("electron exited with signal", signal);
    process.exit(1);
  }
  process.exit(code);
});

const handleSignal = (sig) => {
  process.on(sig, () => {
    if (!child.killed) child.kill(sig);
  });
};
handleSignal("SIGINT");
handleSignal("SIGTERM");
