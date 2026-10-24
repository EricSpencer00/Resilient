# Authenticated manifest verification

The scoped tool use is `verify-all --require-manifest-signature --z3` with an
approved public key. It authenticates the exact emitted manifest before replay.
This supplements per-file signatures; it does not establish source-to-query
completeness or external qualification approval under any safety standard.

The public CLI suite `manifest_authentication` covers complete signed manifests,
removed and reordered obligations, changed program identity, missing signatures,
wrong keys, compiler signing and legacy compatibility. The production baseline is
`1ba035737f057197a40dbd10400fdac051210e22`, with only the candidate's regression
test file overlaid. Evidence records that test-only working-tree change and
its input hash; the compiler source remains the baseline revision.
The first development run recorded seven failures and one passing legacy
control: [baseline execution](https://github.com/EricSpencer00/Resilient/actions/runs/40048323027/job/120735070127).
The generated-bundle fixture was then corrected to use an already-supported
loop proof, retaining the signature and real-solver assertions. The paired
workflow reruns that corrected input against both production revisions.

The failures establish that the proposed strict mode and emitted manifest
signature were absent; they do not claim legacy mode promised those checks.

The Manifest qualification workflow runs each unchanged case separately on the
baseline and candidate, retains checksummed execution records, compares the
actual solver identity, and verifies seven red-green attestations. The workflow
pins Rust 1.99.1 and records the executed toolchain. All builds and test execution
use GitHub-hosted runners. Artifacts are retained for 90 days and should be
archived with the applicable release dossier before expiry.
