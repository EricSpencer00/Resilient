# CICD Quality Gates

This document describes the comprehensive quality gates applied to all code in the Resilient repository.

## Overview

The Resilient compiler targets safety-critical embedded systems. Our CICD pipeline enforces multiple layers of quality gates to ensure code reliability:

1. **Main CI** (`.github/workflows/ci.yml`) - Core compilation and testing
2. **AI Threat Model** (`.github/workflows/ai_threats.yml`) - LLM failure mode detection
3. **Strict Quality Gates** (`.github/workflows/strict_quality_gates.yml`) - Enhanced verification

## Main CICD Pipeline (`ci.yml`)

### Required Checks (Merge Gate)

All of these must pass for a PR to merge:

| Check | Purpose | Blocks Merge |
|-------|---------|------------|
| `default_ci` (build/test/clippy) | Compilation, tests, linting | ✓ Yes |
| `z3_ci` (Z3 verification) | SMT solver verification | ✓ Yes |
| `extra_feature_tests` (lsp, jit) | Optional feature validation | ✓ Yes |
| `ai_threat_tests` | AI threat model validation | ✓ Yes |
| `board` (commit hygiene) | Commit message format | ✓ Yes |
| Embedded cross-compile tests | Cortex-M, RISC-V, Thumb | ✓ Yes |
| Size gate (≤64 KiB .text) | Embedded code budget | ✓ Yes |
| Performance gate | Regression detection | ✓ Yes |

### Optimization: Parallel Execution

**Format + Clippy**: Now run in parallel instead of sequentially:

```yaml
# Before: 2 sequential steps (~30s total)
cargo fmt --check
cargo clippy --all-targets -- -D warnings

# After: 2 parallel jobs (~25s total)
cargo fmt --check & cargo clippy --all-targets -- -D warnings &
```

**Impact**: ~5-10s faster CI runs per PR.

## Strict Quality Gates Workflow (`strict_quality_gates.yml`)

New workflow that runs in parallel with main CI. These checks are **informational but not merge-blocking** (advisory layer):

### 1. Documentation Audit

```bash
cargo doc --no-deps
cargo test --doc
```

**Validates**:
- Public API documentation completeness
- Doc example compilation
- Documentation link integrity

**Threshold**: Informational (tracks but doesn't block)

### 2. Strict Linting

```bash
cargo clippy --all-targets -- -D warnings         # Already do this
cargo clippy --all-targets -- -W clippy::pedantic # New (advisory)
```

**Validates**:
- All clippy warnings denied (existing)
- Pedantic lints reported (new)
- Unsafe blocks have `// SAFETY:` comments

**Threshold**: Warnings reported but don't block merge

### 3. Dependency Audit

```bash
cargo audit --locked
```

**Validates**:
- No known security vulnerabilities
- Dependency freshness
- Lock file consistency

**Threshold**: Advisory (recommend fixes before release)

### 4. Dead Code Detection

```bash
cargo clippy --all-targets -- -W dead_code
```

**Validates**:
- Unused functions identified
- Unused imports flagged
- Dead code quantified

**Threshold**: Tracked but doesn't block (future: enforce threshold)

### 5. Binary Size Tracking

```bash
cargo build --release
# Measure rz binary size
```

**Validates**:
- Release binary size trending
- Regression detection (alert if >30 MB)
- Size reports stored as artifacts

**Baselines**:
- Current: ~20 MB (debug build)
- Alert threshold: 30 MB

**Threshold**: Alert on > 30 MB, trend tracked

### 6. Test Quality

```bash
cargo test --locked --verbose
```

**Validates**:
- Test count & pass rate
- Test naming conventions
- Coverage distribution

**Threshold**: Informational (baseline 100+ tests passing)

### 7. Security Scanning

```bash
# Check for:
# - Hardcoded secrets/credentials
# - Panics in core library code
# - Unsafe blocks without justification
```

**Validates**:
- No hardcoded secrets
- Library code panic-free (except tests)
- Unsafe justified with comments

**Threshold**: Alerting (tracked for violations)

## Workflow Hierarchy

```
┌─────────────────────────────────────────────────┐
│ Pull Request Created                            │
└────────────────┬────────────────────────────────┘
                 │
        ┌────────┴────────┐
        │                 │
        ▼                 ▼
    ┌─────────┐     ┌──────────────┐
    │ Main CI │     │ AI Threats   │
    │ (Fast)  │     │ (Parallel)   │
    └────┬────┘     └──────┬───────┘
         │                 │
    ┌────▼────┐     ┌──────▼───────┐
    │ ✓Build  │     │ ✓Threats     │
    │ ✓Test   │     │ ✓Demo File   │
    │ ✓Clippy │     └──────────────┘
    │ ✓Fmt    │
    │ ✓Z3     │           ▼
    └────┬────┘     ┌──────────────────┐
         │          │ Strict Quality   │
         │          │ (Advisory)       │
         │          │                  │
         │          │ ✓Docs            │
         │          │ ✓Linting (pedantic)
         │          │ ✓Audit           │
         │          │ ✓Dead Code       │
         │          │ ✓Binary Size     │
         │          │ ✓Test Quality    │
         │          │ ✓Security Scan   │
         │          └──────┬───────────┘
         │                 │
         └────────┬────────┘
                  │
           ┌──────▼──────┐
           │ All Checks  │
           │ Summary     │
           └──────┬──────┘
                  │
                  ▼
         ┌────────────────┐
         │ ✓ Ready Merge? │
         │ (All required  │
         │  checks pass)  │
         └────────────────┘
```

## Quality Gate Strictness Levels

| Level | Purpose | Blocks Merge | Example |
|-------|---------|------------|---------|
| **Critical** | Safety-critical | YES | Compilation errors, panics in lib code |
| **Required** | Functional correctness | YES | Test failures, type errors, clippy warnings |
| **Strong** | Code quality | YES | AI threats, documentation gaps |
| **Advisory** | Best practice | NO | Pedantic lints, dead code suggestions |
| **Tracked** | Observability | NO | Binary size, dependency audit recommendations |

## Performance Impact

### Parallel Optimization (fmt + clippy)

```
Before (sequential):
  fmt: 3s
  clippy: 25s
  Total: 28s

After (parallel):
  max(fmt, clippy) = 25s
  Saved: ~3s per PR
```

### Overall CI Times

- **Fast path** (docs-only PR): <10s
- **Regular PR** (with AI threats): ~45-60s total (main CI ~35s + AI threats ~20s in parallel)
- **Full verification** (with strict gates): ~75s (gates run in parallel, advisory only)

## Future Enhancements

### Phase 2: Coverage Tracking

```bash
cargo tarpaulin --out Html --output-dir coverage
# Enforce: lib ≥ 75%, tests ≥ 60%
```

### Phase 3: Incremental Strictness

- Enforce documentation on new APIs only
- Gradually increase coverage threshold
- Selective pedantic lint enforcement

### Phase 4: Custom Lints

- Safety-critical pattern detection
- Embedded-specific checks
- No_std compliance verification

## Debugging CI Failures

### Local Reproduction

All CI checks can be run locally:

```bash
# Main CI checks
cargo build --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check

# AI threat checks
cargo test --test ai_threats_smoke --locked

# Strict quality gates
cargo doc --no-deps
cargo audit --locked
cargo build --release
```

### Artifact Inspection

Failed runs produce artifacts (7-day retention):

- `cargo-timings-NNN.html` - Build profile
- `ai-threat-scan-NNN` - Threat detection reports
- `binary-size-NNN` - Release binary
- `test-results-NNN` - Test output logs

## Adding New Gates

To add a new quality gate:

1. **Define in workflow** - Add step to appropriate `.yml`
2. **Set threshold** - Critical (block) vs Advisory (track)
3. **Generate reports** - Use `actions/upload-artifact@v4`
4. **Document** - Add to this file
5. **Test locally** - Verify before PR
6. **Gradual rollout** - Start advisory, promote to required

## See Also

- `.github/workflows/ci.yml` - Main pipeline
- `.github/workflows/ai_threats.yml` - AI threat model validation
- `.github/workflows/strict_quality_gates.yml` - Quality gate layer
- `CLAUDE.md` - Agent guidelines
- `CONTRIBUTING.md` - Contribution guidelines
