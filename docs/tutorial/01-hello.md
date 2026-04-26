---
title: "1. Hello, Resilient"
parent: Tutorial
nav_order: 1
permalink: /tutorial/01-hello
---

# 1. Hello, Resilient
{: .no_toc }

Install the compiler, run your first program, pick a backend.
{: .fs-5 .fw-300 }

---

## Install

Pick the path that fits — see [Getting Started]({{ site.baseurl }}/getting-started#install)
for the full menu. The shortest path:

```bash
curl -fsSL https://raw.githubusercontent.com/EricSpencer00/Resilient/main/scripts/install.sh | bash
# Then add ~/.rz/bin to $PATH (the script prints the line for your shell).
rz --version
```

If you have Rust locally and prefer to build from source:

```bash
git clone https://github.com/EricSpencer00/Resilient.git
cd Resilient
cargo install --path resilient
# `rz` lands in ~/.cargo/bin/rz
```

If you don't want anything local, the [Docker image]({{ site.baseurl }}/getting-started#docker-res-203)
is a one-liner:

```bash
docker run --rm ghcr.io/ericspencer00/resilient:latest --help
```

## Your first program

Open `hello.rz` and paste:

```resilient
fn main() {
    println("Hello, Resilient world!");
}
main();
```

Then run it:

```bash
rz hello.rz
```

You should see:

```
Hello, Resilient world!
Program executed successfully
```

Two things worth calling out:

- **The file ends in `.rz`.** Resilient source uses the `.rz`
  extension. Install the [VS Code extension](https://marketplace.visualstudio.com/items?itemName=fromamerica.resilient-vscode)
  for syntax highlighting and one-click run.
- **`main();` at the bottom.** Functions declared with `fn`
  are not auto-invoked. The last line kicks off execution.
  If you forget it, the program runs fine but doesn't print
  anything.

## Three backends

Resilient ships with three execution modes: a tree-walking
interpreter (default, most features), a bytecode VM
(`--vm`, faster on hot loops), and a Cranelift JIT (`--jit`,
native machine code for the numeric-heavy subset).

For the same program:

```resilient
fn fib(int n) -> int {
    if n < 2 { return n; }
    return fib(n - 1) + fib(n - 2);
}
fn main() {
    println(fib(20));
}
main();
```

Save as `fib.rz` and run through each:

```bash
rz fib.rz          # tree-walker (default)
rz --vm fib.rz     # bytecode VM
# JIT needs a feature-flagged build:
cargo install --path resilient --features jit  # one-time
rz --jit fib.rz
```

All three print `6765`. The walker is a few hundred ms; the
VM shaves ~30x; the JIT drops into the sub-millisecond
range. The [performance page]({{ site.baseurl }}/performance)
has the numbers.

## What you learned

- `fn`, `println`, and the `main();` call-site idiom.
- How to run a `.rz` file with `rz`.
- That there are three backends; you can pick the one that
  fits your workload.

## What's next

→ [2. Variables and types]({{ site.baseurl }}/tutorial/02-variables-and-types)
