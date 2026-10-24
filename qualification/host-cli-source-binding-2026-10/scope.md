# Supported tool use and acceptance boundary

This dossier prepares technical evidence for a developer using Resilient's host
CLI to emit saved-query certificate bundles including modeled contract proofs
with `--emit-certificate` and `--sign-cert`, and consume them through
`verify-cert` and `verify-all` (including `--require-manifest-signature` and
`--z3`). Source-content identity claims require
`verify-all certs/ --source-root snapshot/ --pubkey public.pem --z3`; that flag
implies manifest-signature verification. The separate `--emit-contract-certificate` report/schema is excluded. The configuration is Linux x86_64, debug profile, Cargo locked,
`z3` feature, and the exact executable, dependency lock and solver identities
recorded in `binary-identity.json`. The evidence does not qualify a later rebuild.
The author of this technical preparation is Codex acting for Eric Spencer.

The nine suites cover straight, constant and branching return certificates;
loop certificate round trips and conservative iteration-binding guards; exact
signed manifest enumeration; unavailable solvers and empty replay sets; strict
solver responses; replay time/output limits; and signed source-content/frozen import-graph
verification, including relocated identical snapshots. Operational requirements and
all included test names are in `requirements.json`. Their pass observations are
bound to the retained source and executable by `tests/manifest.json` and the
hosted workflow. These are selected tool-use requirements, not a complete language
or safety-product requirements inventory.

The manifest's `program` field remains a path/label. Strict manifest signatures
alone protect that label and obligation enumeration. With the required
`--source-root` operation, the signed `source_binding` graph additionally binds
the exact parsed entry/import buffers (SHA-256 and byte length), frozen import
edges and builtin declarations. Verification reparses the supplied stable snapshot
and checks complete file/edge/builtin reachability before replay. A relocated
snapshot must preserve its paths relative to the common canonical source root.
Legacy verification without `--source-root` has no source-content identity claim.

Source binding does not bind current package manifests, lockfiles or registry
configuration, or rerun package resolution. It checks the producer's frozen
translation unit and never invokes Git, downloads dependencies or materializes
caches. Builtins are implemented by the separately identified compiler. The
consuming project must separately retain accepted requirements and link them to
this source/binary; content identity does not establish requirement correctness
or source-to-query translation correctness.

Use a controlled, stable filesystem snapshot. Canonical containment and bounded
regular-file checks precede reads, but path checks/opens are not atomic against
adversarial concurrent mutation. Version 1 limits are 256 inputs, 1,024 import/builtin
edges, 10 MiB per source, 64 MiB total source bytes and 64 import levels. The graph
suite observes bounded cycle/excessive-inventory rejection; it is not exhaustive
validation of every resource boundary or every filesystem race.

## Limits and excluded uses

- SMT replay establishes the emitted formula's solver result. It does not
  independently validate source-to-query translation or compiler lowering.
- Existing contract body models use mathematical integers. No machine-width
  overflow, integer representation or target arithmetic equivalence is claimed.
- Finite-loop semantic completeness, mutation/alias behavior beyond the concrete
  guarded examples, and general program-level obligation completeness are
  excluded while issue #4904 is resolved. Passing the loop regressions does not
  discharge this issue.
- `clause-only` proofs are not implementation-satisfaction evidence. Unknown,
  unsupported and timed-out obligations remain unresolved.
- Embedded hardware, release/optimized builds, other architectures, JIT/LLVM/WASM
  backends, runtime deployment, concurrency, timing, WCET and generated machine
  code correctness are excluded from this compiler executable record.
- Earlier runtime evidence is supplemental historical evidence at its recorded
  revision; it is not proof that the compiler-baseline runtime passed those runs.
- No ASIL, TCL, airborne software level, TQL, medical software class or SIL is
  assigned by this dossier. No regulator, assessor, applicant or manufacturer
  acceptance is represented.

## Entry and revalidation conditions

Use only the identified executable/solver configuration, inspect proof basis and
all unresolved obligations, retain exact source and requirement hashes, and apply
independent output verification appropriate to the consuming project. Changes to
source, executable, dependency locks, solver/library, build profile, environment,
backend or intended use require impact analysis and another hosted baseline run.
New supported semantics require positive and negative requirements-based tests.
Independent technical review, licensed-standard tailoring and project/assessment
acceptance remain explicit pending activities in `package.json`.
