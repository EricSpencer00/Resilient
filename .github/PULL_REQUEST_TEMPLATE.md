<!--
Thanks for contributing to Resilient!

Before you hit "Create pull request":
  - Read CONTRIBUTING.md (dev setup, ticket workflow, commit format)
  - Keep this PR focused on a single ticket where possible
  - Match the commit subject `RES-NNN: short description`
-->

## Description

_What does this PR change? Describe the *why* more than the *what* —
reviewers can read the diff for the what._

## Linked issue

Closes #<issue-number>

<!-- If there is no GitHub issue but there is a .board ticket, reference it:
Ticket: `.board/tickets/IN_PROGRESS/RES-NNN-*.md`
-->

## Acceptance criteria

<!-- Copy the acceptance criteria from the linked issue and tick them off as
they land in this PR. -->

- [ ] _Criterion 1_
- [ ] _Criterion 2_
- [ ] _Criterion 3_

## Test evidence

<!-- Paste the commands you ran and summarise the result. Screenshots /
logs welcome for user-visible behaviour. -->

```
$ cargo test --manifest-path resilient/Cargo.toml
... all tests passed ...
```

- [ ] New tests added (unit and/or `.expected.txt` golden)
- [ ] `cargo fmt --all` clean
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo build --manifest-path resilient/Cargo.toml` succeeds with any
      feature flags this PR touches

## Stability impact

<!-- If this PR changes the language surface, note which STABILITY.md
category is affected and whether a CHANGELOG entry was added.
Delete this section if it is purely internal / not surface-visible. -->

- [ ] STABILITY.md CHANGELOG updated (if user-visible syntax or builtin
      changed)
- [ ] `.board/` ticket moved from `IN_PROGRESS/` to `DONE/` (with closing
      commit hash recorded)

## Notes for reviewers

_Anything tricky, surprising, or explicitly out-of-scope?_
