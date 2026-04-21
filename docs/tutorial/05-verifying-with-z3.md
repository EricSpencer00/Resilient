---
title: "5. Verifying with Z3"
parent: Tutorial
nav_order: 5
permalink: /tutorial/05-verifying-with-z3
---

# 5. Verifying with Z3
{: .no_toc }

Build with `--features z3`, emit a certificate, re-verify it
with stock Z3.
{: .fs-5 .fw-300 }

---

## Prerequisite

This lesson needs a Resilient build with the Z3 SMT solver
enabled, and the `z3` binary on your `$PATH`. On macOS:

```bash
brew install z3
cd resilient
cargo build --release --features z3
```

On Debian / Ubuntu:

```bash
sudo apt-get install -y z3 libz3-dev
cd resilient
cargo build --release --features z3
```

If you skipped this setup, the snippets below still *run* —
they just don't emit certificates. The whole point of the
lesson, though, is the certificate.

## Contracts the cheap folder can't decide

Resilient always runs a *cheap fold* first: simple algebraic
simplifications that handle tautologies like `x + 0 == x`
without bothering Z3. Most contracts discharge at that layer.

But some clauses involve free variables the folder can't
eliminate. Take this program:

```resilient
fn ident_round(int x) -> int
    requires x + 0 == x
{
    return x;
}

fn main() {
    let r = ident_round(42);
    println(r);
}
main();
```

`x + 0 == x` is a tautology *for every integer x*. The cheap
folder can't conclude that on its own (it doesn't
algebraically distribute `+` over a free var). Z3 can.

## Emitting a certificate

With the Z3 feature enabled, and the `--emit-certificate <dir>`
flag, the compiler writes one SMT-LIB2 file per discharged
obligation:

```bash
mkdir -p certs
resilient --typecheck --emit-certificate certs path/to/ident_round.rs
```

You'll see:

```
Running type checker...
Type check passed
Wrote 1 verification certificate(s) to certs
42
Program executed successfully
```

Look at `certs/`:

```
certs/
├── ident_round__decl__0.smt2
└── manifest.json
```

The `.smt2` file is a self-contained SMT-LIB2 script:

```
; RES-071 verification certificate
; expected solver result: unsat (proves the contract is a tautology)
(set-logic AUFLIA)
(declare-const x Int)
(assert (not (= (+ x 0) x)))
(check-sat)
```

The script asserts the **negation** of the contract clause and
asks the solver for `(check-sat)`. If it returns `unsat`, the
negation is unsatisfiable, which means the original clause is
valid — the contract always holds.

## Re-verifying independently

Stock Z3 reads the file without any Resilient-specific setup:

```bash
z3 -smt2 certs/ident_round__decl__0.smt2
```

Output:

```
unsat
```

That's the proof. A downstream consumer doesn't have to trust
the Resilient compiler — they run the certificate through
their own solver and get the same answer.

## The manifest + `verify-all`

`certs/manifest.json` is an index of every obligation's
cert + SHA-256 + (optionally) Ed25519 signature. The
`verify-all` subcommand walks it:

```bash
resilient verify-all certs
```

```
Verifying 1 obligation(s) from path/to/ident_round.rs
  fn                               kind       sha256   sig      z3
  ident_round::decl[0]             decl       ok       -        -
verify-all: all checks passed
```

The `z3` column is skipped by default — add `--z3` to have
`verify-all` shell out to the `z3` binary for each cert and
require `unsat`:

```bash
resilient verify-all --z3 certs
```

## What you learned

- Most contracts discharge via the cheap fold; some need Z3.
- `--emit-certificate <dir>` writes an SMT-LIB2 file per
  obligation Z3 proved.
- The cert is self-contained — any SMT-LIB2 solver can
  re-verify it independently of Resilient.
- `verify-all <dir>` walks the manifest + re-checks hashes,
  signatures, and (with `--z3`) the SMT proofs themselves.

## What's next

You've got the full tour: from `Hello, world!` to independently
auditable proofs of your contracts. Next steps:

- Skim the [syntax reference]({{ site.baseurl }}/syntax)
  for details this tutorial skipped (`match`, structs,
  arrays, `Result` + `?`).
- Read the [design philosophy]({{ site.baseurl }}/philosophy)
  for WHY the language leans into self-healing +
  verification.
- Dig into a real example: `resilient/examples/sensor_monitor.rs`
  exercises the full feature set in ~70 lines.
- Browse [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues)
  to see what's next and what's in flight.
