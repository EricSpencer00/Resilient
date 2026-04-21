---
id: RES-255
title: "`resilient pkg add` — declare a dependency in resilient.toml"
state: OPEN
priority: P3
goalpost: tooling
created: 2026-04-20
owner: executor
---

## Summary

`resilient pkg init` (RES-205) scaffolds a new project but the `[dependencies]`
table it creates in `resilient.toml` is empty and there is no way to add
entries without editing the file by hand. `resilient pkg add <name> [--version <v>]`
should be the standard way to declare a dependency.

RES-205 explicitly deferred this: "Don't implement `pkg add` / `pkg build` /
dep resolution in this ticket."

## Acceptance criteria

- `resilient pkg add <package-name>` appends a new entry to the
  `[dependencies]` table in the nearest `resilient.toml` (found by
  walking upward from cwd via `pkg_init::find_manifest_upwards`).
- `--version <semver>` flag specifies the version constraint (default
  `"*"` if omitted).
- `--path <relative-path>` flag specifies a local path dependency.
- Running `resilient pkg add` when no `resilient.toml` is found exits 2
  with a helpful error ("no resilient.toml found; run `resilient pkg init` first").
- Adding a name that already exists in `[dependencies]` exits 1 with
  "dependency `<name>` already declared; edit resilient.toml to update it".
- `resilient pkg --help` and `resilient pkg add --help` are updated to
  list the new subcommand.
- Integration tests in `tests/pkg_init_smoke.rs` (or a new `pkg_add_smoke.rs`)
  cover the happy path, duplicate-name error, and missing-manifest error.
- **No** dep resolution, fetching, or build graph — only manifest mutation.
- Commit: `RES-255: \`resilient pkg add\` — declare a dependency in resilient.toml`.

## Notes

- The manifest is TOML; use the `toml_edit` crate (preserves comments and
  formatting) or a minimal hand-rolled TOML appender. Either way, do NOT
  parse with `toml` and re-serialize — that would strip comments.
- `pkg_init::MANIFEST_FILENAME` is the canonical manifest filename.
- `pkg_init::find_manifest_upwards` already walks upward to find the
  manifest.
- `pkg_init::render_manifest` shows the expected `[dependencies]` table shape.
- Supply-chain note: if adding `toml_edit`, explain the choice in the PR
  description per the security rules in CLAUDE.md.

## Log

- 2026-04-20 created by analyzer (deferred in RES-205 closing note)
