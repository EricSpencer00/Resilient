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

As of RES-4102 the extension is at **`1.6.0`** and the compiler at
**`1.0.0-rc.1`**: the version numbers are intentionally decoupled, so the
extension's own CHANGELOG — not the number — is the source of truth for
which compiler version a given extension release targets.

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
