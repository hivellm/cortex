//! phase28_docs-truth-reconciliation §1.7 — every `NN-*.md` file in
//! `docs/specs/` must own its leading number exclusively. Four
//! historical collisions (20/26/27/28) were resolved on 2026-07-14;
//! this test keeps new ones from landing. The shell-native variant for
//! CI steps lives at `scripts/check-spec-numbering.sh`.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn specs_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/cortex-cli → workspace root is ../..
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("specs")
}

#[test]
fn every_spec_number_maps_to_exactly_one_file() {
    let dir = specs_dir();
    let mut by_number: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for entry in std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") {
            continue;
        }
        let Some(prefix) = name.split('-').next() else {
            continue;
        };
        let Ok(number) = prefix.parse::<u32>() else {
            continue; // non-numbered files (none today) are out of scope
        };
        by_number.entry(number).or_default().push(name);
    }
    assert!(
        !by_number.is_empty(),
        "no numbered spec files found under {} — wrong path?",
        dir.display()
    );
    let dupes: Vec<String> = by_number
        .iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(n, files)| format!("{n:02}: {}", files.join(", ")))
        .collect();
    assert!(
        dupes.is_empty(),
        "duplicate spec numbers in docs/specs/ — renumber the newcomer \
         (see docs/specs/00-index.md §Numbering changelog):\n{}",
        dupes.join("\n")
    );
}
