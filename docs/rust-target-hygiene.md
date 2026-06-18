# Keeping `target/` small

Cargo's `target/` directory has no garbage collector. Object files,
incremental caches, and compiled `rlib`s from **old dependency versions**
accumulate indefinitely until something deletes them. On this workspace it
grew past **500 GB** before anyone cleaned it. This note documents how the
repo is configured to avoid that and how to keep it in check.

## Why it grows

1. **Full debuginfo (the biggest driver).** The dev profile defaults to
   *full* debug symbols for **every workspace crate and every dependency**.
   With 14 crates and dozens of integration-test binaries, debuginfo alone
   is most of `target/`.
2. **Three parallel build trees.** `target/debug` (dev), `target/release`
   (LTO release), and `target/llvm-cov-target` (coverage instrumentation)
   are independent full trees. Running tests + a release build + coverage
   triples the artifact volume.
3. **No automatic cleanup.** Stale artifacts from every dependency version
   you have ever built linger forever. Incremental caches grow with edits.

## What the repo already does (committed)

`Cargo.toml` profile settings (apply to everyone, no per-machine setup):

```toml
[profile.dev]
opt-level = 0
debug = "line-tables-only"   # was full debuginfo — keeps file:line in
                             # backtraces, slashes size, ~30-40% faster
                             # incremental rebuilds

[profile.release]
# ...
strip = true                 # drop the residual symbol table from
                             # release binaries
```

`line-tables-only` keeps panic/backtrace line numbers. If you need to
*step-debug* a crate in a debugger, opt back into full debuginfo locally
(do **not** commit this):

```toml
# .cargo/config.toml  (git-ignored, your machine only) — or a transient
# Cargo.toml override on the crate you are debugging:
[profile.dev]
debug = "full"
```

## Routine cleanup (do this periodically)

Install the tool once:

```bash
cargo install cargo-sweep
```

Then sweep artifacts you are not actively building (kept hot set stays, so
the next build is still incremental):

```bash
scripts/sweep-target.sh            # remove artifacts not accessed in 14 days
scripts/sweep-target.sh 7          # ...in 7 days
scripts/sweep-target.sh --dry-run  # preview only
```

Run it from a scheduler (cron / Task Scheduler) every week or two and
`target/` stays bounded.

## Reclaiming everything now (full rebuild next time)

```bash
cargo clean                  # nuke target/ entirely
cargo clean --release        # release artifacts only
cargo llvm-cov clean --workspace   # the coverage tree
cargo sweep --installed      # drop artifacts from toolchains you uninstalled
cargo sweep --maxsize 20GiB  # shrink target/ until under 20 GiB
```

## CI / Docker notes

- CI runners and the Docker build start from a clean cache, so **disable
  incremental compilation** there — it only adds artifacts and slows the
  build: set `CARGO_INCREMENTAL=0` in the CI/job environment.
- Docker layer caching already discards the in-container `target/` between
  builds, so container `target/` never accumulates on the host; the 500 GB
  was entirely from **local** `cargo` runs.

## References

- Disable/limit debuginfo to shrink `target/` and speed builds —
  <https://kobzol.github.io/rust/rustc/2025/05/20/disable-debuginfo-to-improve-rust-compile-times.html>
- `cargo-sweep` — <https://github.com/holmgr/cargo-sweep>
- Cargo profiles reference —
  <https://doc.rust-lang.org/cargo/reference/profiles.html>
- Reducing `target/` size (`-Zno-embed-metadata`, nightly) —
  <https://kobzol.github.io/rust/rustc/2025/06/02/reduce-cargo-target-dir-size-with-z-no-embed-metadata.html>
