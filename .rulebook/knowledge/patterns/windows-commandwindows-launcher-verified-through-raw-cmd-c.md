# Windows commandWindows launcher verified through raw cmd /C

**Category**: testing
**Tags**: windows, codex, hooks

## Description

phase36: Codex runs commandWindows as `%COMSPEC% /C "<command>"` after textual ${PLUGIN_ROOT} substitution. Write the command as `"${PLUGIN_ROOT}\scripts\x.cmd" <Event>`. It has more than two quotes, so cmd strips only the outer pair, and spaced roots and forward slashes both work. Test it with `Command::new(comspec).raw_arg(format!("/C \"{cmd}\""))`. Without `raw_arg`, Rust's MSVC quoting would escape the inner quotes. Use a fake cortex-hook.exe compiled with rustc into CARGO_TARGET_TMPDIR. In the batch file, use `where $PATH:name.exe` (PATH only, never the cwd), CRLF line endings pinned by .gitattributes, and `more >nul` to drain stdin on fail-open (1 MiB took 0.2 s). See crates/cortex-mcp-server/tests/hook_launcher.rs and spec 46.
