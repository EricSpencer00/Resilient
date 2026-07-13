# VS Code Extension Release Runbook

`vscode-extension/` ships as `fromamerica.resilient-vscode` on the VS Code
Marketplace. This page documents the automated build → package → publish
pipeline, the current version divergence between the repo and the
Marketplace, and how to reconcile it when the maintainer is ready.

## The three-job pipeline

`.github/workflows/vscode_extension.yml` defines three jobs that gate each
other in sequence:

| Job | Trigger | What it does |
|---|---|---|
| `build` | every push to `main`, every PR touching `vscode-extension/**` | `npm install` + `npm run compile` (TypeScript type-check). Catches breakage early; produces no artifact. |
| `package` | push of a `v*` tag, after `build` passes | Runs `npx --yes @vscode/vsce package --out resilient-vscode.vsix` and uploads the `.vsix` as a GitHub Actions artifact (30-day retention). |
| `publish` | push of a `v*` tag, after `package` passes | Runs `npx --yes @vscode/vsce publish --pat "$VSCE_PAT"`, authenticated with the `VSCE_PAT` repository secret, to push the extension live to the Marketplace. |

Pushing a tag matching `v*` (e.g. `v0.2.3`) is what triggers a real release:
it runs `build` → `package` → `publish` end to end, uploading the `.vsix`
and publishing it to the Marketplace in the same run. Plain pushes to `main`
and PRs only run `build` — no packaging, no publish, no Marketplace side
effects.

`VSCE_PAT` is an Azure DevOps personal access token scoped to
`Marketplace: Manage` for the `fromamerica` publisher. It is stored as a
GitHub Actions repository secret and is never printed or persisted anywhere
in the workflow. A separate scheduled workflow,
`.github/workflows/vsce-token-check.yml`, probes the token weekly
(`vsce verify-pat`) and files a tracking issue if it is missing, invalid, or
approaching its expiry window (`VSCE_PAT_EXPIRY` repo variable) — the goal
is to catch a dead token before a real release needs it, not on a failed
publish.

## Version-coherence invariant

`vscode-extension/package.json` (`version`, `publisher`, `name`) and
`resilient/Cargo.toml` (`version`) are asserted to stay in sync by
`resilient/tests/it/vscode_release_sync_smoke.rs`, run as part of the normal
compiler test suite (`cargo test --manifest-path resilient/Cargo.toml`).
That test currently enforces:

- `publisher == "fromamerica"`
- `name == "resilient-vscode"`
- `version` is present, non-empty, and parses as `major.minor.patch` semver
- `version` in `vscode-extension/package.json` equals `version` in
  `resilient/Cargo.toml` (both are `0.2.3` as of this writing)

If a future change decouples the extension version from the compiler
version (see "Reconciliation options" below), that equality assertion is
the one line in the test that needs to change — do it deliberately, in the
same PR that makes the decoupling decision, not as a side effect of an
unrelated version bump.

## Marketplace divergence: 1.5.3 vs the 0.2.x line

The live published extension `fromamerica.resilient-vscode` on the
Marketplace is currently at **1.5.3** — a line published above, and
unrelated to, the repo's `0.2.x` compiler-aligned version line. This means:

- The repo's `package.json` at `0.2.3` does **not** match what a user
  installing from the Marketplace today receives (`1.5.3`).
- The VS Code Marketplace enforces **monotonically increasing** version
  numbers per extension. Publishing `0.2.3` while `1.5.3` is live would be
  **rejected outright** — `vsce publish` cannot push a lower version over a
  higher one.
- The only way to make a `0.2.x` version "latest" again is
  `vsce unpublish` on the `1.x` versions, which is destructive: it wipes
  Marketplace version history and install-count continuity for those
  versions. That is a decision only the maintainer can make, and it is not
  automated by this workflow or any script in this repo.

**This repo intentionally does not roll `package.json`'s version backward**
to try to "fix" this divergence — a lower number does not resolve a
Marketplace ordering conflict, it just makes the mismatch worse (the CI
`publish` job would then fail outright on the next tag push, since
`0.2.3 < 1.5.3`).

### Reconciliation options

Pick one, deliberately, when the maintainer is ready to make the Marketplace
line match the repo again:

1. **Unpublish the accidental `1.x` line.** Maintainer runs
   `vsce unpublish fromamerica.resilient-vscode` (or unpublishes only the
   offending versions, if the CLI/portal supports partial removal) from
   their own machine with their own PAT. Irreversible: version history and
   download stats for the removed versions are gone. After that, a
   `0.2.x`-tagged release publishes cleanly again and the compiler-version
   mirroring in the smoke test continues to hold.
2. **Publish forward from `1.5.3` and decouple the extension version from
   the compiler version.** Bump `vscode-extension/package.json` to
   `1.5.4` (or higher) independent of `resilient/Cargo.toml`, and relax the
   `vscode_release_sync_smoke.rs` equality assertion to no longer require
   `package.json` version == `Cargo.toml` version (keep the publisher/name/
   semver-shape checks). This is non-destructive but means the two version
   numbers no longer tell you anything about each other — the extension's
   CHANGELOG becomes the source of truth for "what compiler version does
   this extension version target."

Neither option is executed by any code in this repo. This document exists
so the tradeoff is visible before the next real `v*` tag push, not
discovered as a failed `publish` job.

## Manual fallback

If the tag-triggered workflow is unavailable (e.g. debugging a `VSCE_PAT`
issue outside CI), the same steps can be run locally from
`vscode-extension/`:

```bash
npm install
npm run compile
npx @vscode/vsce package --out resilient-vscode.vsix   # produces the .vsix
npx @vscode/vsce publish --pat <token>                  # publishes it
```

Never paste a real PAT into a shell history file or commit it anywhere —
treat it the same as any other credential per `CLAUDE.md`'s secrets rules.
