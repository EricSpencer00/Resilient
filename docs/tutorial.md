---
title: Tutorial
nav_order: 3
has_children: true
permalink: /tutorial
---

# Tutorial
{: .no_toc }

Five short lessons that take you from `Hello, world!` to a
self-healing `live` block whose contracts are discharged by
Z3. Each one builds on the last. Every code snippet in every
lesson is verified by `docs/verify_tutorial_snippets.sh` — if
a snippet stops working, CI notices.
{: .fs-6 .fw-300 }

## What you'll build

1. **[Hello, Resilient]({{ site.baseurl }}/tutorial/01-hello)** —
   install the compiler, run your first program, peek at the
   three execution backends.
2. **[Variables and types]({{ site.baseurl }}/tutorial/02-variables-and-types)** —
   `let`, primitives, optional type annotations, and how
   `--typecheck` catches mismatches.
3. **[Functions and contracts]({{ site.baseurl }}/tutorial/03-functions-and-contracts)** —
   `fn` + `requires` + `ensures`, and what `--audit` reports
   back.
4. **[Live blocks]({{ site.baseurl }}/tutorial/04-live-blocks)** —
   the self-healing construct: how retries work, how to read
   the runtime telemetry.
5. **[Verifying with Z3]({{ site.baseurl }}/tutorial/05-verifying-with-z3)** —
   build with `--features z3`, emit a certificate, re-verify
   it with stock Z3.

## Conventions

Code blocks marked ```resilient are runnable as-is:

```bash
resilient path/to/snippet.rz
```

Copy-paste any one of them into a `.rz` file and run it.
Install the [VS Code extension](https://marketplace.visualstudio.com/items?itemName=fromamerica.resilient-vscode)
for syntax highlighting and one-click run. See the
[syntax reference]({{ site.baseurl }}/syntax) for the grammar.

If you'd rather start with the grand tour, the
[getting-started guide]({{ site.baseurl }}/getting-started) is
the 60-second version; the tutorial is the 30-minute version.
