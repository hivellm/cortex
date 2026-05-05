# TmlTextmate — Overview

## Purpose

TmlTextmate provides a TextMate grammar definition (`tml.tmLanguage.json`) for TML — a multi-paradigm systems programming language targeting LLVM IR.

The grammar enables syntax highlighting and language detection for `.tml` files across any editor supporting TextMate grammars:
- VS Code
- Sublime Text
- Atom
- Nova
- GitHub Linguist (for `.tml` language recognition)

## Role in HiveLLM

TmlTextmate is a **submodule dependency** integrated into GitHub Linguist for language detection. It provides:

1. **Syntax highlighting** — colorizes TML code across IDEs
2. **Language detection** — identifies `.tml` files as TML code
3. **Grammar reference** — documents TML lexicon (keywords, types, operators, literals)

The grammar **does not compile or execute TML** — the TML compiler (self-hosting, ~150k lines of stdlib) is a separate project.

## Project Status

- **License**: Apache-2.0
- **Grammar format**: TextMate Language (JSON schema)
- **File scope**: `source.tml`
- **Integration**: GitHub Linguist submodule
- **Samples**: 6 reference `.tml` files demonstrating language features
