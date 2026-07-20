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

Pushing a tag matching `v*` (e.g. `v1.0.0-rc.1`) is what triggers a real release:
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
That test enforces:

- `publisher == "fromamerica"`
- `name == "resilient-vscode"`
- `version` is present, non-empty, and parses as `major.minor.patch` semver
- the `version` core in `vscode-extension/package.json` is **>=** the
  `version` core in `resilient/Cargo.toml` — the extension line may *lead*
  the compiler (see the Marketplace reconciliation below) but must never
  fall behind it

As of the `v1.1.0` release the extension is at **`1.8.0`** and the compiler at
**`1.1.0`**: the version numbers are still decoupled (see the convergence plan
below), so the extension's own CHANGELOG — not the number — is the source of
truth for which compiler version a given extension release targets.

## Version convergence plan: unify at `v2.0.0`

**Decision (maintainer, 2026-07-19):** stop bumping the extension's minor
version on compiler-only releases, and **converge both version lines at
`v2.0.0`** — from `v2.0.0` onward the extension and the compiler share one
version number and bump in lockstep.

Rationale: the `>= compiler` decoupling (RES-4102) was a one-time fix to
publish *past* the stale Marketplace `1.5.3` line. It has since caused the
extension to drift ahead on every release (`1.6.0 → 1.7.0 → 1.8.0`) for no
functional reason, widening the gap it was meant to close. Freezing the
extension line lets the compiler's own version catch up; at the `2.0.0`
milestone both are set to `2.0.0` together and stay aligned thereafter.

Concretely, through the rest of the `1.x` line:

- **Do not** bump `vscode-extension/package.json` `version` for a compiler
  release. Leave it at `1.8.0`. A `v*` tag still republishes the extension,
  but the idempotent publish step (RES-4102) skips it when `1.8.0` is already
  live, so compiler-only releases are a no-op for the Marketplace.
- The `vscode_release_sync_smoke.rs` `extension core >= compiler core` guard
  keeps holding automatically while the compiler stays below `1.8.0`. **If a
  `1.x` compiler release would reach or exceed `1.8.0`, do not bump the
  extension to stay ahead — instead cut `v2.0.0` and set both to `2.0.0`**,
  which is the convergence point this plan is steering toward.
- At `v2.0.0`: bump `vscode-extension/package.json` to `2.0.0` in the same
  release PR as the compiler, and from then on treat the two versions as one.

## Marketplace divergence: reconciled by publishing forward (RES-4102)

The live published extension `fromamerica.resilient-vscode` reached **1.5.3**
under an old versioning scheme (the `v1.5.x` git tags were VSCE-package
version relics, not real compiler releases — see `RELEASE_AUDIT.md`). The VS
Code Marketplace enforces **monotonically increasing** versions, so nothing
on the `0.2.x`/`1.0.0-rc` compiler line can ever be published over `1.5.3`.

**Decision (maintainer, 2026-07-16):** publish *forward* and decouple, rather
than wipe public history. The extension version line moves to `1.6.0` (ahead
of `1.5.3`) independent of the compiler's `1.0.0-rc.1`; the
`vscode_release_sync_smoke.rs` guard was relaxed from strict equality to
`extension core >= compiler core` in the same PR. This is non-destructive:
Marketplace version history and install stats are preserved, and a `v*` tag
push now publishes `1.6.0` cleanly.

The rejected alternative was `vsce unpublish` of the `1.x` line, which would
have wiped Marketplace version history and install-count continuity — a
destructive, maintainer-only action this repo deliberately does not take.

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
