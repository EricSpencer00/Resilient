# Repeat and inspect the hosted operation

All compilation and test execution for this preparation runs on GitHub-hosted
Actions. The workflow is `.github/workflows/qualification-baseline.yml`; its
recorded source revision, complete command and tool versions are retained in the
evidence. The workflow uses Rust 1.99.1 and `cargo test --locked --features z3`
against seven named integration suites. Each suite invokes the Cargo-provided
`CARGO_BIN_EXE_rz`; the workflow retains that target-directory `debug/rz`, rather
than hashing an unrelated installation.

To create a new current-main baseline through the same hosted procedure:

```sh
gh workflow run qualification-baseline.yml --repo EricSpencer00/Resilient --ref main
gh run list --repo EricSpencer00/Resilient --workflow qualification-baseline.yml --limit 5
gh run watch RUN_ID --repo EricSpencer00/Resilient --exit-status
gh run download RUN_ID --repo EricSpencer00/Resilient --dir /controlled/archive/RUN_ID
```

Select the run ID from that dispatch and record its actual head SHA. A new run
on changed main is a new baseline. To rerun the exact recorded source instead,
use `gh run rerun RECORDED_RUN_ID --repo EricSpencer00/Resilient`; preserve the
attempt number, downloaded output and new tool identities separately. Runner
images and package repositories may change even when source does not. These
instructions do not assert bit-for-bit reproducible compilation.

For a downloaded artifact, from its directory:

```sh
sha256sum -c SHA256SUMS
python3 /path/to/recorded/source/agent-scripts/e2e-evidence.py verify tests
```

Compare `rz` to `binary-identity.json.compiler_sha256`, and confirm the source
revision/tree, Cargo.lock digest, `features`, profile, architecture, Rust/Cargo,
runner image, Z3 executable and libz3 hashes. Compare every requirement test name
against the raw test output and the evidence manifest; require all 33 named
outcomes, all seven suite summaries, command exit 0 and successful bundle
verification. Do not substitute an installed compiler or a later checkout.

Run the preparation inventory checker on `package.json` in a hosted validation
job or controlled assessment environment:

```sh
python3 scripts/check_qualification.py qualification/host-cli-2026-10/package.json
```

Its expected state is `gaps`, `certified: false`, because project-specific
classification and independent/assessment acceptance remain pending. A changed
status alone does not discharge those activities. Review evidence hashes and
technical adequacy separately. The retained tests do not supply structural
coverage, complete source-obligation coverage or target verification.
