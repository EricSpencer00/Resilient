# Controlled requirements to checked implementation

This tool generates a finite controller from a deliberately small English grammar,
checks its temporal model with TLC, and executes every state/event pair using
Resilient's interpreter, match VM and direct VM. It retains requirements, generated
artifacts, tool identities, commands, raw output, requirement-level observations,
and an evidence checksum inventory.

```sh
python3 qualification/requirements-pipeline/pipeline.py generate \
  qualification/requirements-pipeline/controller.requirements --output /tmp/controller-bundle
python3 qualification/requirements-pipeline/pipeline.py verify /tmp/controller-bundle \
  --rz /absolute/path/to/rz --jar /absolute/path/to/tla2tools.jar \
  --output /tmp/controller-evidence
```

Use fresh output directories. `check-runtime` performs only the executable checks
and records `model_check: not-run`; it cannot substitute for `verify`.
`--timeout-seconds` bounds each checking process to at most 60 seconds.
`--implementation candidate.rz` checks another implementation of `step(int, int)`
against the same table and records the exact candidate. It does not change or
re-sign the generated bundle.

## Accepted semantics

The complete example is [controller.requirements](controller.requirements).
Names contain lowercase ASCII letters, digits and underscores, starting with a
letter; each name has at most 32 characters. Requirement IDs start with an
uppercase ASCII letter and otherwise contain uppercase letters, digits or
underscores. Lists use comma-space separators. Declarations and sentences are
case-sensitive. Blank lines and outer whitespace are ignored; other prose is
rejected, with its line number.

| Sentence | Meaning |
| --- | --- |
| `States: off, ready, tripped.` | Ordered state domain; integer encoding starts at zero. |
| `Events: arm, fault, reset.` | Ordered event domain with the same encoding. |
| `Initial: off.` | Initial state. |
| `R01: When state is off and event is arm, next state shall be ready.` | One total transition table cell. |
| `S01: After event fault, state shall be tripped.` | Invariant on every transition with that event. |
| `S02: From state tripped, event arm shall lead to state tripped.` | Invariant on that source/event pair. |
| `L01: Assuming fair event fault, state tripped shall eventually be reached.` | Eventually reach the target, including the initial instant, under weak fairness of the event action. |

The profile permits 1–16 states and 1–16 events, a source of at most 64 KiB,
one initial state, unique requirement IDs, exactly one transition per pair,
and at least one separate property. Every declared state must be reachable.
All liveness requirements must name the same fair event: combining different
assumptions would otherwise silently strengthen the premise of each sentence.
No fairness is assumed when there is no liveness requirement. Weak fairness
is a model assumption, not a verified property of a real scheduler.

TLA+ records the previous state and last event so its dumped reachable states
expose the transition relation. Verification requires successful TLC completion,
all safety/liveness properties, and exact agreement of that observed relation
with the declared total table. It then compares actual compiler output for every
pair with the table. Inputs outside the declared domain return `-1` from generated
code; they are outside this profile's verified transition relation.

## Reproduction and negative controls

```sh
REQUIREMENTS_RECORDS=/tmp/controller-test-records \
  RZ_BIN=/absolute/path/to/rz TLC_JAR=/absolute/path/to/tla2tools.jar \
  python3 qualification/requirements-pipeline/tests/test_cli.py
cargo test --manifest-path resilient/Cargo.toml --test requirements_pipeline
```

`REQUIREMENTS_RECORDS` retains each test workspace and invocation result in a
fresh directory, including negative controls. Omit it for temporary test files.
The Cargo integration test supplies the actual built compiler. TLC tests require
`TLC_JAR` and are explicitly skipped when it is absent. The Python suite includes
an independent nine-cell oracle, all nine single-cell implementation mutations,
contradictory safety and liveness properties checked by real TLC, ambiguous and
partial input, unreachable states, rehashed artifact tampering, missing compiler,
timeout and stale evidence rejection. Review skipped tests before using a run as
formal evidence. TLC 1.7.4 is the initially exercised version; the evidence records
the supplied jar hash and Java version rather than trusting a version label.

See the [assurance profile](assurance.md) for operational requirements, the
verification trace, failure analysis and standards preparation boundaries.

## Assurance boundary

Generation is deterministic for identical input bytes and generator bytes.
Evidence logs include tool-dependent timing and absolute paths and are not promised
to be byte-identical. Bundle validation re-generates expected artifacts; changing a
file and its manifest hash does not make it acceptable. Checksums establish content
identity, not authenticity or independent review.

This is finite-instance model checking and executable conformance, not a proof of
the compiler, arbitrary candidate programs, or the general generator. The generated
`step` is a pure finite table; arbitrary candidates may contain state or effects not
covered by these observations. There is no claim about native code, hardware,
timing, interrupt interleavings, device drivers, sensor validity or deployment.
The trusted base includes this checker, Python, TLC, the host, and the observed
Resilient binary. Mutation detection tests the checking path but does not remove
that trusted base.

Unrestricted NLP cannot be treated as an unambiguous formal specification. An LLM
may propose these sentences, but a competent reviewer must approve that they
capture the intended behavior and hazards. NASA's
[FRET](https://software.nasa.gov/software/ARC-18066-1) provides a broader example of
structured requirements and formal semantics. This profile is not FRETISH-compatible.

Every report has `certified: false`. A successful run is one evidence item for a
specified tool-use/deployment profile. It does not establish ISO 26262, IEC 61508,
IEC 62304, DO-178C/DO-330 compliance or a safety classification. Use the repository's
[qualification readiness process](../../docs/qualification-readiness.md) to track
requirements review, tool qualification, configuration control, independent review,
remaining defects and acceptance by the applicable authority.

## Separate translation checker

Before accepting generated artifacts, `independent_checker.py` independently
reads the original sentences using a token grammar, compares the complete IR,
checks the restricted pure generated function across the declared domain, and
checks the emitted TLA+ transitions, property formulas, fairness and configuration
against independently derived expectations. It does not import the generator.
The manifest binds both generator and checker hashes; execution reports identify
this check as covering the generated artifacts, not arbitrary candidate code.

`tests/test_independent.py` injects consistent wrong generator translations
(transition, initial state, encoding, safety property and fairness), mutates the
emitted artifacts, and exercises domains up to 16 states by 16 events. This reduces
shared-implementation failure exposure. Both implementations still embody the
same documented semantics and can share conceptual mistakes: this is neither a
machine-checked universal refinement theorem nor an independent safety assessment.
