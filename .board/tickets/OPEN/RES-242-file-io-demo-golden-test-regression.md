---
id: RES-242
title: "file_io_demo golden test regression: file_read returns empty string"
state: OPEN
priority: P2
goalpost: G3
created: 2026-04-20
owner: executor
---

## Summary

The `golden_outputs_match` test in `resilient/tests/examples_golden.rs` is failing
for the `file_io_demo.res` example. The test harness runs the example and compares
its output against the expected output in `examples/file_io_demo.expected.txt`.

**Expected output:**
```
wrote: Hello from Resilient file I/O!
read:  Hello from Resilient file I/O!
Program executed successfully
```

**Actual output:**
```
wrote: Hello from Resilient file I/O!
read:
Program executed successfully
```

The `file_read` builtin is returning an empty string instead of the contents of the
file that was just written by `file_write`. This indicates a runtime bug in either
the `file_read` or `file_write` builtins (both added in RES-143).

### Example code (file_io_demo.res)

```resilient
fn main() {
    let path = "/tmp/resilient_file_io_demo.txt";
    let greeting = "Hello from Resilient file I/O!";

    file_write(path, greeting);
    let read_back = file_read(path);

    println("wrote: " + greeting);
    println("read:  " + read_back);
}
```

## Acceptance criteria

- Investigate why `file_read(path)` returns an empty string after
  `file_write(path, greeting)` writes to the same path.
- Fix the underlying bug in `file_read` or `file_write` implementation
  (likely in `src/interpreter.rs` builtin handling).
- Verify that `cargo test` passes, including `golden_outputs_match`.
- Verify that the `file_io_demo.res` example produces the expected output.
- `cargo clippy --all-targets -- -D warnings` remains clean.
- Commit message: `RES-240: fix file_read regression in file_io_demo`.

## Affected code

- `resilient/src/interpreter.rs` — `file_read` and `file_write` builtins
- `resilient/examples/file_io_demo.res` — test example
- `resilient/examples/file_io_demo.expected.txt` — golden expected output

## Testing

1. Run `cargo test --test examples_golden` to reproduce the failure.
2. Manually test: `resilient examples/file_io_demo.res` to see the empty string bug.

## Notes

- Do not modify the `.expected.txt` golden file; fix the implementation.
- This may be a file handle/filesystem synchronization issue, or the
  `file_read` implementation may not be reading from the correct file or
  handling file paths correctly.

## Log

- 2026-04-20 created by analyzer (golden test failure discovered during analysis)
