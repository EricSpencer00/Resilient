---
layout: page
title: Tutorial verification
nav_order: 34
permalink: /tutorial-verification/
---

# Tutorial verification

Run the tutorial source blocks with Python 3 and a built compiler:

```bash
RESILIENT_BIN=/absolute/path/to/rz bash docs/verify_tutorial_snippets.sh
```

The shell entry point delegates to `docs/verify_tutorial_snippets.py`. It finds
the compiler through `RESILIENT_BIN`, the repository's release/debug build paths,
or `PATH`. An explicit `--compiler` overrides discovery.

The verifier reads `docs/tutorial/*.md` in filename order and executes fences
whose language tag is `resilient`. It handles backtick and tilde fences, longer
closing fences, up to three spaces of indentation, trailing whitespace and
CRLF input. Text inside another language's code fence is not executable
Resilient source.

Missing lessons, zero source blocks, empty source fences and unterminated
fences fail verification. Each execution uses seed zero, disables the compiler
cache, and has a 30-second timeout. Override it with `--timeout SECONDS` when a
lesson intentionally needs more time. Compiler failures report the original
lesson path, fence line and the tail of the compiler's diagnostic output.
A successful result includes the number of snippets that actually ran.

This checks execution, not displayed-output equivalence or formal proofs.
Adjacent shell commands are not executed. Certificate emission, independent
solver replay and the compiler's enabled verification features require separate
checks. The verifier does not add a required CI workflow.

Run the isolated coverage/failure regressions with:

```bash
python3 -m unittest discover -s docs/tests -p 'test_tutorial_snippets.py' -v
```
