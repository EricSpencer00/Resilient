---
id: RES-261
title: "formatter.rs: replace infallible `unwrap()` in BytesLiteral with `char::from`"
state: DONE
priority: P4
goalpost: tooling
created: 2026-04-20
owner: executor
Claimed-by: Claude
Closed-by: 10d3527
---

## Summary

Replaced `std::str::from_utf8(&buf).unwrap()` with `char::from(x).to_string()` in
the `BytesLiteral` match arm of `formatter.rs`. The temporary `buf` array was removed.
No behavioural change — pure refactor to eliminate the infallible `unwrap()`.

## Log

- 2026-04-20 created by analyzer
- 2026-04-20 claimed and fixed by Claude (RES-261 RES-260 RES-259 commit)
