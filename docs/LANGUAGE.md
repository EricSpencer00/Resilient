# Resilient Language Reference

## Feature Tier Classification

Resilient features are classified into three tiers that define stability guarantees, deprecation policies, and adoption recommendations for users and library developers.

### Stable

Features in the **Stable** tier are:
- Fully specified in the language reference
- Tested across all supported backends (interpreter, VM, JIT, embedded targets)
- Expected to remain compatible across minor versions
- Safe for users to depend on long-term in production systems
- Guaranteed to work on Cortex-M, RISC-V, and other supported embedded targets

**Graduation criteria:**
- Must have comprehensive test coverage (≥80%)
- Must work identically across all backends
- Must be documented in this reference
- Must have at least 2 minor releases with no breaking changes
- Must have no open design questions (RES-* issues resolved)

---

### Backend-Limited

Features in the **Backend-Limited** tier are:
- Specified and functional on certain backends only
- Documented with explicit backend compatibility tables
- May change in minor versions if backend support changes
- Safe to use if your deployment targets a backend where the feature is stable

**Graduation criteria:**
- Must be fully specified and consistent within the supported backend(s)
- Must have dedicated backend tests that pass
- Must be clearly documented with backend compatibility table
- Can graduate to Stable once all backends implement it

**Examples:**
- JIT-specific optimizations
- RISC-V-specific interrupt handling
- Cortex-M-specific memory-mapped register mappings

---

### Experimental

Features in the **Experimental** tier are:
- Under active design or implementation
- May change significantly between releases, including breaking changes
- Not recommended for production use
- Provided for early feedback and research

**Graduation criteria:**
- Must have an associated RES-* issue describing the design
- Should have at least one end-to-end example
- Requires an issue update every 2 releases explaining status
- Can graduate to Backend-Limited or Stable once design is finalized and implementation is complete

---

## Feature Stability Policy

### Deprecation

When a Stable feature must be removed or significantly changed:
1. Announce deprecation in release notes (minor version bump)
2. Provide clear migration path in compiler diagnostics
3. Support deprecated feature for at least 3 minor versions
4. Remove in next major version

Experimental and Backend-Limited features may be removed or significantly changed without deprecation notice, though RES-* issues should be updated to explain the change.

### Breaking Changes

The following constitute breaking changes to Stable features and require a major version bump:
- Removing a language construct or keyword
- Changing the type or behavior of a built-in function
- Changing error codes or diagnostic messages in a non-backward-compatible way
- Removing support for a previously-stable backend target

The following are NOT breaking changes (minor version bump is sufficient):
- Adding new features
- Making the type system stricter (previously accepted code may now error)
- Adding new compiler warnings
- Improving diagnostic messages

---

## Current Feature Classification

This table will be populated in follow-up PRs with all Resilient language features and their tier classifications.

| Feature | Tier | Backends | Notes |
|---------|------|----------|-------|
| `@require_contracts` module directive | Experimental | Typechecker | Enrols every function in the file into non-vacuous-contract and loop-bound verification; `(strict)` additionally mandates contract presence (RES-3854). |
| `@ai_generated` function attribute | Experimental | Typechecker | Pure provenance alias of `#[generated]` (RES-3858); records audit metadata, grants no verification behaviour. |

---

## Tier Graduation Workflows

### Experimental → Backend-Limited

When an Experimental feature has a finalized design and partial backend coverage:
1. Move from `experimental` label to `backend-limited` on related issues
2. Add feature to "Backend-Limited Features" section below with compatibility table
3. Add comprehensive tests for each supported backend
4. Document any limitations or differences between backend implementations

### Backend-Limited → Stable

When a Backend-Limited feature is implemented on all backends consistently:
1. Add to "Stable Features" section with full specification
2. Ensure test coverage across all backends (≥80%)
3. Run regression test suite across all backends
4. Remove from Backend-Limited section
5. Update this reference document

### Removing Features

When a Stable feature must be removed:
1. Document in the issue and release notes
2. Add compiler warning (not error) in the version announcing deprecation
3. Allow 3+ minor releases for users to migrate
4. Change compiler warning to hard error in next major release
5. Remove implementation code in subsequent release

---

## User Guidance

**Building Safety-Critical Systems:**
Use only features from the **Stable** tier. These features have the strongest compatibility guarantees and will be maintained for long-term production use.

**Building Research & Experimental Projects:**
You can use Backend-Limited and Experimental features if you understand their limitations. Check the compatibility tables and design documents (RES-* issues) before using these features.

**Library Developers:**
Document which feature tiers your library uses. If you use Experimental or Backend-Limited features, clearly state the tier and compatibility constraints in your README.

**Adopters:**
When evaluating Resilient for a project, check that your required features are in the Stable tier before committing.

---

## References

- **RES-3501**: Stabilize the language reference and feature-tier policy
- **RES-3502**: Design a real module and package system
- **RES-3503**: Unify the long-term type system roadmap
- **RES-3504**: Specify and enforce the memory model
- **RES-3505**: Consolidate the failure and recovery semantics
- **RES-3506**: Define the backend architecture contract
