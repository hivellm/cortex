Mechanical `crate::` → `super::` rewrites during a crate-merge refactor break inside nested `mod tests` blocks because the depth shifts.

Concrete example from phase11s §4 (cortex-consolidator merge):

- File `consolidator/orchestrator.rs` at depth 1 from crate root.
  - Original `crate::summariser` (resolves to crate-root summariser at top of cortex-consolidator).
  - Post-merge target: `cortex_workers::consolidator::summariser`.
  - File-level rewrite: `crate::summariser` → `super::summariser` (super = consolidator). Correct.
  - Inside the file's `#[cfg(test)] mod tests {}` (depth +1): the same `super::summariser` now resolves to `orchestrator::summariser` which doesn't exist. Needs `super::super::summariser`.
- File `consolidator/producer/session.rs` at depth 2.
  - File-level: `crate::summariser` → `super::super::summariser` (super::super = consolidator). Correct.
  - Inside `mod tests` (depth +1 = 3): needs `super::super::super::summariser`.

A blind `sed -i 's|crate::summariser|super::summariser|g'` works at file-level but breaks every test-mod reference. A second blind `sed -i 's|super::summariser|super::super::summariser|g'` then breaks the file-level back.

Workable approach: split the rewrite by depth boundary. Find the line where `mod tests {` starts, then `sed -i "1,${cutoff} s/old/new1/g; ${cutoff},\$ s/old/new2/g" <file>` so the test mod gets one extra `super::` hop.

Alternative (cleaner for one-shot refactors): use absolute `crate::<full_path>::*` inside test mods so the path is explicit and depth-independent. Trades terseness for stability.

The phase11s §4 commit body documents the depth-aware split per file. Future merges that move modules into a deeper nesting should follow the same pattern.

The post-merge cold workspace build (`cargo build --workspace --all-features --bins`) was 1 min 14 s on the developer machine (Windows 10, single Rust toolchain, with reqwest TLS deps cached). A clean pre-merge baseline was not captured because the merge was already in progress when §6 started measuring; the empirical comparison the §6.4 plan called for is therefore deferred to the next workspace cold-build that follows a structural change.
