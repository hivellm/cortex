"""One-shot port migration: rename 1500x/1501x/1502x → 1700x/1701x/1702x.

Drives a literal port-number substitution across .rs / .toml / .json /
.ts / .tsx / .md / .yaml / .yml / .sh / .ps1 / .env files. Skips
target/, node_modules/, .git/, lockfiles. Idempotent.
"""

import os
import re
import pathlib
import sys

EXTS = {".rs", ".toml", ".json", ".ts", ".tsx", ".md", ".yaml", ".yml", ".sh", ".ps1"}
EXTRA_FILES = {".env", ".env.example", ".mcp.json"}

MAPPING = [
    ("15011", "17000"),
    ("15010", "17010"),
    ("15020", "17020"),
    ("15004", "17004"),
    ("15003", "17003"),
    ("15002", "17002"),
    ("15001", "17001"),
    ("15000", "17000"),
]

SKIPS = ["target/", "node_modules/", "/dist/", ".git/", "/.cortex/",
         "pnpm-lock.yaml", "Cargo.lock", "scripts/port-migration.py"]


def main(root="."):
    repo = pathlib.Path(root)
    modified = []
    for path in repo.rglob("*"):
        if not path.is_file():
            continue
        s = str(path).replace(os.sep, "/")
        if any(skip in s for skip in SKIPS):
            continue
        if path.suffix not in EXTS and path.name not in EXTRA_FILES:
            continue
        try:
            txt = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        new = txt
        for old, new_port in MAPPING:
            pat = re.compile(rf"(?<!\d)({old})(?!\d)")
            new = pat.sub(new_port, new)
        if new != txt:
            path.write_text(new, encoding="utf-8")
            modified.append(s)
    print(f"Modified {len(modified)} files")
    for m in modified[:50]:
        print(f"  {m}")
    if len(modified) > 50:
        print(f"  ... and {len(modified) - 50} more")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else ".")
