# TmlTextmate — Open Questions

## Questions for TML Language Team

1. **Grammar completeness**: Are all TML 1.0 keywords, operators, and types covered?
   - Last update date of grammar?
   - Any planned TML 2.0 syntax changes affecting grammar?

2. **Decorator ecosystem**: Is the current decorator set final?
   - Will new decorators be added (e.g., @inline, @deprecated)?
   - Should grammar be regenerated from a source-of-truth decorator list?

3. **Generic syntax edge cases**: How should grammar handle:
   - Deeply nested generics: `Vec[HashMap[Str, Vec[I32]]]`?
   - Where clauses with multiple bounds: `where T: Ord + Copy + Default`?
   - Higher-ranked trait bounds (if supported): `for<'a> ...`?

4. **Template literal nesting**: Can template expressions contain nested templates?
   - Example: `` `outer {`inner {x}`}` ``
   - Current grammar uses `include: $self` (should work, but untested)

## Questions for GitHub Linguist Maintainers

5. **Submodule version pinning**: How frequently does Linguist pull TmlTextmate updates?
   - Manual trigger or automatic nightly?
   - How to force immediate update if critical bug found?

6. **Grammar fallback**: If grammar is unavailable, does Linguist degrade gracefully?
   - Does `.tml` default to generic text tokenization?
   - Risk of lost highlighting on github.com?

## Questions for HiveLLM Ecosystem

7. **Cortex integration**: Should Cortex dynamically regenerate grammar from:
   - TML compiler's keyword/operator definitions?
   - VSCode extension configuration files?
   - Or trust TmlTextmate as canonical?

8. **Multi-language support**: Are there plans for:
   - LSP (Language Server Protocol) implementation for TML?
   - IntelliJ IDE plugin (requires custom grammar format)?
   - If yes, how to keep TextMate grammar in sync?

9. **Grammar validation in CI**: Should HiveLLM CI pipeline:
   - Validate grammar JSON syntax?
   - Test grammar against sample files (expected token chains)?
   - Compare grammar keywords against TML compiler keyword list?

## Potential Issues

### Issue 1: Grammar Divergence

**Problem**: TML language evolves, but grammar doesn't update immediately.

**Impact**: Syntax highlighting lags behind new TML features.

**Mitigation**:
- Establish SLA for grammar updates (e.g., within 1 week of TML release)
- Automated tests comparing TML compiler keywords to grammar
- Linguist submodule pinned to specific commit (audit trail)

### Issue 2: Editor-Specific Quirks

**Problem**: TextMate regex flavor differs slightly from JavaScript/Python/PCRE.

**Impact**: Patterns work in VSCode but not in Sublime Text or Atom.

**Mitigation**:
- Test grammar in multiple editors (VSCode, Sublime, Atom)
- Document any regex deviations in comments
- Use only lowest-common-denominator regex features (no lookahead/lookbehind)

### Issue 3: Scope Name Collisions

**Problem**: Custom theme rules might target generic scope names (e.g., `keyword.*`) unintentionally affecting TML.

**Impact**: Unexpected color changes when user installs third-party theme.

**Mitigation**:
- Always use `*.tml` suffix for specificity
- Document standard scope names in README
- Advise users against overly broad theme rules

### Issue 4: Sample File Staleness

**Problem**: Sample files may not reflect current TML language features.

**Impact**: Misleading documentation; developers copying old syntax patterns.

**Mitigation**:
- Update samples alongside grammar changes
- Add comments with TML version each sample targets
- Periodically audit samples against TML changelog

### Issue 5: No Type Checking in Grammar

**Problem**: Grammar does not validate type correctness (e.g., assigning `I32` to `Str` variable).

**Impact**: Misleading highlighting suggests valid code that won't compile.

**Mitigation**:
- Document that grammar is for **lexical highlighting only**
- Recommend TML LSP for semantic validation
- Add caveat in README: "Grammar highlights syntax, not semantics"

## Gaps to Address

1. **Documentation**: README should explain TextMate scope names for theme customization
2. **Validation**: Add JSON schema validation to CI pipeline
3. **Testing**: Create comprehensive test suite (patterns vs. expected tokens)
4. **Changelog**: Maintain CHANGELOG.md tracking grammar updates
5. **Performance**: Profile regex performance for large .tml files (1000+ lines)
6. **Accessibility**: Ensure sufficient color contrast in suggested theme colors
