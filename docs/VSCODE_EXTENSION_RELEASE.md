# VS Code Extension Release Sync

The VS Code extension version is intended to track the compiler crate version
in `resilient/Cargo.toml`.

Tracked release state:

- `resilient/Cargo.toml` is the compiler release source of truth.
- `vscode-extension/package.json` must use the same version.
- `.github/workflows/vscode_extension.yml` publishes the extension on `v*`
  release tags with the `VSCE_PAT` GitHub Actions secret.
- `.github/workflows/vsce-token-check.yml` verifies the PAT before release day.
- `npm run vscode:publish` is the manual fallback from `vscode-extension/`.

## Marketplace History

The Marketplace can contain older accidental versions that are numerically
higher than the compiler version. Publishing `0.2.x` after `1.5.x` does not
make `0.2.x` outrank `1.5.x` under semver ordering.

If the Marketplace still presents `1.5.x` as latest after a synced `0.2.x`
publish, fix it from the publisher account by either:

1. unpublishing the accidental `1.5.x` versions, or
2. choosing an explicit forward-only extension version policy and documenting
   that the extension no longer mirrors the compiler crate version.

Do not roll `package.json` backward below the compiler version; that preserves
the mismatch instead of fixing Marketplace history.
