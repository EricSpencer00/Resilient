# Release Audit (F-E6)

Tracks roadmap epic **F-E6** (docs/ROADMAP_PHASE2.md, Track F), part of the
broader v1.0 roadmap under [#3933](https://github.com/EricSpencer00/Resilient/issues/3933).
This began as a documentation-only audit. **Update (RES-4102, 2026-07-16):**
its recommendations have now been executed to cut the first release
candidate — the workspace was aligned to `1.0.0-rc.1` (Finding B resolved),
the extension version was moved forward to `1.6.0` and decoupled (E-E3
resolved, see [VSCODE_EXTENSION_RELEASE.md](VSCODE_EXTENSION_RELEASE.md)),
and a `v1.0.0-rc.1` tag is cut to trigger `release.yml` + `vscode_extension.yml`.
The `v1.5.x` tags are left in place as historical record per Finding A.

**Follow-up (RES-4102, `v1.0.0-rc.2`):** the `v1.0.0-rc.1` tag published the
extension (`1.6.0`) cleanly but its compiler-binary `release` job was skipped
because the `aarch64-apple-darwin` build leg failed — the "Install Z3
static-link build deps (Linux native only)" step in `release.yml` was missing
a `runner.os == 'Linux'` guard, so it ran `sudo apt-get` on the macOS leg
(#4101 added `z3: true` to that leg but not the guard). Fixed here; the
extension is bumped to `1.6.1` and `v1.0.0-rc.2` is cut as the first
fully-green release candidate. `v1.0.0-rc.1` is left in place (not moved) per
this repo's no-moving-pushed-tags policy; it simply has no attached GitHub
Release.

## 1. Tag inventory vs manifest versions

```
$ git tag | sort -V
v0.1.1  v0.1.2  v0.1.3  v0.1.4  v0.1.5  v0.1.6  v0.1.7
v0.2.0  v0.2.1  v0.2.2  v0.2.3
v1.5.2  v1.5.3
```

Manifest versions (original audit → post-RES-4102):

| Manifest | Audit | Now |
|---|---|---|
| `resilient/Cargo.toml` | `0.2.3` | `1.0.0-rc.1` |
| `resilient-runtime/Cargo.toml` | `0.2.1` | `1.0.0-rc.1` |
| `resilient-span/Cargo.toml` | `0.2.1` | `1.0.0-rc.1` |
| `playground/Cargo.toml` | `0.2.1` | `1.0.0-rc.1` |
| `vscode-extension/package.json` | `0.2.3` | `1.6.0` (decoupled — leads, see E-E3) |

**Finding A — the `v1.5.2`/`v1.5.3` tags predate the current tagging
scheme and don't reflect the compiler version.** Confirmed by checking out
`resilient/Cargo.toml` at both tag commits:

```
$ git show v1.5.2:resilient/Cargo.toml | grep '^version'
version = "0.2.0"
$ git show v1.5.3:resilient/Cargo.toml | grep '^version'
version = "0.2.0"
```

Both tags were cut while the compiler was at `0.2.0`. `git log --all` shows
the fix that closed this gap:

```
683a6f96 fix(release): tag from Cargo.toml semver, not VSCE package.json
9ed2c22a Release v1.5.3: bump VS Code extension version for Marketplace publish
```

Before that fix, the release tag was apparently driven by the VS Code
extension's `package.json` version rather than the compiler's — the `1.5.x`
line is a leftover of the extension's own version history (it had already
shipped Marketplace releases numbered independently, see
[docs/VSCODE_EXTENSION_RELEASE.md](VSCODE_EXTENSION_RELEASE.md)), not a
real `1.5.x` compiler release. Every tag from `v0.1.1` through `v0.2.3` was
(and, via `weekly-release.yml`, still is) cut strictly from
`resilient/Cargo.toml`'s `version` field — see "canonical version story"
below. No compiler code, CLI behavior, or language surface at `v1.5.2`/
`v1.5.3` differs in kind from what `v0.2.0`'s tag already captured; they are
not a more-advanced release than the `0.2.x` line, just a mislabeled one.

**Recommendation:** leave the `v1.5.2`/`v1.5.3` tags in place as an
immutable historical record (this repo does not delete or move tags that
have already been pushed — see CLAUDE.md's "force-pushing commits that are
already merged" hard stop, which applies in spirit to tags too) but treat
them as non-authoritative for compiler versioning going forward. Do not
create a `v1.0.0`-shaped tag by incrementing off `1.5.3` — the canonical
line is `0.2.x → 1.0.0`, per the vsce canonicalization decision already
recorded in memory (0.2.x is truth; the public Marketplace `1.5.3` listing
is a separate, maintainer-only reconciliation tracked as **E-E3**).

**Finding B — workspace-internal version drift.** `resilient-runtime`,
`resilient-span`, and `playground` are all pinned at `0.2.1`, one release
behind `resilient` itself (`0.2.3`). Nothing currently asserts these stay
in lockstep (unlike `vscode-extension/package.json`, which
`resilient/tests/it/vscode_release_sync_smoke.rs` pins to
`resilient/Cargo.toml`'s version). This is not a release blocker today —
these crates aren't published to crates.io and are consumed in-tree via
path dependencies — but it means "the workspace version" isn't a single
well-defined string. Before cutting `v1.0.0`, decide whether these three
manifests should be bumped to match `resilient/Cargo.toml` (recommended,
for a project explicitly adopting SemVer at 1.0 per STABILITY.md) or
whether they're allowed to version independently going forward; either
way, write the decision down, since STABILITY.md's SemVer section talks
about "the project" versioning, not a specific crate.

## 2. Canonical version story

- **Source of truth:** `resilient/Cargo.toml`'s `version` field. Every tag
  from `v0.1.1` onward that was cut through the current pipeline
  (`weekly-release.yml`) reads this field, checks whether `v<version>`
  already exists, and if not, tags and pushes it — see the workflow's own
  header comment for the full rationale.
- **What a tag push fires:** `release.yml` (cross-target binary build +
  GitHub release) and `vscode_extension.yml` (`.vsix` package + Marketplace
  publish), both triggered by any `v*` tag regardless of which workflow
  created it.
- **The VS Code Marketplace divergence (`1.5.3` live vs `0.2.3` in-repo)
  is a separate, already-documented, maintainer-only decision** — see
  [docs/VSCODE_EXTENSION_RELEASE.md](VSCODE_EXTENSION_RELEASE.md)'s
  "Marketplace divergence" section. This audit does not re-decide it; it
  is out of scope here the same way it's out of scope for every other
  agent PR (tracked as **E-E3**, explicitly maintainer-only per the Track F
  roadmap notes).

## 3. Dry run: hypothetical `v1.0.0-rc` tag push

**Nothing below was executed.** This traces what pushing a `v1.0.0-rc` tag
to `origin` would fire today, reading `.github/workflows/release.yml`,
`weekly-release.yml`, and `vscode_extension.yml` as they exist on this
branch.

### `release.yml` (`on: push: tags: ["v*"]`)

1. **`build` matrix** — 4 legs: `x86_64-unknown-linux-gnu` (native,
   `--features lsp,z3,z3-static`), `aarch64-unknown-linux-gnu` (cross,
   `--features lsp`, no z3), `x86_64-apple-darwin` and
   `aarch64-apple-darwin` (native, `--features lsp`, no z3).
   - **Gate to note:** only 1 of 4 release legs ships with Z3 support.
     This is an existing, intentional, documented limitation (RES-3979 —
     newer C++ compilers on macOS trip a Z3 4.12.1 build error; the
     `aarch64-linux` cross build doesn't natively execute the smoke test).
     It predates this audit and isn't introduced by cutting `v1.0.0-rc`,
     but it's worth a deliberate maintainer call before a real `v1.0.0`:
     shipping 3 of 4 platforms with Z3 verification entirely absent is a
     bigger deal at 1.0 than at `0.2.x`, since Z3 verification is one of
     the project's headline safety-critical selling points. This is the
     "Z3-off-by-default" gap referenced in prior session memory, scoped
     more precisely here: it's not that Z3 is off by default in local/CI
     builds (the `z3_ci` job exercises `--features z3` on every PR) — it's
     that 3 of 4 *release binaries* never get the feature at all.
   - Each leg runs its smoke test(s) (`scripts/release-smoke-test.sh` for
     the z3 leg, `scripts/release-lsp-smoke-test.sh` for every non-cross
     leg), packages a `rz-v1.0.0-rc-<target>.tar.gz`, and uploads it as a
     build artifact.
2. **`release` job** (`needs: build`, `if: startsWith(github.ref,
   'refs/tags/')`) — downloads all 4 artifacts and runs
   `gh release create v1.0.0-rc --title "Resilient v1.0.0-rc"
   --generate-notes artifacts/*.tar.gz`, publishing a GitHub Release
   with all 4 tarballs attached. This step would succeed as-is.

### `vscode_extension.yml` (`on: push: tags: ["v*"]` too)

1. **`build`** — type-check only, always green if `vscode-extension/`
   compiles.
2. **`package`** (`needs: build`, tag push only) — `vsce package`, uploads
   the `.vsix` as a build artifact. Succeeds regardless of Marketplace
   state (packaging doesn't touch the Marketplace).
3. **`publish`** (`needs: package`, tag push only) — `vsce publish --pat
   $VSCE_PAT`. **This job would fail today.** `vscode-extension/package.json`
   is `0.2.3`; the live Marketplace listing is `1.5.3`. The Marketplace
   enforces monotonically increasing versions per extension, so publishing
   `0.2.3` over `1.5.3` is rejected outright — this is the exact scenario
   `docs/VSCODE_EXTENSION_RELEASE.md` already flags. Practically: a
   `v1.0.0-rc` tag push produces a fully valid GitHub release (compiler
   binaries) and a red `vscode-extension / publish` workflow run
   (Marketplace). Neither `release.yml` nor `vscode_extension.yml` is in
   CLAUDE.md's required-status-checks list (that list gates PR merges into
   `main`, not tag-triggered release infra), so this failure would not
   block anything — it would just leave a visibly red Actions run on the
   tag, and the extension would not actually update on the Marketplace
   for that release.

### `weekly-release.yml`

Cron/dispatch-only; a manual `v1.0.0-rc` tag push does not invoke it. It
reads `resilient/Cargo.toml`'s version, checks whether `v<version>`
already exists, and no-ops if so. If `resilient/Cargo.toml` were bumped to
exactly `1.0.0-rc` as part of cutting the tag, the next scheduled run would
see `v1.0.0-rc` already exists and no-op correctly — no double-tagging risk.

### Gate `release.yml` does *not* check

Nothing in `release.yml` or `weekly-release.yml` verifies that the tagged
commit's own CI (the required-status-checks set gating PR merges,
including the new `conformance` job added in this PR) was green on `main`
before building release artifacts. In practice every tag today is cut from
a `main` HEAD that already passed those checks via the normal PR flow, but
that's a convention, not an enforced gate in the release pipeline itself.

## 4. Concrete steps to cut a real `v1.0.0-rc` (not executed here)

1. Confirm the `main` commit to be tagged has every required status check
   green (`build / test / clippy`, `conformance suite (STABILITY.md Stable
   surface)`, `build / test with --features z3`, `board hygiene`, the
   three `resilient-runtime`/Cortex-M cross-compile checks, and the
   `.text` budget check).
2. Resolve the VS Code Marketplace divergence (**E-E3**, maintainer-only —
   see docs/VSCODE_EXTENSION_RELEASE.md's two reconciliation options)
   *before* pushing any tag that will also fire `vscode_extension.yml`'s
   `publish` job, or accept that `publish` will fail loudly and treat the
   GitHub-release half of the cut as the only thing that actually ships.
3. Decide whether `resilient-runtime`, `resilient-span`, and `playground`
   should be bumped to match `resilient/Cargo.toml` (Finding B above) —
   write the decision down in the same PR that bumps versions.
4. Bump `resilient/Cargo.toml`'s `version` to `1.0.0-rc.1` (Cargo/SemVer
   pre-release syntax) and keep `vscode-extension/package.json` in sync
   (or formally decouple it per option 2 in
   docs/VSCODE_EXTENSION_RELEASE.md, in the same PR).
5. Land that version-bump PR through the normal ship-to-merge flow so it
   passes every required check on `main`.
6. From `main` at that commit: `git tag -a v1.0.0-rc.1 -m "..."` then
   `git push origin v1.0.0-rc.1` — a maintainer or agent action outside
   the scope of this audit.
7. Watch `release.yml`'s 4-leg build matrix and the `release` job's
   GitHub Release creation; watch (or knowingly accept the expected
   failure of) `vscode_extension.yml`'s `publish` job per step 2.
8. Download one of the published tarballs and confirm `rz --version`
   still prints the pre-1.0 stability banner correctly for an `-rc` tag
   (STABILITY.md's pre-1.0 rules stay in effect until the real `v1.0.0`,
   not the `-rc`).

## 5. RES-3985 follow-up: closing Finding B and most of the Z3 gap

This section records what changed after the audit above; it doesn't
re-run the audit, just resolves the two open items it flagged.

**Finding B (workspace version drift) — resolved.** `resilient-runtime`,
`resilient-span`, and `resilient-playground` are now `0.2.3`, matching
`resilient/Cargo.toml`. The decision (option 1 from step 3 above: bump to
match, rather than let them drift independently) is written down in
STABILITY.md's "Versioning Intent" section, which now states plainly
that `resilient/Cargo.toml`'s version is canonical and these three
crates are kept in lockstep with it going forward.

**Z3-by-default gate — 3 of 4 targets now, not 1 of 4.** The "shipping 3
of 4 platforms with Z3 verification entirely absent" gap called out in
section 3 above is mostly closed. `release.yml` now builds
`--features z3,z3-static` for `x86_64-unknown-linux-gnu` (unchanged),
`aarch64-unknown-linux-gnu` (cross-compiled, via a `Cross.toml`
`pre-build` hook that installs `python3` into the `cross-rs` Docker
image — the only thing missing from it; cmake, the GNU cross C++
toolchain, and libclang were already present, verified by inspecting the
image directly), and `aarch64-apple-darwin` (built natively through
Homebrew GCC 13 instead of Apple Clang, which is what was blocking it —
see the "Build (native, Z3 static-linked, macOS via GCC)" step's comment
in `release.yml` for the two extra link flags GCC needs that Clang
didn't: `CXXSTDLIB=stdc++` and an explicit `-l emutls_w` for GCC's
emulated-TLS helper). Verified end-to-end locally on
`aarch64-apple-darwin`: the built binary has no `libz3` runtime
dependency (`otool -L`) and `scripts/release-smoke-test.sh` passes,
i.e. `rz --audit` genuinely discharges the fixture's obligation via Z3.

**`x86_64-apple-darwin` remains the one no-z3 leg.** It's a cross-arch
build from the arm64 `macos-latest` runner, and Homebrew's `gcc@13`
bottle is arm64-native only — passing `-arch x86_64` to it silently
produces an arm64 object instead of cross-compiling (verified locally:
`g++-13 -arch x86_64 -c t.cpp -o t.o` emits a Mach-O arm64 object with a
warning that the flag was ignored). Closing this fully would mean
bootstrapping a second, Rosetta-hosted Intel Homebrew prefix
(`arch -x86_64 /usr/local/bin/brew install gcc@13`) purely to get an
x86_64-native GCC — plausible, but unvalidated here, and risky to wire
into `release.yml` blind since this repo's local dev environment can
build and verify the `aarch64-apple-darwin` leg directly but has no way
to validate an `x86_64-apple-darwin` cross-build's link step without
running it. Left as the residual scope of #3985; the README and
`release.yml` matrix comment both document the current 3-of-4 state
precisely rather than overclaiming.
