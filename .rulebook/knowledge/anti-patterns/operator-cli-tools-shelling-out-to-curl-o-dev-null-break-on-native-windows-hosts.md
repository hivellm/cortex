# Operator CLI tools shelling out to `curl -o /dev/null` break on native Windows hosts

**Category**: ci
**Tags**: cortex, ci, windows, analysis:cortex-platform-2026-07

## Description

`cortex-ops doctor` (crates/cortex-cli/src/bin/cortex-ops/doctor.rs) shells out to the system curl with `-o /dev/null` to discard the response body. On native Windows (not WSL), `/dev/null` is not a real path, so libcurl returns error 23 ("failure writing output") on every single health check — even though the target services are demonstrably healthy (independently confirmed via direct curl and doctor-config.sh in the same session). The identical `curl ... >/dev/null` idiom works fine in this project's own Linux container HEALTHCHECK lines, which masked the bug since it only manifests when the CLI itself runs on a native Windows host rather than inside a container.

## Example

Bad: `Command::new("curl").args(["-o", "/dev/null", ...])`. Better: use a platform sink (`NUL` on Windows, `/dev/null` on POSIX) or, better still, make the HTTP call in-process (reqwest) and discard the body in Rust instead of shelling out at all.

## When to Use

Writing cross-platform operator/CLI tooling in Rust (or any language) that shells out to curl or other POSIX-path-assuming commands, especially when the same codebase also runs inside Linux containers.
