#!/usr/bin/env bash
# phase28_docs-truth-reconciliation §1.7 — fail when two files in
# docs/specs/ share a leading spec number (e.g. `20-foo.md` and
# `20-bar.md`). The same invariant is enforced at `cargo test` time by
# crates/cortex-cli/tests/spec_numbering.rs; this script is the
# shell-native variant for CI steps and pre-commit hooks.
set -euo pipefail

cd "$(dirname "$0")/.."

dupes=$(ls docs/specs/ \
  | grep -E '^[0-9]+-.*\.md$' \
  | sed -E 's/^([0-9]+)-.*/\1/' \
  | sort \
  | uniq -d)

if [ -n "$dupes" ]; then
  echo "FAIL: duplicate spec numbers in docs/specs/:" >&2
  for n in $dupes; do
    ls docs/specs/ | grep -E "^${n}-" | sed 's/^/  /' >&2
  done
  exit 1
fi

echo "ok: every spec number in docs/specs/ maps to exactly one file"
