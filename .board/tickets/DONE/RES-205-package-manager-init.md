---
id: RES-205
title: `resilient pkg init` creates a minimal project skeleton
state: DONE
priority: P3
goalpost: ecosystem
created: 2026-04-17
owner: executor
---

## Summary
One-liner onboarding: `resilient pkg init my-proj` creates a
directory with a manifest, a hello-world entrypoint, and a
gitignore. Doesn't yet imply a full package system — `init` is
the cheapest useful subcommand and lets us validate the manifest
schema before we build dependency resolution.

## Acceptance criteria
- Subcommand `resilient pkg init <name>`:
  - Creates `<name>/`.
  - Writes `<name>/Resilient.toml`:
    ```toml
    [package]
    name = "<name>"
    version = "0.1.0"
    edition = "2026-04"
    ```
  - Writes `<name>/src/main.rs` with a fn main that prints
    "Hello, world!".
  - Writes `<name>/.gitignore` ignoring `target/` and `cert/`.
- `resilient pkg init <existing-nonempty-dir>` errors without
  writing anything.
- Unit test: exec the subcommand in a tempdir, assert file
  presence + content.
- Commit message: `RES-205: pkg init subcommand`.

## Notes
- Don't implement `pkg add` / `pkg build` / dep resolution in
  this ticket — each deserves its own design pass.
- `edition` gives us a migration vector for future breaking
  changes. Start with an edition string tied to today's date for
  now.

## Resolution

### Files added
- `resilient/src/pkg_init.rs` — new module. Public API:
  - `scaffold_in(parent: &Path, name: &str) -> Result<Scaffold, PkgInitError>`
  - `render_manifest(name) / render_hello_world() / render_gitignore()` — pure templates, unit-testable.
  - `PkgInitError` enum — `MissingName`, `InvalidName`, `DirectoryNotEmpty`, `Io(io::Error)`. `Display` impl emits
    user-friendly messages. `From<io::Error>` for `?` ergonomics.
  - `DEFAULT_EDITION = "2026-04"` — date-style edition per ticket note.
  - 10 unit tests in `mod tests` covering template content, empty-dir accept, non-empty-dir refuse,
    preserved-stray-file invariant, and all the `validate_name` rejection cases (path separators, whitespace,
    empty, `.`, `..`) plus acceptance of common forms.
- `resilient/src/main.rs` — added `mod pkg_init;` and a new
  `dispatch_pkg_subcommand(args)` helper. `main()` calls it before the existing arg-parser so the `pkg` verb
  doesn't have to negotiate the flag grammar. Returns `Some(exit_code)` for handled verbs, `None` to fall
  through. Subcommand grid:
  - `pkg init <name>` → scaffold + print "next steps"; exit 0 on success, 1 on directory-not-empty or IO, 2 on
    missing-name / invalid-name.
  - `pkg init` (no name) → exit 2 with "requires a project name: `resilient pkg init <name>`".
  - `pkg <unknown>` → exit 2 with "unknown pkg subcommand `<x>`. Known: init".
  - `pkg` (no subcommand) → exit 2 with "requires a subcommand. Known: init".
- `resilient/tests/pkg_init_smoke.rs` — 4 integration tests that spawn the real binary with
  `current_dir(<tempdir>)`:
  - `pkg_init_creates_project_skeleton` — layout + content
  - `pkg_init_errors_on_nonempty_directory` — refusal + invariant preservation
  - `pkg_init_missing_name_errors` — exit code + usage hint
  - `pkg_unknown_subcommand_errors` — graceful failure

### End-to-end
```
$ cd /tmp/<scratch> && resilient pkg init my_new_proj
Created my_new_proj at …/my_new_proj
  wrote …/my_new_proj/Resilient.toml
  wrote …/my_new_proj/src/main.rs
  wrote …/my_new_proj/.gitignore

Next steps:
  cd my_new_proj
  resilient src/main.rs
$ resilient my_new_proj/src/main.rs
Hello, world!
Program executed successfully
```

### Notes
- The hello-world template uses explicit `\n    ` rather than Rust's line-continuation `\<newline>` — the
  latter collapses the leading indentation on body lines. Caught this on a manual end-to-end test; test
  file asserts `fn main` + `"Hello, world!"` presence but not exact whitespace.
- `DEFAULT_EDITION` is a `const` in the module (exposed `pub` for tests and future `pkg add` to reuse). Per
  ticket note, edition is a "migration-vector date" not a semver; the date tracks today's manager date.
- The module lives in `resilient/src/pkg_init.rs` (alongside the compiler pipeline) rather than a dedicated
  crate because the ticket says "Don't implement `pkg add`/`build`/dep resolution in this ticket" — keeping
  everything in the single binary for now minimizes over-engineering.

### Verification
- `cargo build` → clean
- `cargo test --locked` → 488 + 16 + 4 + 3 + 1 + 12 + 4 tests pass
  (478 → 488 = 10 new pkg_init unit tests; +4 pkg_init_smoke integration tests in the final line)
- `cargo test --locked --features lsp` → 505 + 16 + 4 + 3 + 1 + 12 + 6 + 4 pass
- `cargo clippy --locked --features lsp,z3,logos-lexer --tests -- -D warnings` → clean
- End-to-end manual check: `pkg init` + running the scaffolded main.rs prints "Hello, world!"

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (`pkg init <name>` wired as a pre-flag subcommand; 14 tests; end-to-end
  scaffold-then-run verified)
