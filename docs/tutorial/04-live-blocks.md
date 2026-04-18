---
title: "4. Live blocks"
parent: Tutorial
nav_order: 4
permalink: /tutorial/04-live-blocks
---

# 4. Live blocks
{: .no_toc }

The self-healing construct: retries, invariants, telemetry.
{: .fs-5 .fw-300 }

---

## What is a `live` block?

A `live` block is the core Resilient construct for
self-healing code. Its body runs; if anything inside
**panics** — a failed `assert`, a contract violation, a
division by zero — the runtime doesn't just bubble the error
up. Instead it:

1. Logs the failure.
2. Restores the caller's local state to what it was *before*
   the block entered.
3. Retries the body, up to a bounded retry count.

If the bound is exhausted, the error finally propagates. If
the body succeeds on any retry, the program continues as if
the earlier failures had never happened.

## A concrete example

This program "fails" twice before succeeding on the third
attempt. `static let counter` survives the state restore
(which only resets LOCAL bindings inside the live block), so
each retry increments the counter and eventually clears the
failing condition:

```resilient
static let counter = 0;

fn flaky() -> int {
    counter = counter + 1;
    if counter < 3 {
        assert(false);
    }
    return counter;
}

fn main() {
    live {
        let n = flaky();
        println("succeeded on attempt " + n);
    }
}
main();
```

Run it:

```
[LIVE BLOCK] Starting execution of live block
[LIVE BLOCK] Error detected (attempt 1/3): ASSERTION ERROR: …
[LIVE BLOCK] Retrying execution (attempt 2/3)
[LIVE BLOCK] Error detected (attempt 2/3): ASSERTION ERROR: …
[LIVE BLOCK] Retrying execution (attempt 3/3)
succeeded on attempt 3
[LIVE BLOCK] Successfully executed live block
Program executed successfully
```

The retry budget is 3 by default. Exceeding it propagates the
final error — `live` is a recovery mechanism, not an infinite
loop.

## Invariants

Sometimes "no panic" isn't enough — you want to guarantee a
property holds after the block. `invariant <expr>` on the
same line as `live` makes the runtime check that expression
after every iteration:

```resilient
fn main() {
    let total = 0;
    live invariant total >= 0 {
        total = total + 1;
    }
    println(total);
}
main();
```

The invariant `total >= 0` holds trivially, so the block
succeeds on the first attempt. If an iteration ever left
`total` negative, the runtime would treat it as a failure
and restart the block.

## Telemetry

`live_total_retries()` returns the total retry count
accumulated across every `live` block the process has run.
Useful for health dashboards — a spike signals something
flaky in production.

```resilient
static let counter = 0;

fn flaky() -> int {
    counter = counter + 1;
    if counter < 3 {
        assert(false);
    }
    return counter;
}

fn main() {
    live {
        let n = flaky();
        println("done on " + n);
    }
    println("retries: " + live_total_retries());
}
main();
```

Prints `retries: 2` — the number of failed attempts before
the block finally succeeded. `live_total_exhaustions()` is the
complementary counter for blocks that hit the retry cap.

## What you learned

- `live { … }` retries the body on any panic, up to a
  bounded retry count, with state restored between retries.
- `live invariant <expr> { … }` additionally asserts a
  property after each iteration; failing the invariant is
  treated like a panic.
- `live_total_retries()` + `live_total_exhaustions()` give
  you process-wide health signals.

## What's next

→ [5. Verifying with Z3]({{ site.baseurl }}/tutorial/05-verifying-with-z3)
