//! RES-198: `resilient lint` — five starter lints.
//!
//! Each lint has a stable code (`L0001`..`L0005`) and a
//! `# [allow]`-style suppress syntax: `// resilient: allow L0003`
//! on the line IMMEDIATELY ABOVE the offending node.
//!
//! Lints are WARNINGS by default. The CLI's `--deny L0001`
//! (mirrors `rustc -D`) escalates a specific code to error
//! severity; `--allow L0001` downgrades to suppressed. Unknown
//! codes on either flag are a usage error.
//!
//! Design notes
//! ============
//! - We build on the existing AST + span machinery (no new
//!   lexer work). Comment-based suppress is reconstructed by
//!   scanning the source text for the allow pattern independently
//!   of the parser; the set of suppressed `(line, code)` pairs is
//!   the filter applied to the raw lint output.
//! - Lints walk the AST top-down. Each lint is a separate
//!   function so a future `--only L0003` or `-W all` escalation
//!   has a clean seam to hook into.
//! - The module exports `check(program, source) -> Vec<Lint>`.
//!   Main wires that into the `lint <file>` subcommand.

use crate::{Node, Pattern, span::Span};
use std::sync::atomic::{AtomicBool, Ordering};

/// RES-198: one lint hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    /// Stable code, e.g. "L0001". Matches the code a user
    /// writes in `// resilient: allow L0001` to suppress.
    pub code: String,
    pub severity: Severity,
    /// Human-friendly diagnostic text.
    pub message: String,
    /// Location of the offending node (1-indexed).
    pub line: u32,
    pub column: u32,
}

/// RES-198: lint severity. Warning by default; `--deny` on the
/// CLI escalates to Error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Warning => write!(f, "warning"),
            Severity::Error => write!(f, "error"),
        }
    }
}

/// RES-198: the stable list of lint codes this module emits.
/// `--deny <code>` / `--allow <code>` arguments are validated
/// against this list in the CLI so typos are rejected early.
pub const KNOWN_CODES: &[&str] = &[
    "L0001", // unused local binding
    "L0002", // unreachable arm after `_`
    "L0003", // self-comparison `x == x`
    "L0004", // mixing `&&` and `||` without parens
    "L0005", // redundant trailing bare `return;`
    "L0006", // assume(false) vacuously discharges all verification obligations
    "L0007", // unreachable code after unconditional `return`
    "L0008", // duplicate identical struct literal match arm
    "L0009", // integer division by zero (literal / SMT-proven-possible)
    "L0010", // function has no requires/ensures contract
    "L0011", // RES-308: unused variable warning (let binding never read)
    "L0012", // RES-397: spec annotation lacks `// source:` provenance comment
    "L0013", // RES-798: unchecked array indexing (not proven in-bounds)
    "L0014", // function defined but never called (dead function)
    "L0015", // constant arithmetic expression overflows `int`
    "L0016", // constant boolean condition in `if` (always-true or always-false)
    "L0017", // variable binding shadows an outer binding of the same name
    "L0018", // function with return type may not return on all paths
    "L0019", // format() argument count does not match template placeholder count
    "L0020", // function parameter is never used in the body
    "L0021", // redundant boolean sub-expression (x && x, x || x)
    "L0022", // else branch after unconditional return is redundant
    "L0023", // tautological comparison with boolean literal (x == true)
    "L0024", // struct literal is missing one or more required fields
    "L0025", // unreachable code after infinite `while true` loop (no break)
    "L0026", // duplicate literal key in map literal (earlier binding shadowed)
    "L0027", // empty catch block silently discards the error
    "L0028", // negation of boolean literal (`!true` / `!false`) — use the literal directly
    "L0029", // comparison result discarded as statement — likely a typo for `=` or missed `assert`
    "L0030", // float equality comparison (`==` / `!=`) — almost always a bug; use an epsilon check
    "L0031", // double negation `!!x` is redundant — simplify to `x`
    "L0032", // assignment used as boolean condition — likely typo for `==`
    "L0033", // integer modulo by 1 is always 0 — likely wrong modulus
    "L0034", // string concatenation with `+` inside a loop is O(N²)
    "L0035", // unreachable code after diverging call (exit / abort)
    "L0036", // comparison of len(...) to negative literal — always true or false
    "L0037", // self-assignment `x = x` is a no-op
    "L0038", // RES-1863: panic!() call outside of #[cfg(test)] context
    "L0039", // RES-1863: unreachable code after call to @noreturn-annotated function
    "L0040", // RES-1863: magic number in safety-critical computation (unnamed non-trivial literal)
    "L0041", // redundant `else` block when the `if` arm always returns
    "L0042", // dead code after `return` statement in same block
    "L0043", // `let` binding shadows an existing binding with the same name
    "L0044", // shift amount is a literal outside 0..63 — always a runtime error
    "L0045", // constant-false condition in `while` loop — body never executes
    "L0046", // empty `for` loop body — iteration has no effect
    "L0047", // assert(true) is vacuously satisfied; assert(false) always panics
    "L0048", // bitwise XOR of a value with itself (x ^ x) is always 0 — likely a bug
    "L0049", // empty `if` then-branch — body was probably forgotten or condition is inverted
    "L0050", // redundant `else` after `if` that always breaks or continues the loop
    "L0051", // comparison of two string literals — always evaluates to a constant
    "L0052", // negation of a boolean literal in a condition (`!true` or `!false`)
    "L0053", // array index is a literal that is out of bounds for the literal array
    "L0054", // empty `while` loop body — the iteration has no effect
    "L0055", // redundant boolean `!=` check: `x != true` or `x != false`
    "L0056", // `for x in []` — iterating over an empty literal array, body never executes
    "L0057", // x + 0 or 0 + x — redundant addition of zero
    "L0058", // x - 0 — redundant subtraction of zero
    "L0059", // x * 1 or 1 * x — redundant multiplication by one
    "L0060", // x / 1 — redundant division by one
    "L0061", // x << 0 / x >> 0 — shift by zero is a no-op
    "L0062", // x < x / x > x always false; x <= x / x >= x always true
    "L0063", // dead code after `break` or `continue` in a loop body
    "L0064", // empty else block (else {}) can be removed
    "L0065", // `if cond { return true; } else { return false; }` simplifies to `return cond;`
    "L0066", // `if cond { return false; } else { return true; }` simplifies to `return !cond;`
    "L0067", // `x && true` / `true && x` — AND with true is the identity; simplify to `x`
    "L0068", // `x && false` / `false && x` — AND with false always yields false
    "L0069", // `x || true` / `true || x` — OR with true always yields true
    "L0070", // `x || false` / `false || x` — OR with false is the identity; simplify to `x`
    "L0071", // function has more than 5 parameters — consider grouping into a struct
    "L0072", // for-loop variable is never used in the loop body — dead iteration variable
    "L0073", // duplicate requires/ensures contract clause (same text repeated)
    "L0074", // pure function call result discarded as expression statement
    "L0075", // trivially-vacuous contract clause (requires true / requires false / ensures false)
    "L0076", // `result` identifier used in a `requires` clause (only valid in `ensures`)
    "L0077", // `ensures result` in a function without a declared return type
    "L0078", // function parameter name shadows a builtin function
    "L0079", // function body is empty (no statements)
    "L0080", // `let` binding is immediately overwritten before first use
    "L0081", // duplicate consecutive `assert` statements with the same condition
    "L0082", // both branches of `if/else` are empty blocks
    "L0083", // `@noreturn`-annotated function has a declared return type
    "L0084", // function defined inside another function body (nested function)
    "L0085", // struct declaration with zero fields
    "L0086", // string compared to empty literal `== ""` / `!= ""` — use `is_empty()`
    "L0087", // function declared `pure` contains a `print` or `println` call
    "L0088", // `let _ = expr` — wildcard discard binding; use bare `expr;` statement
    "L0089", // `exit()` or `abort()` call directly inside a `live { }` recovery block
    "L0090", // both arms of `if/else` return the same literal value — condition irrelevant
    "L0091", // for-range with equal start/end literal (0..0) — guaranteed empty
    "L0092", // `ensures false` in a function contract — claims function always diverges
    "L0093", // function parameter named `result` — shadows the postcondition pseudo-variable
    "L0094", // consecutive `break` or `continue` statements — second is unreachable
    "L0095", // `match` with a single wildcard arm (`_ => ...`) — prefer an expression
];

/// Return a human-readable explanation for a lint code, or `None` if unknown.
///
/// Used by `rz lint --explain LXXXX` and the `resilient_explain_lint` MCP tool.
pub fn explain(code: &str) -> Option<&'static str> {
    match code {
        "L0001" => Some(
            "L0001 — unused local binding\n\
             \n\
             A `let` binding is declared inside a function but its value is never read.\n\
             Dead bindings add noise and may indicate a logic error (e.g., a computation\n\
             result that was meant to be used).\n\
             \n\
             Example (bad):\n\
             \n\
             fn f() -> int {\n\
               let x = heavy_computation()   // x is never read\n\
               42\n\
             }\n\
             \n\
             Fix: remove the binding, or use it.\n\
             Suppress: // resilient: allow L0001",
        ),
        "L0002" => Some(
            "L0002 — unreachable match arm after wildcard\n\
             \n\
             A `match` arm pattern appears after a wildcard (`_`) that already catches\n\
             every possible value. The arm can never execute.\n\
             \n\
             Fix: reorder arms so the specific pattern comes before the wildcard.\n\
             Suppress: // resilient: allow L0002",
        ),
        "L0003" => Some(
            "L0003 — self-comparison (`x == x`)\n\
             \n\
             A value is compared against itself. The result is always `true` for `==`\n\
             and always `false` for `!=`. This is almost certainly a typo.\n\
             \n\
             Fix: replace one side with the intended value.\n\
             Suppress: // resilient: allow L0003",
        ),
        "L0004" => Some(
            "L0004 — mixing `&&` and `||` without parentheses\n\
             \n\
             An expression mixes `&&` and `||` at the same precedence level without\n\
             explicit parentheses. The evaluation order may surprise readers.\n\
             \n\
             Fix: add parentheses to make the intent explicit.\n\
             Suppress: // resilient: allow L0004",
        ),
        "L0005" => Some(
            "L0005 — redundant trailing `return;`\n\
             \n\
             A `return` with no value at the end of a void function is redundant;\n\
             the function returns implicitly.\n\
             \n\
             Fix: remove the trailing `return;`.\n\
             Suppress: // resilient: allow L0005",
        ),
        "L0006" => Some(
            "L0006 — `assume(false)` vacuously discharges all verification obligations\n\
             \n\
             `assume(false)` tells the Z3 verifier that the current path is unreachable.\n\
             Any `ensures` clause trivially holds because the premise is false. This\n\
             is dangerous in safety-critical code: it silently hides incomplete proofs.\n\
             \n\
             In `--safety-critical` mode this is promoted to a hard error.\n\
             \n\
             Fix: replace with a real proof or mark the path unreachable with a panic.\n\
             Suppress: // resilient: allow L0006  (not allowed in safety-critical mode)",
        ),
        "L0007" => Some(
            "L0007 — unreachable code after unconditional `return`\n\
             \n\
             Statements that follow an unconditional `return` in the same block can\n\
             never execute. They are dead code.\n\
             \n\
             Fix: remove the dead statements.\n\
             Suppress: // resilient: allow L0007",
        ),
        "L0008" => Some(
            "L0008 — duplicate identical struct literal match arm\n\
             \n\
             Two arms in a `match` have structurally identical struct literal patterns.\n\
             The second arm can never match because the first already does.\n\
             \n\
             Fix: remove or differentiate the duplicate arm.\n\
             Suppress: // resilient: allow L0008",
        ),
        "L0009" => Some(
            "L0009 — integer division by zero\n\
             \n\
             A division or modulo expression has a constant zero denominator, or the\n\
             Z3 verifier proved that zero is reachable. Division by zero is undefined\n\
             behaviour in Resilient.\n\
             \n\
             Fix: add a `requires y != 0` contract or guard the call site.\n\
             Suppress: // resilient: allow L0009",
        ),
        "L0010" => Some(
            "L0010 — function has no `requires`/`ensures` contract\n\
             \n\
             A non-trivial function does not declare any formal specification.\n\
             Without contracts the Z3 verifier cannot prove callers safe and cannot\n\
             catch implementation bugs at proof time.\n\
             \n\
             Example (bad):\n\
             fn div(int x, int y) -> int { x / y }\n\
             \n\
             Example (good):\n\
             fn div(int x, int y) -> int\n\
               requires y != 0\n\
               ensures result == x / y\n\
             { x / y }\n\
             \n\
             Suppress: // resilient: allow L0010",
        ),
        "L0011" => Some(
            "L0011 — unused variable (binding never read)\n\
             \n\
             A `let` binding is never referenced after its declaration. Unlike L0001\n\
             this fires on every unused binding including those at the top of a function\n\
             body, not just inner-block ones.\n\
             \n\
             Fix: prefix the name with `_` to signal intentional disuse, or remove it.\n\
             Suppress: // resilient: allow L0011",
        ),
        "L0012" => Some(
            "L0012 — spec annotation lacks `// source:` provenance comment\n\
             \n\
             A `@spec` annotation (or `requires`/`ensures` clause derived from an\n\
             external requirement) does not carry a `// source:` comment linking it to\n\
             the originating requirement document or ticket.\n\
             \n\
             In safety-critical codebases every formal property must be traceable to a\n\
             requirement. Without the provenance comment, audits cannot verify coverage.\n\
             \n\
             Fix: add `// source: REQ-NNN` above the annotation.\n\
             Suppress: // resilient: allow L0012",
        ),
        "L0013" => Some(
            "L0013 — unchecked array indexing (not proven in-bounds)\n\
             \n\
             An array index expression `a[i]` could be out-of-bounds at runtime and\n\
             the verifier cannot prove otherwise from the surrounding contracts.\n\
             \n\
             Fix: add a `requires i < len(a) && i >= 0` contract, or use a bounds-safe\n\
             accessor.\n\
             Suppress: // resilient: allow L0013",
        ),
        "L0014" => Some(
            "L0014 — function defined but never called (dead function)\n\
             \n\
             A top-level function is defined but no call site references it in the\n\
             same source file. It may be dead code, or it may be an entry point that\n\
             should be exported.\n\
             \n\
             Fix: call it, delete it, or mark it as an entry point with `@export`.\n\
             Suppress: // resilient: allow L0014",
        ),
        "L0015" => Some(
            "L0015 — constant arithmetic expression overflows `int`\n\
             \n\
             A compile-time-evaluable arithmetic expression produces a value outside\n\
             the range of the `int` type (−2^63 … 2^63−1 on 64-bit targets).\n\
             \n\
             Fix: use a smaller constant, or widen the type.\n\
             Suppress: // resilient: allow L0015",
        ),
        "L0016" => Some(
            "L0016 — constant boolean condition in `if`\n\
             \n\
             An `if` condition evaluates to a compile-time constant (`true` or `false`).\n\
             One branch is dead code; the whole `if` can be simplified.\n\
             \n\
             Fix: remove the dead branch.\n\
             Suppress: // resilient: allow L0016",
        ),
        "L0017" => Some(
            "L0017 — binding shadows an outer binding of the same name\n\
             \n\
             A `let` declaration inside a nested scope has the same name as an outer\n\
             binding. The outer value is inaccessible in the inner scope, which can\n\
             hide bugs.\n\
             \n\
             Fix: rename one of the bindings.\n\
             Suppress: // resilient: allow L0017",
        ),
        "L0018" => Some(
            "L0018 — function with return type may not return on all paths\n\
             \n\
             A function declares a non-void return type but contains a control-flow\n\
             path that reaches the end of the body without a `return` statement.\n\
             \n\
             Fix: add a `return` on every path, or add a `panic`/`unreachable` sentinel.\n\
             Suppress: // resilient: allow L0018",
        ),
        "L0019" => Some(
            "L0019 — `format()` argument count mismatch\n\
             \n\
             The number of `{}` placeholders in the format string does not match the\n\
             number of extra arguments passed to `format()`.\n\
             \n\
             Example (bad):  format(\"{} {}\", x)       // 2 placeholders, 1 arg\n\
             Example (good): format(\"{} {}\", x, y)    // 2 placeholders, 2 args\n\
             \n\
             Suppress: // resilient: allow L0019",
        ),
        "L0020" => Some(
            "L0020 — function parameter is never used in the body\n\
             \n\
             A parameter is declared but never referenced in the function body.\n\
             This may indicate a forgotten computation or a stale signature.\n\
             \n\
             Fix: remove the parameter, or use it.\n\
             Suppress: // resilient: allow L0020",
        ),
        "L0021" => Some(
            "L0021 — redundant boolean sub-expression (`x && x`, `x || x`)\n\
             \n\
             A boolean expression ANDs or ORs a sub-expression with itself. The result\n\
             is always equal to the sub-expression alone.\n\
             \n\
             Fix: remove one copy of the sub-expression.\n\
             Suppress: // resilient: allow L0021",
        ),
        "L0022" => Some(
            "L0022 — else branch after unconditional return is redundant\n\
             \n\
             The `if` branch ends with an unconditional `return`, so the `else` block\n\
             is always entered when the `if` condition is false. The `else` can be\n\
             flattened (removed) without changing semantics.\n\
             \n\
             Fix: remove the `else` and de-indent its body.\n\
             Suppress: // resilient: allow L0022",
        ),
        "L0023" => Some(
            "L0023 — tautological comparison with boolean literal\n\
             \n\
             An expression compares a value against `true` or `false` with `==`/`!=`.\n\
             The comparison is redundant: `x == true` is identical to `x`, and\n\
             `x == false` is identical to `!x`.\n\
             \n\
             Fix: use the value directly.\n\
             Suppress: // resilient: allow L0023",
        ),
        "L0024" => Some(
            "L0024 — struct literal missing required fields\n\
             \n\
             A struct literal does not provide values for one or more fields declared\n\
             in the struct definition. The omitted fields will be zero-initialised,\n\
             which may silently produce incorrect state.\n\
             \n\
             Fix: provide explicit values for all fields.\n\
             Suppress: // resilient: allow L0024",
        ),
        "L0025" => Some(
            "L0025 — unreachable code after infinite `while true` loop\n\
             \n\
             Statements following a `while true { … }` that contains no `break` can\n\
             never execute. The loop never terminates, so control never reaches the\n\
             code after it.\n\
             \n\
             Fix: add a `break` inside the loop, or remove the dead code.\n\
             Suppress: // resilient: allow L0025",
        ),
        "L0026" => Some(
            "L0026 — duplicate literal key in map literal\n\
             \n\
             A map literal (`{ key: val, key: val2 }`) contains two entries with the\n\
             same key. The earlier entry is silently shadowed by the later one.\n\
             \n\
             Fix: remove or rename the duplicate key.\n\
             Suppress: // resilient: allow L0026",
        ),
        "L0027" => Some(
            "L0027 — empty `catch` block silently discards the error\n\
             \n\
             A `try`/`catch` block has an empty catch body. The caught error is\n\
             swallowed without logging, handling, or re-throwing. This is a common\n\
             source of hard-to-debug failures.\n\
             \n\
             Fix: log the error, handle it, or re-throw it.\n\
             Suppress: // resilient: allow L0027",
        ),
        "L0028" => Some(
            "L0028 — negation of boolean literal (`!true` / `!false`)\n\
             \n\
             `!true` evaluates to `false` and `!false` evaluates to `true`. Use the\n\
             resulting literal directly instead of negating the other.\n\
             \n\
             Fix: replace `!true` → `false`, `!false` → `true`.\n\
             Suppress: // resilient: allow L0028",
        ),
        "L0029" => Some(
            "L0029 — comparison result discarded as statement\n\
             \n\
             A comparison expression (e.g. `x == y`) is used as a standalone statement.\n\
             Its boolean result is immediately thrown away. This is almost always a\n\
             typo — either `=` (assignment) was intended, or the result should be\n\
             passed to `assert(…)`.\n\
             \n\
             Fix: use `assert(x == y)` or turn it into an assignment.\n\
             Suppress: // resilient: allow L0029",
        ),
        "L0030" => Some(
            "L0030 — float equality comparison (`==` / `!=`)\n\
             \n\
             Comparing floating-point values with `==` or `!=` is almost always a bug\n\
             due to rounding errors accumulated during computation.\n\
             \n\
             Example (bad):  if a == b { … }\n\
             Example (good): if abs(a - b) < 1e-9 { … }\n\
             \n\
             Fix: use an epsilon-based comparison instead.\n\
             Suppress: // resilient: allow L0030",
        ),
        "L0031" => Some(
            "L0031 — double negation (`!!x`) is redundant\n\
             \n\
             Applying `!` twice cancels out: `!!x` is always equal to `x`.\n\
             \n\
             Fix: simplify to `x`.\n\
             Suppress: // resilient: allow L0031",
        ),
        "L0032" => Some(
            "L0032 — assignment used as boolean condition\n\
             \n\
             The condition of an `if` or `while` statement is an assignment expression\n\
             (`x = value`). Assignments return the assigned value, so this compiles,\n\
             but it is almost certainly a typo for `==` (equality test).\n\
             \n\
             Fix: replace `=` with `==` if equality was intended.\n\
             Suppress: // resilient: allow L0032",
        ),
        "L0033" => Some(
            "L0033 — integer modulo by 1 is always 0\n\
             \n\
             `x % 1` is always `0` for any integer `x`. A modulus of 1 is almost\n\
             certainly a wrong constant — the likely intent is `% 2` (even/odd test)\n\
             or some other power of two.\n\
             \n\
             Fix: use the correct modulus.\n\
             Suppress: // resilient: allow L0033",
        ),
        "L0034" => Some(
            "L0034 — string concatenation with `+` inside a loop is O(N²)\n\
             \n\
             Building a string by repeated `+` concatenation inside a loop copies\n\
             the accumulated string on every iteration, resulting in O(N²) total\n\
             work for N iterations.\n\
             \n\
             Fix: collect parts into a list and join at the end, or use a string\n\
             builder if available.\n\
             Suppress: // resilient: allow L0034",
        ),
        "L0035" => Some(
            "L0035 — unreachable code after diverging call\n\
             \n\
             A call to a diverging function (`exit()` or `abort()`) is followed by\n\
             additional statements in the same block. Those statements can never\n\
             execute because the diverging call never returns.\n\
             \n\
             Fix: remove the dead code after the diverging call.\n\
             Suppress: // resilient: allow L0035",
        ),
        "L0036" => Some(
            "L0036 — comparison of `len(...)` to negative literal\n\
             \n\
             `len()` always returns a non-negative integer (≥ 0). Comparing it to a\n\
             negative literal with `<`, `<=`, `==`, or `!=` produces a result that\n\
             is always the same (`false` for `< -1`, etc.), making the check dead.\n\
             \n\
             Example (bad):  if len(arr) < 0 { ... }  // always false\n\
             Example (bad):  if len(arr) <= -1 { ... } // always false\n\
             \n\
             Fix: remove the dead branch or fix the comparison.\n\
             Suppress: // resilient: allow L0036",
        ),
        "L0037" => Some(
            "L0037 — self-assignment `x = x` is a no-op\n\
             \n\
             Assigning a variable to itself has no effect. This is almost certainly\n\
             a typo — the right-hand side was meant to be a different expression.\n\
             \n\
             Example (bad):  x = x  // does nothing\n\
             \n\
             Fix: replace the right-hand side with the intended value.\n\
             Suppress: // resilient: allow L0037",
        ),
        "L0038" => Some(
            "L0038 — `panic!()` call outside of test context\n\
             \n\
             A call to `panic()` was found in non-test code. In safety-critical embedded\n\
             systems, panics are forbidden outside of test scaffolding because they produce\n\
             uncontrolled program termination.\n\
             \n\
             Fix: replace with a proper error return or use `abort()` with a diagnostic.\n\
             Suppress: // resilient: allow L0038",
        ),
        "L0039" => Some(
            "L0039 — unreachable code after `@noreturn` call\n\
             \n\
             A call to a function annotated with `// @noreturn` is followed by additional\n\
             statements in the same block. Those statements can never execute because\n\
             the annotated function never returns.\n\
             \n\
             Fix: remove the dead statements after the diverging call.\n\
             Suppress: // resilient: allow L0039",
        ),
        "L0040" => Some(
            "L0040 — magic number in safety-critical computation\n\
             \n\
             An unnamed integer literal (other than 0, 1, or powers of two) appears in\n\
             an arithmetic expression inside a function that has no `requires` or `ensures`\n\
             contract. In safety-critical embedded code, numeric constants must be named\n\
             via `let` bindings so their intent is auditable.\n\
             \n\
             Fix: extract the literal into a named constant (`let THRESHOLD = 42;`).\n\
             Suppress: // resilient: allow L0040",
        ),
        "L0041" => Some(
            "L0041 — redundant `else` after `return`\n\
             \n\
             An `if` branch ends with a `return` statement, making the following `else`\n\
             block structurally redundant — its body is only reached when the condition\n\
             is false, and the early return already ensures that. The `else` can be\n\
             removed and the else-body dedented to improve readability.\n\
             \n\
             Fix: remove the `else { ... }` wrapper and dedent the else-body.\n\
             Suppress: // resilient: allow L0041",
        ),
        "L0042" => Some(
            "L0042 — dead code after `return`\n\
             \n\
             A `return` statement is followed by one or more statements in the same\n\
             block. Those statements can never be executed. This often indicates a\n\
             logic error (missing `if` condition) or copy-paste residue.\n\
             \n\
             Fix: remove the dead statements, or wrap them in a conditional.\n\
             Suppress: // resilient: allow L0042",
        ),
        "L0043" => Some(
            "L0043 — shadowed binding\n\
             \n\
             A `let` binding introduces a name that already exists in the same scope\n\
             (either a function parameter or a prior `let` in the same block). The\n\
             new binding silently hides the original, which can make code difficult to\n\
             reason about and is a frequent source of unintended aliasing bugs.\n\
             \n\
             Fix: rename one of the bindings.\n\
             Suppress: // resilient: allow L0043",
        ),
        "L0044" => Some(
            "L0044 — shift amount out of range\n\
             \n\
             A bitwise shift (`<<` or `>>`) uses an integer literal shift amount that\n\
             is outside the valid range 0..63. Any shift amount less than 0 or\n\
             greater than 63 is a guaranteed runtime error in Resilient (the VM and\n\
             interpreter both reject it with `shift amount out of range`).\n\
             \n\
             In safety-critical embedded code this is always a bug — shifting by a\n\
             constant out-of-range value produces no useful result.\n\
             \n\
             Fix: use a shift amount in 0..63.\n\
             Suppress: // resilient: allow L0044",
        ),
        "L0045" => Some(
            "L0045 — `while` loop with constant-false condition\n\
             \n\
             The loop condition is statically `false`, so the body never executes.\n\
             This is dead code: the loop could be removed entirely without changing\n\
             the program's behavior.\n\
             \n\
             In safety-critical embedded code this is almost always a mistake — either\n\
             the condition is wrong, or the loop should have been an `if` statement.\n\
             \n\
             Fix: remove the dead loop or correct the condition.\n\
             Suppress: // resilient: allow L0045",
        ),
        "L0046" => Some(
            "L0046 — `for` loop with empty body\n\
             \n\
             A `for`-in loop with an empty body iterates over the collection but\n\
             performs no work — no bindings, no side effects. The iteration is dead\n\
             code and can be removed.\n\
             \n\
             In safety-critical embedded code silent no-ops around loop scaffolding\n\
             are a reliability hazard — the logic may have been accidentally deleted.\n\
             \n\
             Fix: add the missing loop body, or remove the loop.\n\
             Suppress: // resilient: allow L0046",
        ),
        "L0047" => Some(
            "L0047 — vacuous or always-failing `assert`\n\
             \n\
             The assertion condition is the literal `true` (always satisfied — provides\n\
             no safety guarantee) or the literal `false` (always fails — unconditional\n\
             panic at runtime).\n\
             \n\
             `assert(true)` is a no-op and should be removed. `assert(false)` will\n\
             always halt the program; if this is intentional, use a named `abort()` or\n\
             add a comment explaining the invariant that was violated.\n\
             \n\
             Fix: supply a meaningful runtime-checkable predicate, or remove the assert.\n\
             Suppress: // resilient: allow L0047",
        ),
        "L0048" => Some(
            "L0048 — bitwise XOR of a value with itself (`x ^ x`)\n\
             \n\
             XOR-ing any integer with itself always produces 0: `x ^ x == 0` for all `x`.\n\
             This is almost certainly a copy-paste error — the two operands were meant to\n\
             be different values. In assembly, `xor reg, reg` is used to zero a register,\n\
             but at the source level `x ^ x` is confusing and should be written as `0`\n\
             if zeroing was the intent.\n\
             \n\
             Fix: replace one operand with the intended value, or use `0` directly.\n\
             Suppress: // resilient: allow L0048",
        ),
        "L0049" => Some(
            "L0049 — empty `if` then-branch\n\
             \n\
             The body of the `if` statement is an empty block `{ }`. This is almost\n\
             always either:\n\
             (a) a forgotten body — the statements that belong here were not written, or\n\
             (b) an inverted condition — the else branch was intended to be the then\n\
                 branch; negate the condition to fix.\n\
             \n\
             Example (bad):   if error { } else { do_work(); }\n\
             Example (fixed): if !error { do_work(); }\n\
             \n\
             Fix: add the missing body, or invert the condition and drop the else.\n\
             Suppress: // resilient: allow L0049",
        ),
        "L0050" => Some(
            "L0050 — redundant `else` after `if` that always `break`s or `continue`s\n\
             \n\
             When the `if` body always exits the current loop iteration via `break` or\n\
             `continue`, the `else` block is never reached from the `if` path. The else\n\
             body is dead under the if-is-true case and can be de-nested to the same\n\
             scope as the if statement.\n\
             \n\
             Example (bad):\n\
               for x in arr {\n\
                 if x < 0 { break; } else { process(x); }\n\
               }\n\
             Example (good):\n\
               for x in arr {\n\
                 if x < 0 { break; }\n\
                 process(x);\n\
               }\n\
             \n\
             Fix: remove the `else` keyword and de-nest the body.\n\
             Suppress: // resilient: allow L0050",
        ),
        "L0051" => Some(
            "L0051 — comparison of two string literals\n\
             \n\
             Comparing two string literals with `==` or `!=` always evaluates to a\n\
             compile-time constant (`true` or `false`). This is almost always a bug —\n\
             either one side should be a variable, or the check is redundant.\n\
             \n\
             Example (bad):\n\
               if mode == \"debug\" == \"release\" { ... }  // always false\n\
             Example (bad):\n\
               if \"hello\" == \"hello\" { ... }  // always true\n\
             \n\
             Fix: replace one operand with the variable you intended to compare.\n\
             Suppress: // resilient: allow L0051",
        ),
        "L0052" => Some(
            "L0052 — negation of a boolean literal\n\
             \n\
             `!true` evaluates to `false` and `!false` evaluates to `true`. Using\n\
             negation of a boolean literal in a condition is always a bug or a\n\
             readability problem — replace with the simplified literal directly.\n\
             \n\
             Example (bad): if !true { ... }   // body is unreachable\n\
             Example (bad): if !false { ... }  // condition is always true\n\
             Example (good): if false { ... }  // or remove the check entirely\n\
             \n\
             Fix: replace `!true` with `false` and `!false` with `true`.\n\
             Suppress: // resilient: allow L0052",
        ),
        "L0053" => Some(
            "L0053 — array index literal is out of bounds\n\
             \n\
             When an array literal (`[a, b, c]`) is indexed with an integer literal\n\
             and that index is >= the array length or < 0, the access will always\n\
             panic at runtime. Catch this at lint time before the program runs.\n\
             \n\
             Example (bad): [1, 2, 3][5]  // index 5 >= length 3\n\
             Example (bad): [1, 2, 3][-1] // negative index\n\
             \n\
             Fix: use a valid index, or store the array in a variable and guard\n\
             the index with a bounds check.\n\
             Suppress: // resilient: allow L0053",
        ),
        "L0054" => Some(
            "L0054 — empty `while` loop body\n\
             \n\
             A `while` loop with an empty body (`while cond {}`) iterates until\n\
             the condition becomes false but performs no work. This is either a\n\
             placeholder left in by mistake or a busy-wait loop that should be\n\
             replaced with a proper sleep/yield mechanism.\n\
             \n\
             Example (bad): while !ready() {}\n\
             \n\
             Fix: add the intended body, or replace with an appropriate yield\n\
             mechanism.\n\
             Suppress: // resilient: allow L0054",
        ),
        "L0055" => Some(
            "L0055 — redundant boolean `!=` check\n\
             \n\
             Comparing a boolean expression with `!=` against the literal `true`\n\
             or `false` is redundant. The negated form is always clearer.\n\
             \n\
             Example (bad): if x != true { ... }   // same as: if !x { ... }\n\
             Example (bad): if x != false { ... }  // same as: if x { ... }\n\
             \n\
             Fix: replace `x != true` with `!x` and `x != false` with `x`.\n\
             Suppress: // resilient: allow L0055",
        ),
        "L0056" => Some(
            "L0056 — `for x in []` — iterating over an empty literal array\n\
             \n\
             When a `for`-in loop iterates over an empty array literal `[]`,\n\
             the loop body is never executed. This is almost certainly a mistake:\n\
             either the array was meant to contain elements, or the loop is dead.\n\
             \n\
             Example (bad): for x in [] { process(x); }\n\
             \n\
             Fix: populate the array, or remove the loop.\n\
             Suppress: // resilient: allow L0056",
        ),
        "L0057" => Some(
            "L0057 — redundant addition of zero (`x + 0` / `0 + x`)\n\
             \n\
             Adding zero to a value is a no-op. The expression simplifies to `x`.\n\
             \n\
             Fix: remove the `+ 0` / `0 +` operand.\n\
             Suppress: // resilient: allow L0057",
        ),
        "L0058" => Some(
            "L0058 — redundant subtraction of zero (`x - 0`)\n\
             \n\
             Subtracting zero from a value is a no-op. The expression simplifies to `x`.\n\
             \n\
             Fix: remove the `- 0` operand.\n\
             Suppress: // resilient: allow L0058",
        ),
        "L0059" => Some(
            "L0059 — redundant multiplication by one (`x * 1` / `1 * x`)\n\
             \n\
             Multiplying a value by one is a no-op. The expression simplifies to `x`.\n\
             \n\
             Fix: remove the `* 1` / `1 *` operand.\n\
             Suppress: // resilient: allow L0059",
        ),
        "L0060" => Some(
            "L0060 — redundant division by one (`x / 1`)\n\
             \n\
             Dividing a value by one is a no-op. The expression simplifies to `x`.\n\
             \n\
             Fix: remove the `/ 1` operand.\n\
             Suppress: // resilient: allow L0060",
        ),
        "L0061" => Some(
            "L0061 — shift by zero is a no-op (`x << 0` / `x >> 0`)\n\
             \n\
             Shifting a value by zero bits leaves it unchanged. The expression simplifies to `x`.\n\
             \n\
             Fix: remove the shift or use the intended shift amount.\n\
             Suppress: // resilient: allow L0061",
        ),
        "L0062" => Some(
            "L0062 — tautological inequality comparison with self\n\
             \n\
             Comparing a value to itself with `<`, `>`, `<=`, or `>=` always produces\n\
             a constant result: `x < x` and `x > x` are always `false`; `x <= x` and\n\
             `x >= x` are always `true`. Extends L0003 (which covers `==`/`!=`) to\n\
             the inequality operators.\n\
             \n\
             Fix: replace one operand with the intended value.\n\
             Suppress: // resilient: allow L0062",
        ),
        "L0063" => Some(
            "L0063 — dead code after `break` or `continue` statement\n\
             \n\
             Statements following a `break` or `continue` in the same block are never\n\
             executed. The loop iteration ends at the `break`/`continue` and control\n\
             jumps to the next iteration or past the loop body immediately.\n\
             \n\
             Fix: remove or relocate the unreachable statements.\n\
             Suppress: // resilient: allow L0063",
        ),
        "L0064" => Some(
            "L0064 — empty `else {}` block can be removed\n\
             \n\
             An `else` clause whose body is an empty block `{}` has no effect on\n\
             program behaviour. It adds visual noise without any semantic content.\n\
             \n\
             Fix: remove the `else {}` clause entirely.\n\
             Suppress: // resilient: allow L0064",
        ),
        "L0065" => Some(
            "L0065 — `if cond { return true; } else { return false; }` simplifies to `return cond;`\n\
             \n\
             Returning a boolean literal that matches the condition value directly is\n\
             redundant. The condition expression itself already evaluates to the same\n\
             boolean, so wrapping it in an `if`/`else` is unnecessary.\n\
             \n\
             Fix: replace with `return cond;`.\n\
             Suppress: // resilient: allow L0065",
        ),
        "L0066" => Some(
            "L0066 — `if cond { return false; } else { return true; }` simplifies to `return !cond;`\n\
             \n\
             Returning the boolean negation of the condition via an `if`/`else` is\n\
             redundant. The negated condition expression already evaluates to the same\n\
             boolean.\n\
             \n\
             Fix: replace with `return !cond;`.\n\
             Suppress: // resilient: allow L0066",
        ),
        "L0067" => Some(
            "L0067 — `x && true` / `true && x` — AND with `true` is the identity\n\
             \n\
             In boolean algebra, `x && true` always equals `x`. The `&& true` operand\n\
             is redundant and can be removed. This often indicates a leftover from\n\
             refactoring or a misunderstood guard condition.\n\
             \n\
             Fix: replace `x && true` with `x`.\n\
             Suppress: // resilient: allow L0067",
        ),
        "L0068" => Some(
            "L0068 — `x && false` / `false && x` — AND with `false` is always false\n\
             \n\
             In boolean algebra, `x && false` always evaluates to `false` regardless\n\
             of `x`. The `x &&` part is dead code (short-circuit may even skip it).\n\
             This is almost certainly a logic error.\n\
             \n\
             Fix: remove the dead operand or fix the condition logic.\n\
             Suppress: // resilient: allow L0068",
        ),
        "L0069" => Some(
            "L0069 — `x || true` / `true || x` — OR with `true` is always true\n\
             \n\
             In boolean algebra, `x || true` always evaluates to `true` regardless\n\
             of `x`. The `x ||` part is dead code (short-circuit may skip it entirely).\n\
             This is almost certainly a logic error or leftover guard.\n\
             \n\
             Fix: remove the dead operand or replace the expression with `true`.\n\
             Suppress: // resilient: allow L0069",
        ),
        "L0070" => Some(
            "L0070 — `x || false` / `false || x` — OR with `false` is the identity\n\
             \n\
             In boolean algebra, `x || false` always equals `x`. The `|| false` operand\n\
             is redundant and can be removed. This often indicates a leftover from\n\
             refactoring or an unnecessary guard.\n\
             \n\
             Fix: replace `x || false` with `x`.\n\
             Suppress: // resilient: allow L0070",
        ),
        "L0071" => Some(
            "L0071 — function has more than 5 parameters\n\
             \n\
             Functions with many parameters are harder to call correctly, harder to read,\n\
             and easier to misorder. In safety-critical code, argument confusion is a\n\
             common source of bugs.\n\
             \n\
             Fix: group related parameters into a struct and pass the struct instead.\n\
             Suppress: // resilient: allow L0071",
        ),
        "L0072" => Some(
            "L0072 — for-loop variable is never used in the body\n\
             \n\
             A `for x in collection { ... }` loop declares the iteration variable `x`\n\
             but never reads it inside the body. The variable name is dead, and the\n\
             intent is unclear. This often indicates a copy-paste error or a missing use.\n\
             \n\
             Example (bad): for x in items { total = total + 1; }  // x unused\n\
             \n\
             Fix: use `x` in the body, or rename it to `_` to signal intentional discard.\n\
             Suppress: // resilient: allow L0072",
        ),
        "L0073" => Some(
            "L0073 — duplicate contract clause\n\
             \n\
             The same `requires` or `ensures` clause text appears more than once in the\n\
             same function declaration. The duplicate adds no information and is almost\n\
             always a copy-paste error.\n\
             \n\
             Example (bad): fn f(int x) requires x > 0 requires x > 0 { ... }\n\
             \n\
             Fix: remove the duplicate clause.\n\
             Suppress: // resilient: allow L0073",
        ),
        "L0074" => Some(
            "L0074 — pure function call result discarded\n\
             \n\
             A function declared `pure` (no side effects) is called as a standalone\n\
             expression statement, meaning its return value is thrown away. Since the\n\
             function has no observable side effects, the call has no effect at all.\n\
             This is almost always a logic error — the programmer likely forgot to\n\
             use the return value.\n\
             \n\
             Example (bad): pure fn square(int x) -> int { return x * x; }\n\
                            fn f() { square(5); }  // result never used\n\
             \n\
             Fix: assign the result to a variable, or remove the call if it was accidental.\n\
             Suppress: // resilient: allow L0074",
        ),
        "L0075" => Some(
            "L0075 — trivially-vacuous contract clause\n\
             \n\
             A `requires` or `ensures` clause is a boolean literal:\n\
             * `requires true` — vacuous precondition; every call satisfies it and the\n\
               clause provides no safety guarantee. Remove it or write a real constraint.\n\
             * `requires false` — unsatisfiable precondition; the function can never be\n\
               called legally. This is almost always wrong.\n\
             * `ensures false` — impossible postcondition; the function can never satisfy\n\
               its own specification. Almost always wrong.\n\
             * `ensures true` — vacuous postcondition; every return value satisfies it.\n\
               Remove it or write a real constraint.\n\
             \n\
             Fix: replace with a meaningful contract, or remove the clause.\n\
             Suppress: // resilient: allow L0075",
        ),
        "L0076" => Some(
            "L0076 — `result` in `requires` clause\n\
             `result` refers to the return value of a function and is only in scope inside\n\
             `ensures` clauses. Using it in `requires` is almost certainly a typo — move\n\
             the condition to an `ensures` clause.\n\
             Suppress: // resilient: allow L0076",
        ),
        "L0077" => Some(
            "L0077 — `ensures result` on a void function\n\
             The special identifier `result` refers to a function's return value. A function\n\
             with no declared return type has no `result` to constrain.\n\
             Suppress: // resilient: allow L0077",
        ),
        "L0078" => Some(
            "L0078 — parameter name shadows builtin\n\
             Choosing a parameter name that matches a builtin function name hides the builtin\n\
             inside the function body. Rename the parameter to avoid confusion.\n\
             Suppress: // resilient: allow L0078",
        ),
        "L0079" => Some(
            "L0079 — empty function body\n\
             The function contains no statements. If this is intentional (a stub or no-op),\n\
             add a comment explaining why. Otherwise add the missing implementation.\n\
             Suppress: // resilient: allow L0079",
        ),
        "L0080" => Some(
            "L0080 — initial `let` value overwritten before use\n\
             The binding is assigned in the `let` declaration and then immediately overwritten\n\
             on the next line before the original value is read. The initializer is dead code.\n\
             Suppress: // resilient: allow L0080",
        ),
        "L0081" => Some(
            "L0081 — duplicate consecutive `assert`\n\
             The same condition is asserted twice in a row. The second assertion is redundant\n\
             because the first already guarantees it holds (or aborts if it doesn't).\n\
             Suppress: // resilient: allow L0081",
        ),
        "L0082" => Some(
            "L0082 — both `if/else` branches are empty\n\
             Both the `if` and `else` blocks contain no statements. The entire `if/else`\n\
             expression has no effect and can be removed.\n\
             Suppress: // resilient: allow L0082",
        ),
        "L0083" => Some(
            "L0083 — `@noreturn` function with return type\n\
             A function annotated with `// @noreturn` never returns to its caller, so a\n\
             return-type annotation is a contradiction. Remove the return type or the\n\
             `@noreturn` annotation.\n\
             Suppress: // resilient: allow L0083",
        ),
        "L0084" => Some(
            "L0084 — nested function definition\n\
             Defining a function inside another function body is unusual and can make the\n\
             code harder to follow. Consider hoisting the inner function to the top level.\n\
             Suppress: // resilient: allow L0084",
        ),
        "L0085" => Some(
            "L0085 — struct with no fields\n\
             A struct with zero fields is usually a placeholder. If it's intentional,\n\
             add a comment. Otherwise add the missing fields.\n\
             Suppress: // resilient: allow L0085",
        ),
        "L0086" => Some(
            "L0086 — string compared to empty literal\n\
             \n\
             `s == \"\"` / `s != \"\"` compares against the empty string literal. \
             Prefer `is_empty(s)` / `!is_empty(s)` for clarity.\n\
             Suppress: // resilient: allow L0086",
        ),
        "L0087" => Some(
            "L0087 — pure function calls print/println\n\
             \n\
             A `pure` function must not produce I/O side effects. Remove the\n\
             `print`/`println` call or remove the `pure` annotation.\n\
             Suppress: // resilient: allow L0087",
        ),
        "L0088" => Some(
            "L0088 — wildcard let discard binding\n\
             \n\
             `let _ = expr;` is clearer written as a bare `expr;` statement.\n\
             Suppress: // resilient: allow L0088",
        ),
        "L0089" => Some(
            "L0089 — exit/abort inside a live recovery block\n\
             \n\
             Calling `exit()` or `abort()` inside a `live { }` block defeats \
             recovery. Use a controlled error path instead.\n\
             Suppress: // resilient: allow L0089",
        ),
        "L0090" => Some(
            "L0090 — both if/else arms return the same value\n\
             \n\
             Both branches return the same literal; the condition is irrelevant. \
             Simplify to `return <value>;` or fix the branch.\n\
             Suppress: // resilient: allow L0090",
        ),
        "L0091" => Some(
            "L0091 — for-range with equal start and end (empty)\n\
             \n\
             `for i in N..N` never executes its body. Check the range bounds.\n\
             Suppress: // resilient: allow L0091",
        ),
        "L0092" => Some(
            "L0092 — `ensures false` claims function always diverges\n\
             \n\
             A contract `ensures false` means the verifier treats every return\n\
             as unreachable. Unless the function truly never returns, this is a\n\
             bug in the contract. Use `// @noreturn` for intentional divergence.\n\
             Suppress: // resilient: allow L0092",
        ),
        "L0093" => Some(
            "L0093 — parameter named `result`\n\
             \n\
             The name `result` is the postcondition pseudo-variable in `ensures`\n\
             clauses. A parameter of the same name will shadow it, producing\n\
             incorrect verification. Rename the parameter.\n\
             Suppress: // resilient: allow L0093",
        ),
        "L0094" => Some(
            "L0094 — consecutive break/continue (second unreachable)\n\
             \n\
             The second `break`/`continue` in a sequence is dead code. Remove it.\n\
             Suppress: // resilient: allow L0094",
        ),
        "L0095" => Some(
            "L0095 — match with single wildcard arm\n\
             \n\
             A `match` with one `_ => body` arm is equivalent to `body` directly.\n\
             Remove the `match` or add meaningful patterns.\n\
             Suppress: // resilient: allow L0095",
        ),
        _ => None,
    }
}

/// RES-778: process-wide policy switch for safety-critical CLI mode.
///
/// When enabled, `assume(false)` (L0006) is promoted from a warning to
/// a hard error and cannot be silenced by a local allow-comment.
static SAFETY_CRITICAL_MODE: AtomicBool = AtomicBool::new(false);

/// Enable or disable safety-critical lint policy for the current
/// process. Mirrors the atomic flag pattern already used by other
/// strict CLI modes in the compiler driver.
pub fn set_safety_critical_mode(on: bool) {
    SAFETY_CRITICAL_MODE.store(on, Ordering::Relaxed);
}

/// Returns true when safety-critical lint policy is active.
pub fn safety_critical_mode() -> bool {
    SAFETY_CRITICAL_MODE.load(Ordering::Relaxed)
}

/// RES-1376: cached trigger-presence flags for the lint passes.
/// Built by `scan_lint_triggers` in one AST visit; each lint pass is
/// gated on the flag for its trigger node so passes whose trigger
/// never appears in the program don't pay for a full AST walk.
#[derive(Default)]
struct LintTriggers {
    has_assume: bool,
    has_index: bool,
    has_match: bool,
    has_division: bool,
    has_infix: bool,
    has_function: bool,
    has_let: bool,
    has_block: bool,
    has_call: bool,
    has_integer_literal: bool,
    has_if_statement: bool,
    has_let_in_nested_block: bool,
    has_format_call: bool,
    has_if_with_else: bool,
    has_bool_literal: bool,
    has_while_true: bool,
    has_struct_literal: bool,
    has_map_literal: bool,
    has_try_catch: bool,
    has_prefix_expr: bool,
    has_float_literal: bool,
    has_expr_stmt_cmp: bool,
    has_assign_in_cond: bool,
    has_string_literal: bool,
    has_loop: bool,
    has_assignment: bool,
    /// L0038: a call to `panic()` was found anywhere in the program.
    has_panic_call: bool,
    /// L0039: a function annotated `// @noreturn` was referenced via call.
    has_noreturn_call: bool,
    /// L0040: an arithmetic infix expression with an integer literal exists.
    has_arith_int_literal: bool,
    /// L0041/L0042: any IfStatement with an alternative or any ReturnStatement in a block.
    has_return_in_block: bool,
    /// L0043: any LetStatement present (potential shadowing).
    has_let_binding: bool,
    /// L0044: any bitwise shift (`<<` / `>>`) with an integer literal as the RHS.
    has_literal_shift: bool,
    /// L0045: any `while` statement exists (checked against constant-false condition).
    has_while_stmt: bool,
    /// L0046: any `for`-in statement exists (checked for empty body).
    has_for_in_stmt: bool,
    /// L0047: any `assert` statement exists (checked for literal condition).
    has_assert_stmt: bool,
    /// L0048: any `^` (Bxor) infix expression exists.
    has_xor_infix: bool,
    // L0049 reuses has_if_statement; L0050 reuses has_if_with_else — no new fields needed.
    /// L0051: any `==` / `!=` infix on two string literals.
    has_string_literal_cmp: bool,
    /// L0052: any prefix `!` applied to a boolean literal.
    has_negated_bool_literal: bool,
    /// L0053: any index expression whose object is an array literal.
    has_array_literal_index: bool,
    /// L0055: any `!=` infix where one operand is a boolean literal.
    has_bool_neq_cmp: bool,
    /// L0057–L0061: infix expression with an integer literal that is 0 or 1 as an operand.
    has_arith_identity: bool,
    /// L0063: any `break` or `continue` statement found in the program.
    has_break_continue: bool,
    /// L0067–L0070: any `&&` / `||` infix where one operand is a boolean literal.
    has_bool_logic_with_literal: bool,
    /// L0056: any `for`-in whose iterable is an empty array literal.
    has_empty_array_for: bool,
    /// L0071: any function with more than 5 parameters.
    has_many_param_fn: bool,
    /// L0072: any `for`-in statement with a named loop variable.
    has_for_in_with_var: bool,
    /// L0073: any function with ≥2 contract clauses (potential duplicate).
    has_multi_contract_fn: bool,
    /// L0074: any `pure`-declared function AND any expression-statement call.
    has_pure_fn: bool,
    has_expr_stmt_call: bool,
    /// L0075: any function with a boolean-literal contract clause.
    has_bool_literal_contract: bool,
    /// L0085: any `struct` declaration with zero fields.
    has_empty_struct: bool,
    /// L0086: any `==` / `!=` infix where one operand is an empty string literal.
    has_empty_str_cmp: bool,
    /// L0087: any `pure` function AND any `print`/`println` call in program.
    has_pure_fn_with_print: bool,
    /// L0088: any `let _ = expr` wildcard discard binding.
    has_wildcard_let: bool,
    /// L0089: any `live` block containing `exit`/`abort` call.
    has_live_block: bool,
    /// L0090: any `if/else` where both arms have a return statement.
    has_if_else_with_returns: bool,
    /// L0091: any for-in with a Range iterable whose lo == hi as integer literals.
    has_empty_range_for: bool,
    /// L0092: any function with an `ensures false` clause.
    has_ensures_false: bool,
    /// L0093: any function with a parameter named "result".
    has_result_param: bool,
    /// L0094: any block containing consecutive break/continue.
    has_consecutive_break_continue: bool,
    /// L0095: any match expression with exactly one wildcard arm.
    has_single_wildcard_match: bool,
}

fn scan_lint_triggers(program: &Node) -> LintTriggers {
    let mut t = LintTriggers::default();
    scan_node(program, &mut t);
    // L0087: set composite trigger after full scan (run fn does fine-grained check).
    if t.has_pure_fn && t.has_call {
        t.has_pure_fn_with_print = true;
    }
    t
}

fn scan_node(node: &Node, t: &mut LintTriggers) {
    match node {
        Node::Assume { .. } => t.has_assume = true,
        Node::IndexExpression { target, .. } => {
            t.has_index = true;
            // L0053: index into an array literal.
            if matches!(target.as_ref(), Node::ArrayLiteral { .. }) {
                t.has_array_literal_index = true;
            }
        }
        Node::Match { arms, .. } => {
            t.has_match = true;
            // L0095: single wildcard arm.
            if arms.len() == 1 && matches!(arms[0].0, crate::Pattern::Wildcard) {
                t.has_single_wildcard_match = true;
            }
        }
        Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } => {
            t.has_infix = true;
            if operator == "/" || operator == "%" {
                t.has_division = true;
            }
            // L0040: arithmetic operator — combined with has_integer_literal
            // to gate the magic-number pass.
            if matches!(operator.as_str(), "+" | "-" | "*" | "/" | "%") {
                t.has_arith_int_literal = true;
            }
            // L0044: shift with a literal RHS.
            if matches!(operator.as_str(), "<<" | ">>")
                && matches!(right.as_ref(), Node::IntegerLiteral { .. })
            {
                t.has_literal_shift = true;
            }
            // L0048: any Bxor (^) infix expression.
            if operator == "^" {
                t.has_xor_infix = true;
            }
            // L0051: `==` or `!=` between two string literals.
            if matches!(operator.as_str(), "==" | "!=")
                && matches!(left.as_ref(), Node::StringLiteral { .. })
                && matches!(right.as_ref(), Node::StringLiteral { .. })
            {
                t.has_string_literal_cmp = true;
            }
            // L0055: `!=` where one operand is a boolean literal.
            if operator == "!="
                && (matches!(left.as_ref(), Node::BooleanLiteral { .. })
                    || matches!(right.as_ref(), Node::BooleanLiteral { .. }))
            {
                t.has_bool_neq_cmp = true;
            }
            // L0057-L0061: arithmetic identity operands (0 or 1 as literal).
            if matches!(operator.as_str(), "+" | "-" | "*" | "/" | "<<" | ">>")
                && (matches!(left.as_ref(), Node::IntegerLiteral { value: 0 | 1, .. })
                    || matches!(right.as_ref(), Node::IntegerLiteral { value: 0 | 1, .. }))
            {
                t.has_arith_identity = true;
            }
            // L0067-L0070: `&&` / `||` where one operand is a boolean literal.
            if matches!(operator.as_str(), "&&" | "||")
                && (matches!(left.as_ref(), Node::BooleanLiteral { .. })
                    || matches!(right.as_ref(), Node::BooleanLiteral { .. }))
            {
                t.has_bool_logic_with_literal = true;
            }
            // L0086: `==` / `!=` with an empty string literal operand.
            if matches!(operator.as_str(), "==" | "!=")
                && (matches!(left.as_ref(), Node::StringLiteral { value, .. } if value.is_empty())
                    || matches!(right.as_ref(), Node::StringLiteral { value, .. } if value.is_empty()))
            {
                t.has_empty_str_cmp = true;
            }
        }
        Node::PrefixExpression {
            operator, right, ..
        } => {
            t.has_prefix_expr = true;
            // L0052: `!` applied directly to a bool literal.
            if operator == "!" && matches!(right.as_ref(), Node::BooleanLiteral { .. }) {
                t.has_negated_bool_literal = true;
            }
        }
        Node::Function {
            parameters,
            pure,
            requires,
            ensures,
            ..
        } => {
            t.has_function = true;
            if parameters.len() > 5 {
                t.has_many_param_fn = true;
            }
            if *pure {
                t.has_pure_fn = true;
            }
            // L0073: ≥2 requires OR ≥2 ensures → possible duplicate.
            if requires.len() >= 2 || ensures.len() >= 2 {
                t.has_multi_contract_fn = true;
            }
            // L0075: any clause that is a boolean literal.
            let has_bool_clause = requires
                .iter()
                .chain(ensures.iter())
                .any(|c| matches!(c, Node::BooleanLiteral { .. }));
            if has_bool_clause {
                t.has_bool_literal_contract = true;
            }
            // L0092: ensures false clause.
            if ensures
                .iter()
                .any(|e| matches!(e, Node::BooleanLiteral { value: false, .. }))
            {
                t.has_ensures_false = true;
            }
            // L0093: parameter named "result".
            if parameters.iter().any(|(_, n)| n == "result") {
                t.has_result_param = true;
            }
        }
        Node::LetStatement { name, .. } => {
            t.has_let = true;
            t.has_let_binding = true;
            // L0088: wildcard discard `let _ = ...`
            if name == "_" {
                t.has_wildcard_let = true;
            }
        }
        Node::Block { stmts, .. } => {
            t.has_block = true;
            // L0017 trigger: a let inside a block (potential shadowing site).
            if stmts.iter().any(|s| matches!(s, Node::LetStatement { .. })) {
                t.has_let_in_nested_block = true;
            }
            // L0094: consecutive break/continue statements.
            {
                let mut last_was_jump = false;
                for s in stmts {
                    if matches!(s, Node::Break { .. } | Node::Continue { .. }) {
                        if last_was_jump {
                            t.has_consecutive_break_continue = true;
                            break;
                        }
                        last_was_jump = true;
                    } else {
                        last_was_jump = false;
                    }
                }
            }
        }
        Node::CallExpression { function, .. } => {
            t.has_call = true;
            if matches!(function.as_ref(), Node::Identifier { name, .. } if name == "format") {
                t.has_format_call = true;
            }
            // L0038: flag any call to `panic`.
            if matches!(function.as_ref(), Node::Identifier { name, .. } if name == "panic") {
                t.has_panic_call = true;
            }
            // L0039: set coarse trigger so the noreturn pass always
            // runs when any call exists; the pass does source-level
            // @noreturn detection internally.
            t.has_noreturn_call = true;
        }
        Node::IntegerLiteral { .. } => {
            t.has_integer_literal = true;
        }
        Node::BooleanLiteral { .. } => t.has_bool_literal = true,
        Node::WhileStatement { condition, .. } => {
            t.has_loop = true;
            t.has_while_stmt = true;
            if matches!(condition.as_ref(), Node::BooleanLiteral { value: true, .. }) {
                t.has_while_true = true;
            }
        }
        Node::ForInStatement { iterable, name, .. } => {
            t.has_loop = true;
            t.has_for_in_stmt = true;
            // L0056: iterable is an empty array literal `[]`.
            if matches!(iterable.as_ref(), Node::ArrayLiteral { items, .. } if items.is_empty()) {
                t.has_empty_array_for = true;
            }
            // L0072: named loop variable (parser currently rejects `_`).
            if !name.is_empty() {
                t.has_for_in_with_var = true;
            }
            // L0091: Range iterable with equal integer literal bounds.
            if let Node::Range { lo, hi, .. } = iterable.as_ref()
                && let (
                    Node::IntegerLiteral { value: lv, .. },
                    Node::IntegerLiteral { value: hv, .. },
                ) = (lo.as_ref(), hi.as_ref())
                && lv == hv
            {
                t.has_empty_range_for = true;
            }
        }
        Node::Assert { .. } => t.has_assert_stmt = true,
        Node::IfStatement {
            condition,
            alternative,
            ..
        } => {
            t.has_if_statement = true;
            if alternative.is_some() {
                t.has_if_with_else = true;
                // L0090: run fn does detailed same-literal check.
                t.has_if_else_with_returns = true;
            }
            if matches!(condition.as_ref(), Node::Assignment { .. }) {
                t.has_assign_in_cond = true;
            }
        }
        Node::StructLiteral { .. } => t.has_struct_literal = true,
        Node::MapLiteral { .. } => t.has_map_literal = true,
        Node::TryCatch { .. } => t.has_try_catch = true,
        Node::FloatLiteral { .. } => t.has_float_literal = true,
        Node::ExpressionStatement { expr, .. } => {
            if matches!(expr.as_ref(), Node::InfixExpression { operator, .. }
                if matches!(operator.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">="))
            {
                t.has_expr_stmt_cmp = true;
            }
            if matches!(expr.as_ref(), Node::CallExpression { .. }) {
                t.has_expr_stmt_call = true;
            }
        }
        Node::StringLiteral { .. } => t.has_string_literal = true,
        Node::Assignment { .. } => t.has_assignment = true,
        Node::ReturnStatement { .. } => t.has_return_in_block = true,
        Node::Break { .. } | Node::Continue { .. } => t.has_break_continue = true,
        Node::StructDecl { fields, .. } if fields.is_empty() => {
            t.has_empty_struct = true;
        }
        // L0089: live block — the pass itself inspects content.
        Node::LiveBlock { .. } => {
            t.has_live_block = true;
        }
        _ => {}
    }
    recurse_children(node, &mut |child| scan_node(child, t));
}

/// RES-198: top-level entry. Runs every lint, filters via the
/// `// resilient: allow LXXXX` comments found in `source`, and
/// returns the surviving diagnostics sorted by (line, column).
///
/// RES-1376: a single `scan_lint_triggers` AST visit caches which
/// lint trigger nodes appear; each `run_l00XX` is gated on the
/// matching flag. Lints whose trigger never appears are skipped —
/// single visit replaces up to 13 separate walks.
pub fn check(program: &Node, source: &str) -> Vec<Lint> {
    let mut out = Vec::new();
    let t = scan_lint_triggers(program);
    if t.has_function {
        run_l0001_unused_local(program, &mut out);
    }
    if t.has_match {
        run_l0002_unreachable_arm(program, &mut out);
    }
    if t.has_infix {
        run_l0003_self_comparison(program, &mut out);
        run_l0004_mixed_and_or(program, &mut out);
    }
    if t.has_function {
        run_l0005_redundant_return(program, &mut out);
    }
    if t.has_assume {
        run_l0006_assume_false(program, &mut out);
    }
    if t.has_block {
        run_l0007_unreachable_code(program, &mut out);
    }
    if t.has_match {
        run_l0008_duplicate_struct_match_arm(program, &mut out);
    }
    if t.has_division {
        run_l0009_division_by_zero(program, &mut out);
    }
    if t.has_function {
        run_l0010_no_contract(program, &mut out);
    }
    if t.has_let {
        run_l0011_unused_variable(program, &mut out);
    }
    if t.has_function {
        run_l0012_spec_provenance(program, source, &mut out);
    }
    if t.has_index {
        run_l0013_unchecked_indexing(program, &mut out);
    }
    if t.has_function {
        run_l0014_unused_function(program, &mut out);
    }
    if t.has_infix && t.has_integer_literal {
        run_l0015_const_overflow(program, &mut out);
    }
    if t.has_if_statement {
        run_l0016_constant_condition(program, &mut out);
    }
    if t.has_function && t.has_let_in_nested_block {
        run_l0017_variable_shadowing(program, &mut out);
    }
    if t.has_function {
        run_l0018_missing_return(program, &mut out);
    }
    if t.has_format_call {
        run_l0019_format_arity(program, &mut out);
    }
    if t.has_function {
        run_l0020_unused_parameter(program, &mut out);
    }
    if t.has_infix {
        run_l0021_redundant_bool_subexpr(program, &mut out);
    }
    if t.has_if_with_else {
        run_l0022_needless_else(program, &mut out);
    }
    if t.has_bool_literal && t.has_infix {
        run_l0023_bool_literal_comparison(program, &mut out);
    }
    if t.has_while_true && t.has_block {
        run_l0025_unreachable_after_infinite_loop(program, &mut out);
    }
    if t.has_struct_literal {
        run_l0024_struct_missing_fields(program, &mut out);
    }
    if t.has_map_literal {
        run_l0026_duplicate_map_key(program, &mut out);
    }
    if t.has_try_catch {
        run_l0027_empty_catch_block(program, &mut out);
    }
    if t.has_prefix_expr && t.has_bool_literal {
        run_l0028_negation_of_literal(program, &mut out);
    }
    if t.has_expr_stmt_cmp {
        run_l0029_comparison_result_discarded(program, &mut out);
    }
    if t.has_float_literal && t.has_infix {
        run_l0030_float_equality(program, &mut out);
    }
    if t.has_prefix_expr {
        run_l0031_double_negation(program, &mut out);
    }
    if t.has_assign_in_cond {
        run_l0032_assign_in_condition(program, &mut out);
    }
    if t.has_division {
        run_l0033_modulo_by_one(program, &mut out);
    }
    if t.has_loop && t.has_string_literal && t.has_infix {
        run_l0034_string_concat_in_loop(program, &mut out);
    }
    if t.has_call && t.has_block {
        run_l0035_unreachable_after_exit(program, &mut out);
    }
    if t.has_call && t.has_infix {
        run_l0036_len_negative_comparison(program, &mut out);
    }
    if t.has_assignment {
        run_l0037_self_assignment(program, &mut out);
    }
    // RES-1863: embedded-safety lints.
    if t.has_panic_call {
        run_l0038_panic_in_non_test(program, &mut out);
    }
    if t.has_noreturn_call && t.has_block {
        run_l0039_unreachable_after_noreturn(program, source, &mut out);
    }
    if t.has_function && t.has_arith_int_literal && t.has_integer_literal {
        run_l0040_magic_number(program, &mut out);
    }
    if t.has_return_in_block && t.has_if_with_else {
        run_l0041_redundant_else(program, &mut out);
    }
    if t.has_return_in_block && t.has_block {
        run_l0042_dead_code_after_return(program, &mut out);
    }
    if t.has_let_binding && t.has_function {
        run_l0043_shadowed_binding(program, &mut out);
    }
    if t.has_literal_shift {
        run_l0044_shift_out_of_range(program, &mut out);
    }
    if t.has_while_stmt {
        run_l0045_while_false(program, &mut out);
    }
    if t.has_for_in_stmt {
        run_l0046_empty_for_body(program, &mut out);
    }
    if t.has_empty_array_for {
        run_l0056_for_over_empty_array(program, &mut out);
    }
    if t.has_while_stmt {
        run_l0054_empty_while_body(program, &mut out);
    }
    if t.has_assert_stmt {
        run_l0047_vacuous_assert(program, &mut out);
    }
    if t.has_xor_infix {
        run_l0048_xor_with_self(program, &mut out);
    }
    if t.has_if_statement {
        run_l0049_empty_if_body(program, &mut out);
    }
    if t.has_if_with_else {
        run_l0050_redundant_else_after_loop_exit(program, &mut out);
    }
    if t.has_string_literal_cmp {
        run_l0051_string_literal_comparison(program, &mut out);
    }
    if t.has_negated_bool_literal {
        run_l0052_negated_bool_literal(program, &mut out);
    }
    if t.has_array_literal_index {
        run_l0053_out_of_bounds_literal_index(program, &mut out);
    }
    if t.has_bool_neq_cmp {
        run_l0055_redundant_bool_neq(program, &mut out);
    }
    if t.has_arith_identity {
        run_l0057_add_zero(program, &mut out);
        run_l0058_sub_zero(program, &mut out);
        run_l0059_mul_one(program, &mut out);
        run_l0060_div_one(program, &mut out);
        run_l0061_shift_zero(program, &mut out);
    }
    if t.has_infix {
        run_l0062_inequality_with_self(program, &mut out);
    }
    if t.has_break_continue {
        run_l0063_dead_after_break_continue(program, &mut out);
    }
    if t.has_if_statement {
        run_l0064_empty_else_block(program, &mut out);
    }
    if t.has_if_with_else {
        run_l0065_bool_identity_if(program, &mut out);
        run_l0066_bool_negation_if(program, &mut out);
    }
    if t.has_bool_logic_with_literal {
        run_l0067_and_true(program, &mut out);
        run_l0068_and_false(program, &mut out);
        run_l0069_or_true(program, &mut out);
        run_l0070_or_false(program, &mut out);
    }
    if t.has_many_param_fn {
        run_l0071_too_many_params(program, &mut out);
    }
    if t.has_for_in_with_var {
        run_l0072_unused_for_var(program, &mut out);
    }
    if t.has_multi_contract_fn {
        run_l0073_duplicate_contract_clause(program, &mut out);
    }
    if t.has_pure_fn && t.has_expr_stmt_call {
        run_l0074_pure_call_result_discarded(program, &mut out);
    }
    if t.has_bool_literal_contract {
        run_l0075_vacuous_contract_clause(program, &mut out);
    }
    if t.has_function {
        run_l0076_result_in_requires(program, &mut out);
        run_l0077_ensures_result_void(program, &mut out);
        run_l0078_param_shadows_builtin(program, &mut out);
        run_l0079_empty_function_body(program, &mut out);
        run_l0084_nested_function(program, &mut out);
    }
    if t.has_let_binding && t.has_assignment {
        run_l0080_dead_let_init(program, &mut out);
    }
    if t.has_assert_stmt {
        run_l0081_duplicate_assert(program, &mut out);
    }
    if t.has_if_with_else {
        run_l0082_both_branches_empty(program, &mut out);
    }
    if t.has_noreturn_call {
        run_l0083_noreturn_with_return_type(program, source, &mut out);
    }
    if t.has_empty_struct {
        run_l0085_empty_struct(program, &mut out);
    }
    // RES-2645: L0086–L0095 new lint rules.
    if t.has_empty_str_cmp {
        run_l0086_empty_string_comparison(program, &mut out);
    }
    if t.has_pure_fn_with_print {
        run_l0087_pure_fn_prints(program, &mut out);
    }
    if t.has_wildcard_let {
        run_l0088_wildcard_let_discard(program, &mut out);
    }
    if t.has_live_block {
        run_l0089_exit_in_live_block(program, &mut out);
    }
    if t.has_if_else_with_returns {
        run_l0090_both_arms_same_return(program, &mut out);
    }
    if t.has_empty_range_for {
        run_l0091_empty_range_for(program, &mut out);
    }
    if t.has_ensures_false {
        run_l0092_ensures_false(program, &mut out);
    }
    if t.has_result_param {
        run_l0093_param_named_result(program, &mut out);
    }
    if t.has_consecutive_break_continue {
        run_l0094_consecutive_break_continue(program, &mut out);
    }
    if t.has_single_wildcard_match {
        run_l0095_single_wildcard_match(program, &mut out);
    }
    let safety_critical = safety_critical_mode();
    if safety_critical {
        for lint in out.iter_mut() {
            if lint.code == "L0006" {
                lint.severity = Severity::Error;
            }
        }
    }

    // Filter via allow-comments.
    //
    // RES-308: L0011 is the rustc-style sibling of L0001's
    // unused-let case (both fire on the same `let unused = ...`
    // pattern, but with different phrasings). Authors who write
    // `// resilient: allow L0001` above a let are saying "I know
    // this is unused" — the same intent should silence L0011.
    // Treat the L0001 allow as implying the L0011 allow for the
    // same line, so dual emission stays user-suppressible with
    // a single comment.
    //
    // RES-1515: skip the per-lint clone+contains pair entirely when
    // no `// resilient: allow ...` comments exist in the source.
    // The common case for every fixture in `examples/` and every
    // CI input is an empty `allows` set; the retain closure was
    // cloning `l.code` per lint just to ask a HashSet that was
    // guaranteed empty. The `safety_critical && l.code == "L0006"`
    // gate is also a no-op when nothing would otherwise drop the
    // lint — early-out keeps every lint in that case too.
    let allows = collect_allow_comments(source);
    if !allows.is_empty() {
        out.retain(|l| {
            if safety_critical && l.code == "L0006" {
                return true;
            }
            if allows.contains(&(l.line, l.code.clone())) {
                return false;
            }
            if l.code == "L0011" && allows.contains(&(l.line, "L0001".to_string())) {
                return false;
            }
            true
        });
    }
    out.sort_by(|a, b| a.line.cmp(&b.line).then(a.column.cmp(&b.column)));
    out
}

/// RES-308: lint codes that are aliases of one another for
/// `--allow` / `--deny` purposes. `(primary, alias)` means a
/// flag targeting `primary` also affects `alias`. Today only
/// L0001 ↔ L0011 (unused-let warning re-phrased rustc-style).
pub const ALLOW_ALIASES: &[(&str, &str)] = &[("L0001", "L0011")];

/// RES-198: render a lint as a `<path>:<line>:<col>: <severity>[<code>]: <msg>`
/// single-line diagnostic. Matches the RES-080 prefix convention
/// used by the typechecker so users can copy-paste locations.
pub fn format_lint(l: &Lint, path: &str) -> String {
    format!(
        "{}:{}:{}: {}[{}]: {}",
        path, l.line, l.column, l.severity, l.code, l.message
    )
}

// ============================================================
// L0001: unused local binding
// ============================================================
//
// For each top-level fn, collect `let` + `static let` bindings
// inside the body, then check whether each bound name is
// referenced anywhere else in the body. Names starting with `_`
// are skipped (convention: user explicitly marks the binding as
// intentional).
//
// Limitation: shadowing isn't tracked precisely. `let x = 1;
// let x = 2;` counts `x` as "used" once the second binding's
// body or a later statement references `x`. MVP; precise
// shadow-aware analysis is a follow-up.

fn run_l0001_unused_local(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function { body, .. } => {
                l0001_check_body(body, out);
            }
            // RES-239: descend into impl block methods.
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { body, .. } = method {
                        l0001_check_body(body, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0001_check_body(body: &Node, out: &mut Vec<Lint>) {
    // RES-1533: borrow let-binding names and identifier-read names
    // from the AST into the `lets` Vec and `used` HashSet rather
    // than cloning every name. Same pattern as RES-1500 / RES-1525.
    let mut lets: Vec<(&str, Span)> = Vec::new();
    collect_lets_in(body, &mut lets);
    if !lets.is_empty() {
        let mut used: std::collections::HashSet<&str> = std::collections::HashSet::new();
        collect_identifier_reads_in(body, &mut used);
        for (name, span) in &lets {
            if name.starts_with('_') {
                continue;
            }
            if !used.contains(*name) {
                out.push(Lint {
                    code: "L0001".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "unused local binding `{}` — prefix with `_` to silence",
                        name
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    // RES-259: check match-arm pattern bindings (scoped per arm).
    // This is always called, regardless of whether `let` bindings exist.
    l0001_check_match_arms(body, out);
}

/// RES-259: collect the names bound by a pattern (one level of binding
/// per pattern, recursing into `Or` first-branch and `Bind` inner).
fn collect_pattern_bindings(pattern: &Pattern) -> Vec<&str> {
    match pattern {
        Pattern::Identifier(name) => vec![name.as_str()],
        Pattern::Bind(name, inner) => {
            let mut names = vec![name.as_str()];
            names.extend(collect_pattern_bindings(inner));
            names
        }
        // Or-patterns: all branches bind the same names (parser invariant);
        // read the first branch only to avoid duplicates.
        Pattern::Or(branches) => {
            if let Some(first) = branches.first() {
                collect_pattern_bindings(first)
            } else {
                vec![]
            }
        }
        // Wildcard and Literal introduce no bindings.
        Pattern::Wildcard | Pattern::Literal(_) => vec![],
        Pattern::Struct { fields, .. } => {
            // Pre-size to fields.len(): each field's sub-pattern most
            // commonly binds 0–1 names (Identifier / Wildcard), so the
            // field count is a tight upper bound for the typical case.
            // Sub-patterns that bind more (nested destructure) trigger
            // extend's amortised growth from there.
            let mut names = Vec::with_capacity(fields.len());
            for (_, sub) in fields {
                names.extend(collect_pattern_bindings(sub.as_ref()));
            }
            names
        }
        // RES-375: `Some(inner)` forwards to inner; `None` has no bindings.
        Pattern::Some(inner) => collect_pattern_bindings(inner.as_ref()),
        Pattern::None => vec![],
        // RES-923: Result patterns mirror Option's behaviour.
        Pattern::Ok(inner) | Pattern::Err(inner) => collect_pattern_bindings(inner.as_ref()),
        // RES-915: range patterns bind no names.
        Pattern::Range { .. } => vec![],
        // RES-400: enum-variant pattern bindings.
        Pattern::EnumVariant { payload, .. } => match payload {
            crate::EnumPatternPayload::None => vec![],
            crate::EnumPatternPayload::Named(fields) => {
                let mut names = Vec::with_capacity(fields.len());
                for (_, sub) in fields {
                    names.extend(collect_pattern_bindings(sub.as_ref()));
                }
                names
            }
            crate::EnumPatternPayload::Tuple(subs) => {
                let mut names = Vec::with_capacity(subs.len());
                for sub in subs {
                    names.extend(collect_pattern_bindings(sub));
                }
                names
            }
        },
        // RES-931: tuple-struct destructure — recurse into each field pattern.
        Pattern::TupleStruct { fields, .. } => {
            let mut names = Vec::with_capacity(fields.len());
            for sub in fields {
                names.extend(collect_pattern_bindings(sub));
            }
            names
        }
        // RES-932: anonymous tuple destructure — recurse positionally.
        Pattern::Tuple(items) => {
            let mut names = Vec::with_capacity(items.len());
            for sub in items {
                names.extend(collect_pattern_bindings(sub));
            }
            names
        }
    }
}

/// RES-259: walk every `Node::Match` in `node` and, for each arm,
/// check whether the arm's pattern bindings are used within that
/// arm's guard and body. Reports L0001 for each unused binding.
fn l0001_check_match_arms(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::Match {
            scrutinee, arms, ..
        } => {
            // Recurse into the scrutinee first.
            l0001_check_match_arms(scrutinee, out);

            for (pattern, guard, arm_body) in arms {
                let bindings = collect_pattern_bindings(pattern);
                if !bindings.is_empty() {
                    // Collect reads from the guard (if any) and the arm body.
                    let mut used: std::collections::HashSet<&str> =
                        std::collections::HashSet::new();
                    if let Some(g) = guard {
                        collect_identifier_reads_in(g, &mut used);
                    }
                    collect_identifier_reads_in(arm_body, &mut used);

                    // Use the arm body's span for the diagnostic position.
                    let (line, col) = span_of(arm_body)
                        .map(|s| (s.start.line as u32, s.start.column as u32))
                        .unwrap_or((1, 1));

                    for name in &bindings {
                        if name.starts_with('_') {
                            continue;
                        }
                        if !used.contains(*name) {
                            out.push(Lint {
                                code: "L0001".into(),
                                severity: Severity::Warning,
                                message: format!(
                                    "unused local binding `{}` — prefix with `_` to silence",
                                    name
                                ),
                                line,
                                column: col,
                            });
                        }
                    }
                }

                // Recurse into nested match expressions inside the arm body.
                l0001_check_match_arms(arm_body, out);
                if let Some(g) = guard {
                    l0001_check_match_arms(g, out);
                }
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                l0001_check_match_arms(s, out);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            l0001_check_match_arms(value, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            l0001_check_match_arms(condition, out);
            l0001_check_match_arms(consequence, out);
            if let Some(a) = alternative {
                l0001_check_match_arms(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            l0001_check_match_arms(condition, out);
            l0001_check_match_arms(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            l0001_check_match_arms(iterable, out);
            l0001_check_match_arms(body, out);
        }
        Node::LiveBlock { body, .. } => l0001_check_match_arms(body, out),
        Node::ReturnStatement { value: Some(v), .. } => l0001_check_match_arms(v, out),
        Node::ExpressionStatement { expr, .. } => l0001_check_match_arms(expr, out),
        _ => {}
    }
}

fn collect_lets_in<'a>(node: &'a Node, out: &mut Vec<(&'a str, Span)>) {
    match node {
        Node::LetStatement {
            name, value, span, ..
        } => {
            out.push((name.as_str(), *span));
            collect_lets_in(value, out);
        }
        Node::StaticLet {
            name, value, span, ..
        } => {
            out.push((name.as_str(), *span));
            collect_lets_in(value, out);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_lets_in(s, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_lets_in(condition, out);
            collect_lets_in(consequence, out);
            if let Some(a) = alternative {
                collect_lets_in(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_lets_in(condition, out);
            collect_lets_in(body, out);
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            span,
            ..
        } => {
            if !name.starts_with('_') {
                out.push((name.as_str(), *span));
            }
            collect_lets_in(iterable, out);
            collect_lets_in(body, out);
        }
        Node::LiveBlock { body, .. } => collect_lets_in(body, out),
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_lets_in(scrutinee, out);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    collect_lets_in(g, out);
                }
                collect_lets_in(arm_body, out);
            }
        }
        // RES-237: struct destructure — each local binding name is a
        // new `let`-equivalent that L0001 should track.
        Node::LetDestructureStruct {
            fields,
            value,
            span,
            ..
        } => {
            for (_field_name, local_name) in fields {
                out.push((local_name.as_str(), *span));
            }
            collect_lets_in(value, out);
        }
        _ => {}
    }
}

fn collect_identifier_reads_in<'a>(node: &'a Node, out: &mut std::collections::HashSet<&'a str>) {
    match node {
        Node::Identifier { name, .. } => {
            out.insert(name.as_str());
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            collect_identifier_reads_in(value, out);
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            collect_identifier_reads_in(v, out);
        }
        Node::ReturnStatement { value: None, .. } => {}
        Node::Assignment { value, .. } => {
            collect_identifier_reads_in(value, out);
        }
        Node::ExpressionStatement { expr, .. } => {
            collect_identifier_reads_in(expr, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_identifier_reads_in(condition, out);
            collect_identifier_reads_in(consequence, out);
            if let Some(a) = alternative {
                collect_identifier_reads_in(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_identifier_reads_in(condition, out);
            collect_identifier_reads_in(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_identifier_reads_in(iterable, out);
            collect_identifier_reads_in(body, out);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_identifier_reads_in(s, out);
            }
        }
        Node::LiveBlock {
            body, invariants, ..
        } => {
            collect_identifier_reads_in(body, out);
            for inv in invariants {
                collect_identifier_reads_in(inv, out);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            collect_identifier_reads_in(left, out);
            collect_identifier_reads_in(right, out);
        }
        Node::PrefixExpression { right, .. } => {
            collect_identifier_reads_in(right, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_identifier_reads_in(function, out);
            for a in arguments {
                collect_identifier_reads_in(a, out);
            }
        }
        Node::TryExpression { expr, .. } => {
            collect_identifier_reads_in(expr, out);
        }
        Node::OptionalChain { object, access, .. } => {
            collect_identifier_reads_in(object, out);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    collect_identifier_reads_in(a, out);
                }
            }
        }
        Node::IndexExpression { target, index, .. } => {
            collect_identifier_reads_in(target, out);
            collect_identifier_reads_in(index, out);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            collect_identifier_reads_in(target, out);
            collect_identifier_reads_in(index, out);
            collect_identifier_reads_in(value, out);
        }
        Node::FieldAccess { target, .. } => {
            collect_identifier_reads_in(target, out);
        }
        Node::FieldAssignment { target, value, .. } => {
            collect_identifier_reads_in(target, out);
            collect_identifier_reads_in(value, out);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                collect_identifier_reads_in(i, out);
            }
        }
        Node::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                collect_identifier_reads_in(v, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_identifier_reads_in(scrutinee, out);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    collect_identifier_reads_in(g, out);
                }
                collect_identifier_reads_in(arm_body, out);
            }
        }
        Node::Assert { condition, .. } => {
            collect_identifier_reads_in(condition, out);
        }
        // RES-237: assume(cond[, msg]) — identifiers inside the condition
        // and optional message are reads.
        Node::Assume {
            condition, message, ..
        } => {
            collect_identifier_reads_in(condition, out);
            if let Some(msg) = message {
                collect_identifier_reads_in(msg, out);
            }
        }
        // RES-237: {k -> v, ...} map literal — both keys and values are reads.
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_identifier_reads_in(k, out);
                collect_identifier_reads_in(v, out);
            }
        }
        // RES-237: #{item, ...} set literal — each item is a read.
        Node::SetLiteral { items, .. } => {
            for item in items {
                collect_identifier_reads_in(item, out);
            }
        }
        // RES-237: struct destructure — the RHS value is a read.
        Node::LetDestructureStruct { value, .. } => {
            collect_identifier_reads_in(value, out);
        }
        _ => {}
    }
}

// ============================================================
// L0002: unreachable arm after `_ =>`
// ============================================================
//
// A `_` pattern matches anything, so any arm textually following
// it can never fire. Walk every Match node; once a wildcard-only
// arm appears, flag the start of every subsequent arm.
//
// A `_` nested inside a `Pattern::Or` branch doesn't itself
// render the rest of the match unreachable (each branch of the
// Or tests independently); only a top-level wildcard arm does.
//
// RES-232: `Pattern::Bind` whose inner pattern is a default (e.g.
// `n @ _`, `n @ m`) also catches every value — treat as catch-all.

/// RES-232: mirrors `typechecker::pattern_is_default`. Returns `true`
/// when the pattern matches every value (wildcard, bare identifier,
/// bind whose inner is default, or-pattern with at least one default
/// branch).
fn pattern_is_default_for_lint(p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard | Pattern::Identifier(_) => true,
        // RES-915: range patterns never catch every Int (e.g. `1..=5`
        // misses 0, 6, …).
        Pattern::Literal(_) | Pattern::Range { .. } => false,
        Pattern::Or(branches) => branches.iter().any(pattern_is_default_for_lint),
        Pattern::Bind(_, inner) => pattern_is_default_for_lint(inner),
        Pattern::Struct { fields, .. } => fields
            .iter()
            .all(|(_, sub)| pattern_is_default_for_lint(sub.as_ref())),
        // RES-375: Option patterns are never catch-alls by themselves.
        Pattern::Some(_) | Pattern::None | Pattern::Ok(_) | Pattern::Err(_) => false,
        // RES-400: enum-variant patterns are never catch-alls — each
        // matches one specific variant.
        Pattern::EnumVariant { .. } => false,
        // RES-931: a tuple-struct pattern is a catch-all iff every
        // positional sub-pattern is itself a default — `Pair(_, _)`
        // catches every `Pair`, but `Pair(0, _)` does not.
        Pattern::TupleStruct { fields, .. } => fields.iter().all(pattern_is_default_for_lint),
        // RES-932: same shape — `(_, _)` is a catch-all over 2-tuples;
        // `(0, _)` is not.
        Pattern::Tuple(items) => items.iter().all(pattern_is_default_for_lint),
    }
}

fn run_l0002_unreachable_arm(program: &Node, out: &mut Vec<Lint>) {
    walk_matches(program, out);
}

fn walk_matches(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_matches(&s.node, out);
            }
        }
        Node::Function { body, .. } => walk_matches(body, out),
        // RES-239: descend into impl block methods.
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                walk_matches(method, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_matches(s, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            // Find the first arm whose pattern is a bare wildcard.
            // Report subsequent arms at the arm body's span (the
            // closest accessible position — `Pattern` itself
            // doesn't carry a span today). Falls back to the
            // scrutinee's span when the body has a default span.
            let scrut_line = match span_of(scrutinee) {
                Some(s) => s.start.line as u32,
                None => 1,
            };
            let scrut_col = match span_of(scrutinee) {
                Some(s) => s.start.column as u32,
                None => 1,
            };
            let mut saw_wild = false;
            for (pat, _guard, arm_body) in arms {
                if saw_wild {
                    let arm_span = span_of(arm_body);
                    let (line, col) = match arm_span {
                        Some(s) if s.start.line > 0 => (s.start.line as u32, s.start.column as u32),
                        _ => (scrut_line, scrut_col),
                    };
                    out.push(Lint {
                        code: "L0002".into(),
                        severity: Severity::Warning,
                        message:
                            "arm is unreachable — an earlier `_` arm already matches everything"
                                .into(),
                        line,
                        column: col,
                    });
                }
                walk_matches(arm_body, out);
                if pattern_is_default_for_lint(pat) {
                    saw_wild = true;
                }
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_matches(condition, out);
            walk_matches(consequence, out);
            if let Some(a) = alternative {
                walk_matches(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_matches(condition, out);
            walk_matches(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_matches(iterable, out);
            walk_matches(body, out);
        }
        Node::LiveBlock { body, .. } => walk_matches(body, out),
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::ReturnStatement {
            value: Some(value), ..
        } => {
            walk_matches(value, out);
        }
        Node::ExpressionStatement { expr, .. } => walk_matches(expr, out),
        Node::InfixExpression { left, right, .. } => {
            walk_matches(left, out);
            walk_matches(right, out);
        }
        Node::PrefixExpression { right, .. } => walk_matches(right, out),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            walk_matches(function, out);
            for a in arguments {
                walk_matches(a, out);
            }
        }
        _ => {}
    }
}

fn struct_literal_match_arm_key(pat: &Pattern) -> Option<String> {
    let Pattern::Struct {
        struct_name,
        fields,
        has_rest,
    } = pat
    else {
        return None;
    };
    if *has_rest || fields.is_empty() {
        return None;
    }
    // RES-1774: pre-size to fields.len() — one push per field on the
    // happy path (loop returns None early on any non-literal).
    let mut parts = Vec::with_capacity(fields.len());
    for (fname, sub) in fields {
        match sub.as_ref() {
            Pattern::Literal(Node::IntegerLiteral { value, .. }) => {
                parts.push(format!("{}={}", fname, value));
            }
            _ => return None,
        }
    }
    parts.sort();
    Some(format!("{}|{}", struct_name, parts.join("|")))
}

fn run_l0008_duplicate_struct_match_arm(program: &Node, out: &mut Vec<Lint>) {
    walk_dup_struct_arms(program, out);
}

fn walk_dup_struct_arms(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_dup_struct_arms(&s.node, out);
            }
        }
        Node::Function { body, .. } => walk_dup_struct_arms(body, out),
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                walk_dup_struct_arms(method, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_dup_struct_arms(s, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_dup_struct_arms(scrutinee, out);
            let scrut_line = match span_of(scrutinee) {
                Some(s) => s.start.line as u32,
                None => 1,
            };
            let scrut_col = match span_of(scrutinee) {
                Some(s) => s.start.column as u32,
                None => 1,
            };
            let mut seen = std::collections::HashSet::<String>::new();
            for (pat, guard, arm_body) in arms {
                if guard.is_none()
                    && let Some(k) = struct_literal_match_arm_key(pat)
                    && !seen.insert(k)
                {
                    let arm_span = span_of(arm_body);
                    let (line, col) = match arm_span {
                        Some(s) if s.start.line > 0 => (s.start.line as u32, s.start.column as u32),
                        _ => (scrut_line, scrut_col),
                    };
                    out.push(Lint {
                        code: "L0008".into(),
                        severity: Severity::Warning,
                        message: "unreachable match arm — an earlier arm matches the same struct literal pattern"
                            .into(),
                        line,
                        column: col,
                    });
                }
                walk_dup_struct_arms(arm_body, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_dup_struct_arms(condition, out);
            walk_dup_struct_arms(consequence, out);
            if let Some(a) = alternative {
                walk_dup_struct_arms(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_dup_struct_arms(condition, out);
            walk_dup_struct_arms(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_dup_struct_arms(iterable, out);
            walk_dup_struct_arms(body, out);
        }
        Node::LiveBlock { body, .. } => walk_dup_struct_arms(body, out),
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::ReturnStatement {
            value: Some(value), ..
        } => {
            walk_dup_struct_arms(value, out);
        }
        Node::ExpressionStatement { expr, .. } => walk_dup_struct_arms(expr, out),
        Node::InfixExpression { left, right, .. } => {
            walk_dup_struct_arms(left, out);
            walk_dup_struct_arms(right, out);
        }
        Node::PrefixExpression { right, .. } => walk_dup_struct_arms(right, out),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            walk_dup_struct_arms(function, out);
            for a in arguments {
                walk_dup_struct_arms(a, out);
            }
        }
        _ => {}
    }
}

// ============================================================
// L0003: comparison `x == x` always true
// ============================================================
//
// Walk every InfixExpression with operator `==` or `!=`. If
// both sides are syntactically the same Identifier, flag.
// `!=` gets flagged too: `x != x` is always false, equally
// suspect. We report both under the single L0003 code with
// wording tuned to the operator.

fn run_l0003_self_comparison(program: &Node, out: &mut Vec<Lint>) {
    walk_self_comparisons(program, out);
}

fn walk_self_comparisons(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && (operator == "==" || operator == "!=")
        && let (Node::Identifier { name: ln, .. }, Node::Identifier { name: rn, .. }) =
            (left.as_ref(), right.as_ref())
        && ln == rn
    {
        let always = if operator == "==" {
            "always true"
        } else {
            "always false"
        };
        out.push(Lint {
            code: "L0003".into(),
            severity: Severity::Warning,
            message: format!("comparing `{}` to itself is {} (likely a typo)", ln, always),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    // Recurse generically.
    recurse_children(node, &mut |child| walk_self_comparisons(child, out));
}

// ============================================================
// L0004: mixing `&&` and `||` without parens
// ============================================================
//
// Flag any InfixExpression whose operator is `&&` / `||` AND
// whose immediate child (left or right) has the opposite
// boolean operator. Paren-disambiguation isn't tracked in the
// AST, so this has a controlled false-positive rate on
// explicitly-parenthesized code — users suppress with
// `allow L0004`.

fn run_l0004_mixed_and_or(program: &Node, out: &mut Vec<Lint>) {
    walk_and_or(program, out);
}

fn walk_and_or(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
    {
        let opposite = match operator.as_str() {
            "&&" => Some("||"),
            "||" => Some("&&"),
            _ => None,
        };
        if let Some(opp) = opposite
            && (has_top_level_op(left, opp) || has_top_level_op(right, opp))
        {
            out.push(Lint {
                code: "L0004".into(),
                severity: Severity::Warning,
                message: format!(
                    "mixing `{}` and `{}` — add explicit parens to disambiguate precedence",
                    operator, opp
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
    recurse_children(node, &mut |child| walk_and_or(child, out));
}

fn has_top_level_op(node: &Node, op: &str) -> bool {
    matches!(node, Node::InfixExpression { operator, .. } if operator == op)
}

/// RES-198: best-effort span extraction. Mirrors the helper in
/// `lsp_server`; duplicated here so `lint` can stay feature-gate
/// independent of `lsp`.
fn span_of(node: &Node) -> Option<Span> {
    match node {
        Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::BytesLiteral { span, .. }
        | Node::Identifier { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::TryExpression { span, .. }
        | Node::OptionalChain { span, .. }
        | Node::ArrayLiteral { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::Block { span, .. }
        | Node::Match { span, .. }
        | Node::LetStatement { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::ExpressionStatement { span, .. }
        | Node::Function { span, .. }
        | Node::LiveBlock { span, .. }
        | Node::Assert { span, .. }
        | Node::Assume { span, .. } => Some(*span),
        _ => None,
    }
}

// ============================================================
// L0005: redundant trailing `return;`
// ============================================================
//
// A bare `return;` (no value) at the end of a function body is
// redundant — the function would return Void without it. We
// don't flag `return VALUE;` trailing, since that IS load-
// bearing (Resilient doesn't have implicit-last-expression
// returns today).

fn run_l0005_redundant_return(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function { body, .. } => {
                l0005_check_fn_body(body, out);
            }
            // RES-239: check methods inside impl blocks.
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { body, .. } = method {
                        l0005_check_fn_body(body, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0005_check_fn_body(body: &Node, out: &mut Vec<Lint>) {
    if let Node::Block {
        stmts: body_stmts, ..
    } = body
        && let Some(Node::ReturnStatement { value: None, span }) = body_stmts.last()
    {
        out.push(Lint {
            code: "L0005".into(),
            severity: Severity::Warning,
            message: "redundant `return;` at end of function body — remove it".into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
}

// ============================================================
// L0006: assume(false) — vacuously true hypothesis
// ============================================================
//
// `assume(false)` causes the SMT verifier to treat `false` as a
// precondition, making every subsequent obligation trivially satisfied
// (ex-falso). At runtime the call halts unconditionally. This is
// almost always a mistake; flag it as a warning.
//
// Only `assume(false)` with a literal `false` argument is flagged.
// `assume(true)` and `assume(x > 0)` are silent.

fn run_l0006_assume_false(program: &Node, out: &mut Vec<Lint>) {
    walk_assume_false(program, out);
}

fn walk_assume_false(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Assume {
        condition, span, ..
    } = node
        && matches!(
            condition.as_ref(),
            Node::BooleanLiteral { value: false, .. }
        )
    {
        out.push(Lint {
            code: "L0006".into(),
            severity: Severity::Warning,
            message: "assume(false): all subsequent verification obligations in this block \
                are vacuously discharged; code after this point is unreachable at runtime"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_assume_false(child, out));
}

// ============================================================
// L0007: unreachable code after unconditional `return`
// ============================================================
//
// Walk every Block node. Once a `ReturnStatement` is seen, any
// subsequent node in the same block is unreachable. Only the
// FIRST unreachable statement is reported (pointing to it tells
// the user exactly where dead code begins). Nested blocks are
// walked independently — a `return` inside an `if` branch does
// not make statements after the `if` unreachable.
//
// The language does not yet have `break`/`continue` statements;
// if those are added, this lint should be extended to treat them
// as additional terminators.

fn run_l0007_unreachable_code(program: &Node, out: &mut Vec<Lint>) {
    walk_unreachable(program, out);
}

fn walk_unreachable(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::Block { stmts, .. } => {
            let mut saw_terminator = false;
            for stmt in stmts {
                if saw_terminator {
                    if let Some(span) = span_of(stmt) {
                        out.push(Lint {
                            code: "L0007".into(),
                            severity: Severity::Warning,
                            message: "unreachable code after `return`".into(),
                            line: span.start.line as u32,
                            column: span.start.column as u32,
                        });
                    }
                    // Report only the first unreachable statement.
                    break;
                }
                if matches!(stmt, Node::ReturnStatement { .. }) {
                    saw_terminator = true;
                }
                // Descend into nested blocks regardless of whether we have
                // seen a terminator — the nested scope is independent.
                walk_unreachable(stmt, out);
            }
        }
        Node::Program(stmts) => {
            for s in stmts {
                walk_unreachable(&s.node, out);
            }
        }
        Node::Function { body, .. } => walk_unreachable(body, out),
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                walk_unreachable(method, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_unreachable(condition, out);
            walk_unreachable(consequence, out);
            if let Some(a) = alternative {
                walk_unreachable(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_unreachable(condition, out);
            walk_unreachable(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_unreachable(iterable, out);
            walk_unreachable(body, out);
        }
        Node::LiveBlock { body, .. } => walk_unreachable(body, out),
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_unreachable(scrutinee, out);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    walk_unreachable(g, out);
                }
                walk_unreachable(arm_body, out);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            walk_unreachable(value, out);
        }
        Node::ReturnStatement { value: Some(v), .. } => walk_unreachable(v, out),
        Node::ExpressionStatement { expr, .. } => walk_unreachable(expr, out),
        _ => {}
    }
}

// ============================================================
// L0009: integer division by zero (RES-350)
// ============================================================
//
// Division by zero on Cortex-M is a hard fault — no signal, no
// trap handler in the default configuration, just a locked-up
// core. This lint flags `a / b` and `a % b` when `b` cannot be
// proven non-zero given the information available.
//
// Two modes:
//
// - Default build: only literal-zero divisors fire. `a / 0`,
//   `a % 0`, `a / 0.0`, `a % 0.0` are statically obvious bugs
//   and deserve the warning regardless of SMT availability.
// - `--features z3`: the lint additionally asks Z3 "given the
//   enclosing fn's `requires` clauses, is `divisor != 0`
//   provable?". If Z3 returns `Some(true)`, the divisor is
//   proven non-zero and the lint stays silent. Any other verdict
//   (`Some(false)`, `None`, or timeout) triggers the warning with
//   a hint pointing at the missing precondition.
//
// The ticket proposed code `L0004` for this lint, but `L0004` is
// already shipped as the mixed-`&&`/`||` paren warning; renaming
// would silently flip the meaning of every `// resilient: allow
// L0004` comment in the wild. We allocate `L0009` — the next
// unused slot — and note the conflict in the PR that added this
// file.

fn run_l0009_division_by_zero(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function { body, requires, .. } => {
                let axioms = combine_axioms(requires, body);
                l0009_check_body(body, &axioms, out);
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { body, requires, .. } = method {
                        let axioms = combine_axioms(requires, body);
                        l0009_check_body(body, &axioms, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// RES-133b: combine the function's `requires` clauses with the
/// leading `assume(P)` predicates from the body. The result is the
/// axiom set the divide-by-zero prover sees — assumes at the start
/// of a fn body are valid axioms because they're runtime-checked
/// before any expression evaluates.
fn combine_axioms(requires: &[Node], body: &Node) -> Vec<Node> {
    let mut axioms: Vec<Node> = requires.to_vec();
    axioms.extend(crate::assume_axioms::collect_leading_assume_axioms(body));
    axioms
}

/// RES-350: walk one fn body, flagging divisions by zero. The
/// `requires` slice belongs to the enclosing fn and is handed to
/// Z3 as assumption axioms (feature-gated).
fn l0009_check_body(body: &Node, requires: &[Node], out: &mut Vec<Lint>) {
    walk_divisions(body, requires, out);
}

fn walk_divisions(node: &Node, requires: &[Node], out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && (operator == "/" || operator == "%")
    {
        match right.as_ref() {
            Node::IntegerLiteral { value: 0, .. } => {
                out.push(Lint {
                    code: "L0009".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "division by zero: `{}` with a literal-zero divisor is a hard fault on Cortex-M",
                        operator
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
            Node::FloatLiteral { value, .. } if *value == 0.0 => {
                out.push(Lint {
                    code: "L0009".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "division by zero: `{}` with a literal-zero divisor is a hard fault on Cortex-M",
                        operator
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
            other => {
                // Non-literal divisor: under `--features z3` ask the
                // solver whether the enclosing fn's preconditions
                // force it non-zero. Without Z3 we stay silent to
                // avoid false positives.
                if let Some(lint) = l0009_z3_check(left, operator, other, requires, span) {
                    out.push(lint);
                }
            }
        }
    }
    // Recurse through the same generic walker the other lints use.
    recurse_children(node, &mut |child| walk_divisions(child, requires, out));
}

#[cfg(feature = "z3")]
fn l0009_z3_check(
    _left: &Node,
    operator: &str,
    right: &Node,
    requires: &[Node],
    span: &Span,
) -> Option<Lint> {
    use crate::verifier_z3;
    // Construct the synthetic obligation `<right> != 0`.
    let obligation = Node::InfixExpression {
        left: Box::new(right.clone()),
        operator: "!=".to_string(),
        right: Box::new(Node::IntegerLiteral {
            value: 0,
            span: crate::span::Span::default(),
        }),
        span: crate::span::Span::default(),
    };
    let empty = std::collections::HashMap::new();
    // 1 s is plenty for simple non-zero obligations; if the user
    // has unusually complex preconditions they can downgrade via
    // `// resilient: allow L0009`.
    let (verdict, _cert, _cx, _timeout) =
        verifier_z3::prove_with_axioms_and_timeout(&obligation, &empty, requires, 1000);
    if verdict == Some(true) {
        return None;
    }
    Some(Lint {
        code: "L0009".into(),
        severity: Severity::Warning,
        message: format!(
            "division may be by zero: `{}` divisor is not proven non-zero; \
             add `requires <divisor> != 0;` to the enclosing fn, or \
             silence with `// resilient: allow L0009`",
            operator
        ),
        line: span.start.line as u32,
        column: span.start.column as u32,
    })
}

#[cfg(not(feature = "z3"))]
fn l0009_z3_check(
    _left: &Node,
    _operator: &str,
    _right: &Node,
    _requires: &[Node],
    _span: &Span,
) -> Option<Lint> {
    None
}

// ============================================================
// Shared AST walker. Not exhaustive — covers the shapes the
// five lints actually need to descend through.
// ============================================================

fn recurse_children<F: FnMut(&Node)>(node: &Node, f: &mut F) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                f(&s.node);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            f(body);
            for r in requires {
                f(r);
            }
            for e in ensures {
                f(e);
            }
        }
        // RES-239: descend into impl block methods so L0003/L0004/L0006 cover methods.
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                f(method);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                f(s);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => f(value),
        Node::ReturnStatement { value: Some(v), .. } => f(v),
        Node::Assignment { value, .. } => f(value),
        Node::ExpressionStatement { expr, .. } => f(expr),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            f(condition);
            f(consequence);
            if let Some(a) = alternative {
                f(a);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            f(condition);
            f(body);
        }
        Node::ForInStatement { iterable, body, .. } => {
            f(iterable);
            f(body);
        }
        Node::LiveBlock {
            body, invariants, ..
        } => {
            f(body);
            for inv in invariants {
                f(inv);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            f(left);
            f(right);
        }
        Node::PrefixExpression { right, .. } => f(right),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            f(function);
            for a in arguments {
                f(a);
            }
        }
        Node::TryExpression { expr, .. } => f(expr),
        Node::OptionalChain { object, access, .. } => {
            f(object);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    f(a);
                }
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            f(scrutinee);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    f(g);
                }
                f(arm_body);
            }
        }
        Node::IndexExpression { target, index, .. } => {
            f(target);
            f(index);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            f(target);
            f(index);
            f(value);
        }
        Node::FieldAccess { target, .. } => f(target),
        Node::FieldAssignment { target, value, .. } => {
            f(target);
            f(value);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                f(i);
            }
        }
        Node::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                f(v);
            }
        }
        Node::Assert { condition, .. } => f(condition),
        Node::Assume {
            condition, message, ..
        } => {
            f(condition);
            if let Some(msg) = message {
                f(msg);
            }
        }
        _ => {}
    }
}

// ============================================================
// Suppress-comment scanning
// ============================================================
//
// Finds every `// resilient: allow LXXXX` line in the source
// and returns the set of `(line, code)` pairs that should be
// suppressed. An allow on line K suppresses diagnostics on line
// K+1. Only `L` codes are recognized; `// resilient: allow foo`
// is treated as ordinary text.

fn collect_allow_comments(source: &str) -> std::collections::HashSet<(u32, String)> {
    let mut out = std::collections::HashSet::new();
    for (i, raw) in source.lines().enumerate() {
        let line_no = (i as u32) + 1;
        let Some(pos) = raw.find("// resilient: allow") else {
            continue;
        };
        let tail = &raw[pos + "// resilient: allow".len()..];
        // Collect every LXXXX token on the rest of the line.
        for word in tail.split(|c: char| c == ',' || c.is_whitespace()) {
            let w = word.trim();
            if w.starts_with('L') && w.len() == 5 && w.chars().skip(1).all(|c| c.is_ascii_digit()) {
                out.insert((line_no + 1, w.to_string()));
            }
        }
    }
    out
}

// ============================================================
// L0010: function has no requires/ensures contract
// ============================================================
//
// Functions that declare neither `requires` nor `ensures` carry
// no machine-verifiable safety contract.  In safety-critical
// embedded code that is almost always an oversight, so we flag
// it as a warning.  Users can suppress with:
//   `// resilient: allow L0010`
// or add trivial stubs (the LSP `codeAction` offers this as a
// quick-fix: "Add contract stubs").
//
// Deliberately excluded from the check:
//   - Functions that start with `_` (test helpers, entry stubs).
//   - Anonymous functions (name == "").
//   - Impl-block methods (those inherit the struct's invariants).

fn run_l0010_no_contract(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        if let Node::Function {
            name,
            requires,
            ensures,
            span,
            ..
        } = &spanned.node
        {
            // Skip anonymous fns and underscore-prefixed helpers.
            if name.is_empty() || name.starts_with('_') {
                continue;
            }
            if requires.is_empty() && ensures.is_empty() {
                out.push(Lint {
                    code: "L0010".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "function `{name}` has no `requires`/`ensures` contract; \
                         add contract stubs or suppress with `// resilient: allow L0010`"
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
}

// ============================================================
// L0011: unused variable warning (RES-308)
// ============================================================
//
// Issue #100 / RES-308 specifies a dedicated lint for `let`
// bindings whose name is never subsequently read. The earlier
// L0001 lint also flags this case (it covers all "unused local
// binding" forms — `let`, `for`-loop vars, struct-destructure,
// match-arm bindings) but the ticket specifically asks for a
// distinct code with the rustc-style message
// `variable \`x\` is assigned but never used`.
//
// `KNOWN_CODES` already reserves `L0002` for "unreachable arm
// after `_`" with a substantial test suite; per the
// repo-wide test-protection rule we cannot retire that code
// without breaking unrelated tests, so the new lint is
// allocated the next free slot, `L0011`.
//
// Behaviour:
//   - Walks every `let` / `static let` / struct-destructure
//     binding inside fn bodies (incl. impl methods).
//   - A binding is "used" if its name appears in any
//     identifier-read position elsewhere in the same fn body.
//   - Names starting with `_` are exempt.
//   - Reports at the binding's source span — the same site the
//     ticket asks about (`file:line:col`).
//
// `for x in arr` loop variables are intentionally skipped here
// because L0001 already covers them and the ticket's wording
// ("`let x = expr;`") doesn't mention them. Match-arm pattern
// bindings are also out of scope (L0001 covers those).

fn run_l0011_unused_variable(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function { body, .. } => {
                l0011_check_body(body, out);
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { body, .. } = method {
                        l0011_check_body(body, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0011_check_body(body: &Node, out: &mut Vec<Lint>) {
    // RES-1533: same borrow pattern as `l0001_check_body`.
    let mut lets: Vec<(&str, Span)> = Vec::new();
    l0011_collect_let_bindings(body, &mut lets);
    if lets.is_empty() {
        return;
    }
    let mut used: std::collections::HashSet<&str> = std::collections::HashSet::new();
    collect_identifier_reads_in(body, &mut used);
    for (name, span) in &lets {
        if name.starts_with('_') {
            continue;
        }
        if !used.contains(*name) {
            out.push(Lint {
                code: "L0011".into(),
                severity: Severity::Warning,
                message: format!("variable `{}` is assigned but never used", name),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
}

/// L0011-specific binding collector. Mirrors `collect_lets_in` but
/// scoped to the let-style forms named by the ticket: plain `let`,
/// `static let`, and struct-destructure. `for`-loop induction
/// variables are deliberately skipped — L0001 already flags those.
fn l0011_collect_let_bindings<'a>(node: &'a Node, out: &mut Vec<(&'a str, Span)>) {
    match node {
        Node::LetStatement {
            name, value, span, ..
        } => {
            out.push((name.as_str(), *span));
            l0011_collect_let_bindings(value, out);
        }
        Node::StaticLet {
            name, value, span, ..
        } => {
            out.push((name.as_str(), *span));
            l0011_collect_let_bindings(value, out);
        }
        Node::LetDestructureStruct {
            fields,
            value,
            span,
            ..
        } => {
            for (_field_name, local_name) in fields {
                out.push((local_name.as_str(), *span));
            }
            l0011_collect_let_bindings(value, out);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                l0011_collect_let_bindings(s, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            l0011_collect_let_bindings(condition, out);
            l0011_collect_let_bindings(consequence, out);
            if let Some(a) = alternative {
                l0011_collect_let_bindings(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            l0011_collect_let_bindings(condition, out);
            l0011_collect_let_bindings(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            l0011_collect_let_bindings(iterable, out);
            l0011_collect_let_bindings(body, out);
        }
        Node::LiveBlock { body, .. } => l0011_collect_let_bindings(body, out),
        Node::Match {
            scrutinee, arms, ..
        } => {
            l0011_collect_let_bindings(scrutinee, out);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    l0011_collect_let_bindings(g, out);
                }
                l0011_collect_let_bindings(arm_body, out);
            }
        }
        _ => {}
    }
}

// ============================================================
// L0012: spec annotation lacks `// source:` provenance comment (RES-397)
// ============================================================
//
// Reddit critique (https://www.reddit.com/r/VibeCodersNest/comments/1ssv8ih/)
// raised the strongest version of "filtering ≠ safety": if an LLM
// invents the invariant, a self-consistent wrong spec is provable
// and useless. The verification machinery is sound — it doesn't
// trust the LLM — but the *invariants themselves* have no
// provenance trail today. A wrong invariant from an LLM is
// indistinguishable from a right one once it's in the source.
//
// L0012 requires every spec-bearing site to be preceded by a
// `// source: <canonical-reference>` comment on the line above:
//
//   // source: RFC 9293 §3.5
//   fn handle_segment(seq: int) requires seq >= 0 { ... }
//
//   // source: STM32F4 Reference Manual RM0090 §10.4.5
//   assume(adc_value < 4096);
//
// Sites covered:
//   - Function declarations with non-empty `requires`, `ensures`,
//     `recovers_to`, or `fails`.
//   - `assume(...)` statements.
//
// Suppress with `// resilient: allow L0012`. The default severity
// is Warning; `--deny L0012` escalates to Error.

/// RES-397: collect line numbers that have a spec annotation on
/// them, given a `// source: ...` comment on the line above. The
/// returned set contains `K+1` for every `// source: ...` on line
/// `K`. This mirrors the line-offset convention used by
/// `collect_allow_comments`.
fn collect_source_comments(source: &str) -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::new();
    for (i, raw) in source.lines().enumerate() {
        let line_no = (i as u32) + 1;
        let trimmed = raw.trim_start();
        if let Some(rest) = trimmed.strip_prefix("// source:")
            && !rest.trim().is_empty()
        {
            out.insert(line_no + 1);
        }
    }
    out
}

fn run_l0012_spec_provenance(program: &Node, source: &str, out: &mut Vec<Lint>) {
    let sources = collect_source_comments(source);
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        l0012_walk(&spanned.node, &sources, out);
    }
}

fn l0012_walk(node: &Node, sources: &std::collections::HashSet<u32>, out: &mut Vec<Lint>) {
    match node {
        Node::Function {
            name,
            requires,
            ensures,
            recovers_to,
            fails,
            body,
            span,
            ..
        } => {
            let has_spec = !requires.is_empty()
                || !ensures.is_empty()
                || recovers_to.is_some()
                || !fails.is_empty();
            if has_spec && !name.is_empty() && !name.starts_with('_') {
                let fn_line = span.start.line as u32;
                if !sources.contains(&fn_line) {
                    out.push(Lint {
                        code: "L0012".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "function `{name}` has spec annotations without provenance — \
                             add `// source: <canonical-reference>` on the line above, \
                             or suppress with `// resilient: allow L0012`"
                        ),
                        line: fn_line,
                        column: span.start.column as u32,
                    });
                }
            }
            l0012_walk(body, sources, out);
        }
        Node::Assume { span, .. } => {
            let line = span.start.line as u32;
            if !sources.contains(&line) {
                out.push(Lint {
                    code: "L0012".into(),
                    severity: Severity::Warning,
                    message: "`assume()` without provenance — \
                              add `// source: <canonical-reference>` on the line above, \
                              or suppress with `// resilient: allow L0012`"
                        .to_string(),
                    line,
                    column: span.start.column as u32,
                });
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                l0012_walk(stmt, sources, out);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            l0012_walk(consequence, sources, out);
            if let Some(alt) = alternative {
                l0012_walk(alt, sources, out);
            }
        }
        Node::WhileStatement { body, .. } => l0012_walk(body, sources, out),
        Node::ForInStatement { body, .. } => l0012_walk(body, sources, out),
        Node::LiveBlock { body, .. } => l0012_walk(body, sources, out),
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                l0012_walk(method, sources, out);
            }
        }
        _ => {}
    }
}

fn run_l0013_unchecked_indexing(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        l0013_walk(&spanned.node, out);
    }
}

fn l0013_walk(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::IndexExpression {
            target,
            index,
            span,
            ..
        } => {
            // RES-798: check if this index access was proven in-bounds by
            // the bounds_check pass. If not, emit L0013 warning.
            if !crate::bounds_check::is_proven_site(*span) {
                out.push(Lint {
                    code: "L0013".into(),
                    severity: Severity::Warning,
                    message: "unchecked array indexing — bounds not proven at compile time; \
                         use --deny-unproven-bounds to require proof, or suppress with \
                         `// resilient: allow L0013`"
                        .to_string(),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
            // Recurse into both target and index
            l0013_walk(target, out);
            l0013_walk(index, out);
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                l0013_walk(stmt, out);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            for req in requires {
                l0013_walk(req, out);
            }
            for ens in ensures {
                l0013_walk(ens, out);
            }
            l0013_walk(body, out);
        }
        Node::IfStatement {
            consequence,
            alternative,
            condition,
            ..
        } => {
            l0013_walk(condition, out);
            l0013_walk(consequence, out);
            if let Some(alt) = alternative {
                l0013_walk(alt, out);
            }
        }
        Node::WhileStatement {
            body, condition, ..
        } => {
            l0013_walk(condition, out);
            l0013_walk(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            l0013_walk(iterable, out);
            l0013_walk(body, out);
        }
        Node::LiveBlock { body, .. } => {
            l0013_walk(body, out);
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            l0013_walk(scrutinee, out);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    l0013_walk(g, out);
                }
                l0013_walk(body, out);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            l0013_walk(left, out);
            l0013_walk(right, out);
        }
        Node::PrefixExpression { right, .. } => {
            l0013_walk(right, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            l0013_walk(function, out);
            for arg in arguments {
                l0013_walk(arg, out);
            }
        }
        Node::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                l0013_walk(value, out);
            }
        }
        Node::ReturnStatement {
            value: Some(val), ..
        } => {
            l0013_walk(val, out);
        }
        Node::ReturnStatement { value: None, .. } => {}
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                l0013_walk(method, out);
            }
        }
        Node::FieldAccess { target, .. } => {
            l0013_walk(target, out);
        }
        Node::ExpressionStatement { expr, .. } => {
            l0013_walk(expr, out);
        }
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                l0013_walk(stmt, out);
            }
            for (_, handler_body) in handlers {
                for stmt in handler_body {
                    l0013_walk(stmt, out);
                }
            }
        }
        Node::ArrayLiteral { items, .. } => {
            for item in items {
                l0013_walk(item, out);
            }
        }
        _ => {}
    }
}

// L0014: function defined but never called (dead function)
//
// Collects every top-level function name and every call-target
// identifier anywhere in the program.  Any function that was defined
// but whose name never appears as a callee is warned.
//
// Exceptions:
// * `_`-prefixed names (silenced by convention, same as L0001/L0011).
// * Names that appear as identifiers outside of call position (e.g.
//   passed as higher-order values) are treated as "used" — the lint
//   focuses on the unambiguous dead-function case.
fn run_l0014_unused_function(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };

    // Phase 1: collect (name, span) for every top-level fn definition.
    let mut defined: Vec<(&str, Span)> = Vec::new();
    for spanned in stmts {
        if let Node::Function { name, span, .. } = &spanned.node {
            defined.push((name.as_str(), *span));
        }
    }
    if defined.is_empty() {
        return;
    }

    // Phase 2: collect every identifier that appears as a call target
    // anywhere in the program (including top-level call statements).
    let mut called: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(defined.len());
    for spanned in stmts {
        l0014_collect_calls(&spanned.node, &mut called);
    }

    // Phase 3: warn for each defined fn whose name was never called.
    for (name, span) in defined {
        if name.starts_with('_') {
            continue;
        }
        if !called.contains(name) {
            out.push(Lint {
                code: "L0014".into(),
                severity: Severity::Warning,
                message: format!(
                    "function `{}` is defined but never called — prefix with `_` to silence",
                    name
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
}

/// Recursively collect all call-target identifiers in `node`.
fn l0014_collect_calls<'a>(node: &'a Node, out: &mut std::collections::HashSet<&'a str>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                out.insert(name.as_str());
            }
            // Recurse into function expression itself (handles chained calls,
            // method dispatch, etc.) and into all arguments.
            l0014_collect_calls(function, out);
            for a in arguments {
                l0014_collect_calls(a, out);
            }
        }
        Node::Function { body, .. } => l0014_collect_calls(body, out),
        Node::Block { stmts, .. } => {
            for s in stmts {
                l0014_collect_calls(s, out);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. } => l0014_collect_calls(value, out),
        Node::ReturnStatement { value: Some(v), .. } => l0014_collect_calls(v, out),
        Node::ExpressionStatement { expr, .. } => l0014_collect_calls(expr, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            l0014_collect_calls(condition, out);
            l0014_collect_calls(consequence, out);
            if let Some(e) = alternative {
                l0014_collect_calls(e, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            l0014_collect_calls(condition, out);
            l0014_collect_calls(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            l0014_collect_calls(iterable, out);
            l0014_collect_calls(body, out);
        }
        Node::InfixExpression { left, right, .. } => {
            l0014_collect_calls(left, out);
            l0014_collect_calls(right, out);
        }
        Node::PrefixExpression { right, .. } => l0014_collect_calls(right, out),
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                l0014_collect_calls(i, out);
            }
        }
        Node::FieldAccess { target, .. } => l0014_collect_calls(target, out),
        Node::FieldAssignment { target, value, .. } => {
            l0014_collect_calls(target, out);
            l0014_collect_calls(value, out);
        }
        Node::IndexExpression { target, index, .. } => {
            l0014_collect_calls(target, out);
            l0014_collect_calls(index, out);
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                l0014_collect_calls(m, out);
            }
        }
        _ => {}
    }
}

// ============================================================
// L0015: constant arithmetic expression overflows `int`
// ============================================================
//
// Fires when every operand of an arithmetic infix expression is a
// compile-time-known integer literal and the operation overflows
// signed 64-bit integer range.  Division/modulo by zero is already
// covered by L0009 and is not re-reported here.

fn run_l0015_const_overflow(program: &Node, out: &mut Vec<Lint>) {
    walk_l0015(program, out);
}

/// Try to evaluate `node` to a compile-time constant `i64`.
/// Returns `None` on any free identifier, function call, or
/// arithmetic overflow (so the caller can detect the overflow case
/// separately).
fn try_const_int(node: &Node) -> Option<i64> {
    match node {
        Node::IntegerLiteral { value, .. } => Some(*value),
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => try_const_int(right).and_then(i64::checked_neg),
        Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } => {
            let l = try_const_int(left)?;
            let r = try_const_int(right)?;
            match operator.as_str() {
                "+" => l.checked_add(r),
                "-" => l.checked_sub(r),
                "*" => l.checked_mul(r),
                "/" => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_div(r)
                    }
                }
                "%" => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_rem(r)
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn walk_l0015(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
    } = node
    {
        let op = operator.as_str();
        if matches!(op, "+" | "-" | "*" | "/" | "%") {
            let l_val = try_const_int(left);
            let r_val = try_const_int(right);
            if let (Some(l), Some(r)) = (l_val, r_val) {
                let overflows = match op {
                    "+" => l.checked_add(r).is_none(),
                    "-" => l.checked_sub(r).is_none(),
                    "*" => l.checked_mul(r).is_none(),
                    // div/rem by zero → L0009, not L0015
                    "/" => r != 0 && l.checked_div(r).is_none(),
                    "%" => r != 0 && l.checked_rem(r).is_none(),
                    _ => false,
                };
                if overflows {
                    out.push(Lint {
                        code: "L0015".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "constant expression `{l} {op} {r}` overflows `int` — \
                             use smaller values or suppress with \
                             `// resilient: allow L0015`"
                        ),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                    return;
                }
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0015(child, out));
}

// ============================================================
// L0016: constant boolean condition in `if` statement
// ============================================================
//
// Fires when the condition of an `if` is a compile-time constant
// (`true`, `false`, or a fully-folded boolean expression).  This
// catches dead branches (`if false { ... }`) and tautological ones
// (`if true { ... }`) that should be simplified or removed.

fn run_l0016_constant_condition(program: &Node, out: &mut Vec<Lint>) {
    walk_l0016(program, out);
}

fn try_const_bool(node: &Node) -> Option<bool> {
    match node {
        Node::BooleanLiteral { value, .. } => Some(*value),
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "!" => try_const_bool(right).map(|v| !v),
        Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } => match operator.as_str() {
            "==" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l == r)
            }
            "!=" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l != r)
            }
            "<" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l < r)
            }
            ">" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l > r)
            }
            "<=" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l <= r)
            }
            ">=" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l >= r)
            }
            "&&" => match (try_const_bool(left), try_const_bool(right)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            "||" => match (try_const_bool(left), try_const_bool(right)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn walk_l0016(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        condition, span, ..
    } = node
        && let Some(val) = try_const_bool(condition)
    {
        let branch = if val { "always taken" } else { "never taken" };
        out.push(Lint {
            code: "L0016".into(),
            severity: Severity::Warning,
            message: format!(
                "condition is always `{val}` — this branch is {branch}; \
                 simplify or suppress with `// resilient: allow L0016`"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0016(child, out));
}

// ============================================================
// L0017: variable shadowing
// ============================================================
//
// Fires when a `let` binding in an inner scope uses the same name
// as a binding in any enclosing scope (parameters or outer let).
// Names starting with `_` are exempt — the leading underscore is
// the conventional "I know this shadows" signal.

fn run_l0017_variable_shadowing(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function {
                parameters, body, ..
            } => {
                let mut scopes: Vec<std::collections::HashSet<String>> =
                    vec![parameters.iter().map(|(_, name)| name.clone()).collect()];
                l0017_walk(body, &mut scopes, out);
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function {
                        parameters, body, ..
                    } = method
                    {
                        let mut scopes: Vec<std::collections::HashSet<String>> =
                            vec![parameters.iter().map(|(_, name)| name.clone()).collect()];
                        l0017_walk(body, &mut scopes, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0017_walk(
    node: &Node,
    scopes: &mut Vec<std::collections::HashSet<String>>,
    out: &mut Vec<Lint>,
) {
    match node {
        Node::Block { stmts, .. } => {
            scopes.push(std::collections::HashSet::new());
            for stmt in stmts {
                l0017_walk(stmt, scopes, out);
            }
            scopes.pop();
        }
        Node::LetStatement {
            name, value, span, ..
        } => {
            if !name.starts_with('_') {
                let outer_len = scopes.len().saturating_sub(1);
                let shadows = scopes[..outer_len]
                    .iter()
                    .any(|s| s.contains(name.as_str()));
                if shadows {
                    out.push(Lint {
                        code: "L0017".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "variable `{}` shadows a previous declaration — \
                             rename to avoid confusion, or prefix with `_` to silence",
                            name
                        ),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                }
            }
            if let Some(top) = scopes.last_mut() {
                top.insert(name.clone());
            }
            l0017_walk(value, scopes, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            l0017_walk(condition, scopes, out);
            l0017_walk(consequence, scopes, out);
            if let Some(alt) = alternative {
                l0017_walk(alt, scopes, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            l0017_walk(condition, scopes, out);
            l0017_walk(body, scopes, out);
        }
        Node::ForInStatement { body, iterable, .. } => {
            l0017_walk(iterable, scopes, out);
            l0017_walk(body, scopes, out);
        }
        Node::ReturnStatement { value, .. } => {
            if let Some(v) = value {
                l0017_walk(v, scopes, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            l0017_walk(expr, scopes, out);
        }
        Node::Assignment { value, .. } => {
            l0017_walk(value, scopes, out);
        }
        // Nested function definitions have independent scopes; don't
        // carry the outer scope stack into them.
        Node::Function { .. } => {}
        _ => {
            recurse_children(node, &mut |child| l0017_walk(child, scopes, out));
        }
    }
}

// ============================================================
// L0018: missing return on all paths
// ============================================================
//
// Fires for functions with an explicit `-> TYPE` annotation (where
// TYPE is not `void`) whose body does not return on every code path.
// Heuristic: a block "returns on all paths" when its last statement
// is a `return`, or is an `if/else` where both branches return.
// A function with no else clause, or that falls off the end of its
// body, gets a warning.

fn run_l0018_missing_return(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function {
                name,
                return_type,
                body,
                span,
                ..
            } => {
                if let Some(rt) = return_type
                    && !l0018_is_void(rt)
                    && !l0018_all_paths_return(body)
                {
                    out.push(Lint {
                        code: "L0018".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "function `{}` has return type `{}` but may not return \
                             on all paths — add a `return` or suppress with \
                             `// resilient: allow L0018`",
                            name, rt
                        ),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                }
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function {
                        name,
                        return_type,
                        body,
                        span,
                        ..
                    } = method
                        && let Some(rt) = return_type
                        && !l0018_is_void(rt)
                        && !l0018_all_paths_return(body)
                    {
                        out.push(Lint {
                            code: "L0018".into(),
                            severity: Severity::Warning,
                            message: format!(
                                "function `{}` has return type `{}` but may not \
                                 return on all paths",
                                name, rt
                            ),
                            line: span.start.line as u32,
                            column: span.start.column as u32,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0018_is_void(rt: &str) -> bool {
    matches!(rt.trim(), "void" | "()" | "")
}

/// Returns `true` when `node` is guaranteed to produce a value (explicit
/// `return` or implicit expression result) on every execution path through it.
/// Conservative — never false-positive.
fn l0018_all_paths_return(node: &Node) -> bool {
    match node {
        Node::ReturnStatement { .. } => true,
        // An expression statement at the tail of a block is Resilient's
        // implicit-return form (same as Rust's expression-oriented blocks).
        // `fn f() -> int { a + b }` is valid — `a + b` IS the return value.
        Node::ExpressionStatement { .. } => true,
        Node::Block { stmts, .. } => {
            // A block returns on all paths if any statement in it does (once
            // a return is reached, subsequent stmts are unreachable).
            stmts.iter().any(l0018_all_paths_return)
        }
        // Returns on all paths only when both branches cover all paths.
        // No `else` means the `if`-false path falls through.
        Node::IfStatement {
            consequence,
            alternative: Some(alt),
            ..
        } => l0018_all_paths_return(consequence) && l0018_all_paths_return(alt),
        Node::IfStatement {
            alternative: None, ..
        } => false,
        // A while/for loop body might not execute at all, so it doesn't
        // guarantee a return.
        Node::WhileStatement { .. } | Node::ForInStatement { .. } => false,
        _ => false,
    }
}

// ============================================================
// L0019: format() argument count mismatch
// ============================================================
//
// `format(template, args_array)` takes exactly two arguments.
// Fires when:
//   (a) The call has != 2 arguments, OR
//   (b) The template is a static string (no runtime interpolation)
//       and args is an array literal, and the placeholder count
//       doesn't match the array length.
//
// Notes on AST shape:
//   - A Resilient template like `"\{} \{}"` stores `\{` as an
//     "unknown escape" in the lexer; string_interp's parse_parts
//     converts `\{` → `{`, so the InterpolatedString's Literal
//     parts already contain `{}` as the placeholder text.
//   - A template with no braces (e.g. `"hello"`) is a plain
//     StringLiteral; parse_template sees no placeholders.
//   - Templates with runtime interpolation (`"{expr}"`) have
//     Expr parts — arity is not statically checkable.

fn run_l0019_format_arity(program: &Node, out: &mut Vec<Lint>) {
    walk_l0019(program, out);
}

/// Extract the concatenated literal text from a template node, if it
/// has no runtime-interpolation `Expr` parts.
fn l0019_literal_template(node: &Node) -> Option<String> {
    match node {
        Node::StringLiteral { value, .. } => Some(value.clone()),
        Node::InterpolatedString { parts, .. } => {
            if parts
                .iter()
                .all(|p| matches!(p, crate::string_interp::StringPart::Literal(_)))
            {
                Some(
                    parts
                        .iter()
                        .map(|p| match p {
                            crate::string_interp::StringPart::Literal(s) => s.as_str(),
                            _ => "",
                        })
                        .collect(),
                )
            } else {
                None
            }
        }
        _ => None,
    }
}

fn walk_l0019(node: &Node, out: &mut Vec<Lint>) {
    if let Node::CallExpression {
        function,
        arguments,
        span,
    } = node
        && let Node::Identifier { name, .. } = function.as_ref()
        && name == "format"
    {
        if arguments.len() != 2 {
            out.push(Lint {
                code: "L0019".into(),
                severity: Severity::Warning,
                message: format!(
                    "format() requires exactly 2 arguments (template, args_array) but {} {} supplied \
                     — suppress with `// resilient: allow L0019`",
                    arguments.len(),
                    if arguments.len() == 1 { "was" } else { "were" },
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        } else if let Some(tmpl) = l0019_literal_template(&arguments[0])
            && let Node::ArrayLiteral { items, .. } = &arguments[1]
            && let Ok(segments) = crate::format_builtin::parse_template(&tmpl)
        {
            let placeholders = segments
                .iter()
                .filter(|s| matches!(s, crate::format_builtin::FormatSegment::Placeholder(_)))
                .count();
            let array_len = items.len();
            if placeholders != array_len {
                out.push(Lint {
                    code: "L0019".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "format() template has {} placeholder{} but args array has {} element{} \
                         — counts must match; suppress with `// resilient: allow L0019`",
                        placeholders,
                        if placeholders == 1 { "" } else { "s" },
                        array_len,
                        if array_len == 1 { "" } else { "s" },
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0019(child, out));
}

// ============================================================
// L0020: unused function parameter
// ============================================================
//
// For each `fn`, collect parameter names and check whether each
// appears in the body (or in `requires`/`ensures` clauses).
// `_`-prefixed params are intentionally silenced by convention.
// Parameters that appear only in `requires`/`ensures` (pre/post-
// conditions) are considered used — they constrain the contract
// even if the body doesn't directly reference them.

fn run_l0020_unused_parameter(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function {
                parameters,
                body,
                requires,
                ensures,
                span,
                ..
            } => {
                l0020_check_params(parameters, body, requires, ensures, span, out);
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function {
                        parameters,
                        body,
                        requires,
                        ensures,
                        span,
                        ..
                    } = method
                    {
                        l0020_check_params(parameters, body, requires, ensures, span, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0020_check_params(
    parameters: &[(String, String)],
    body: &Node,
    requires: &[Node],
    ensures: &[Node],
    fn_span: &Span,
    out: &mut Vec<Lint>,
) {
    if parameters.is_empty() {
        return;
    }
    let mut used: std::collections::HashSet<&str> = std::collections::HashSet::new();
    collect_identifier_reads_in(body, &mut used);
    for req in requires {
        collect_identifier_reads_in(req, &mut used);
    }
    for ens in ensures {
        collect_identifier_reads_in(ens, &mut used);
    }
    for (_ty, pname) in parameters {
        if pname.starts_with('_') {
            continue;
        }
        if !used.contains(pname.as_str()) {
            out.push(Lint {
                code: "L0020".into(),
                severity: Severity::Warning,
                message: format!("unused parameter `{}` — prefix with `_` to silence", pname),
                line: fn_span.start.line as u32,
                column: fn_span.start.column as u32,
            });
        }
    }
}

// ============================================================
// L0021: redundant boolean sub-expression (x && x, x || x)
// ============================================================
//
// Detects infix `&&` or `||` where both operands are structurally
// identical (same identifier or same literal). The always-true
// tautology `x || x` and always-redundant `x && x` are bugs or
// dead code. This extends L0003 (which catches `x == x`) to
// logical operators.

fn run_l0021_redundant_bool_subexpr(program: &Node, out: &mut Vec<Lint>) {
    walk_l0021(program, out);
}

fn walk_l0021(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && (operator == "&&" || operator == "||")
        && nodes_structurally_equal(left, right)
    {
        out.push(Lint {
            code: "L0021".into(),
            severity: Severity::Warning,
            message: format!(
                "redundant sub-expression: both sides of `{}` are identical",
                operator
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0021(child, out));
}

/// Shallow structural equality for L0021.
/// Two nodes are equal if they are:
/// - The same `Identifier` (same name)
/// - The same `IntegerLiteral` / `FloatLiteral` / `BooleanLiteral`
/// - The same `StringLiteral`
/// - The same `PrefixExpression` with equal operands
/// - The same `InfixExpression` with equal operands
fn nodes_structurally_equal(a: &Node, b: &Node) -> bool {
    match (a, b) {
        (Node::Identifier { name: na, .. }, Node::Identifier { name: nb, .. }) => na == nb,
        (Node::IntegerLiteral { value: va, .. }, Node::IntegerLiteral { value: vb, .. }) => {
            va == vb
        }
        (Node::FloatLiteral { value: va, .. }, Node::FloatLiteral { value: vb, .. }) => {
            va.to_bits() == vb.to_bits()
        }
        (Node::BooleanLiteral { value: va, .. }, Node::BooleanLiteral { value: vb, .. }) => {
            va == vb
        }
        (Node::StringLiteral { value: va, .. }, Node::StringLiteral { value: vb, .. }) => va == vb,
        (
            Node::PrefixExpression {
                operator: oa,
                right: ra,
                ..
            },
            Node::PrefixExpression {
                operator: ob,
                right: rb,
                ..
            },
        ) => oa == ob && nodes_structurally_equal(ra, rb),
        (
            Node::InfixExpression {
                left: la,
                operator: oa,
                right: ra,
                ..
            },
            Node::InfixExpression {
                left: lb,
                operator: ob,
                right: rb,
                ..
            },
        ) => oa == ob && nodes_structurally_equal(la, lb) && nodes_structurally_equal(ra, rb),
        _ => false,
    }
}

// ============================================================
// L0022: needless else after unconditional return
// ============================================================
//
// Detects `if cond { return x; } else { ... }` where the
// consequence block always returns. The `else` keyword is
// redundant because control flow after the if-block already
// implies the condition was false. Removing the `else` and
// de-indenting the body is cleaner and avoids confusion.

fn run_l0022_needless_else(program: &Node, out: &mut Vec<Lint>) {
    walk_l0022(program, out);
}

fn walk_l0022(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        consequence,
        alternative: Some(alt),
        span,
        ..
    } = node
    {
        if l0018_all_paths_return(consequence) {
            out.push(Lint {
                code: "L0022".into(),
                severity: Severity::Warning,
                message: "else block is redundant after a block that always returns; \
                          remove the `else` and de-indent the body"
                    .into(),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
        // Still recurse into the alternative since nested ifs can also trigger.
        walk_l0022(alt, out);
    }
    recurse_children(node, &mut |child| walk_l0022(child, out));
}

// ============================================================
// L0023: tautological comparison with boolean literal
// ============================================================
//
// Detects `expr == true`, `expr == false`, `true == expr`,
// `false == expr`. These comparisons are always redundant:
// - `x == true`  → use `x` directly
// - `x == false` → use `!x`
// The reversed forms (literal on the left) are also caught.

fn run_l0023_bool_literal_comparison(program: &Node, out: &mut Vec<Lint>) {
    walk_l0023(program, out);
}

fn walk_l0023(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && operator == "=="
    {
        let (bool_val, other_side) = if let Node::BooleanLiteral { value, .. } = left.as_ref() {
            (Some(*value), right.as_ref())
        } else if let Node::BooleanLiteral { value, .. } = right.as_ref() {
            (Some(*value), left.as_ref())
        } else {
            (None, left.as_ref())
        };

        if let Some(literal) = bool_val {
            // Skip `true == true` / `false == false` (caught by L0003 or trivially obvious).
            if matches!(other_side, Node::BooleanLiteral { .. }) {
                // Let L0003 handle identical-operand case.
            } else {
                let suggestion = if literal {
                    "use the expression directly instead of `== true`"
                } else {
                    "use `!expr` instead of `== false`"
                };
                out.push(Lint {
                    code: "L0023".into(),
                    severity: Severity::Warning,
                    message: format!("tautological comparison with `{}`; {}", literal, suggestion),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0023(child, out));
}

// ============================================================
// L0025: unreachable code after infinite while-true loop
// ============================================================
//
// A `while true { ... }` loop that never `break`s or `return`s
// from the enclosing function makes all subsequent statements
// in the same block unreachable. Extends L0007 (unreachable
// after explicit `return`) to cover the loop variant.

fn run_l0025_unreachable_after_infinite_loop(program: &Node, out: &mut Vec<Lint>) {
    walk_l0025(program, out);
}

fn walk_l0025(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Block { stmts, .. } = node {
        let mut found_infinite = false;
        for stmt in stmts {
            if found_infinite {
                if let Some(span) = node_span(stmt) {
                    out.push(Lint {
                        code: "L0025".into(),
                        severity: Severity::Warning,
                        message: "unreachable code after infinite `while true` loop".into(),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                }
                // Only report the first unreachable statement (same as L0007).
                break;
            }
            if is_infinite_while(stmt) {
                found_infinite = true;
            }
            walk_l0025(stmt, out);
        }
        if !found_infinite {
            for stmt in stmts {
                walk_l0025(stmt, out);
            }
        }
    } else {
        recurse_children(node, &mut |child| walk_l0025(child, out));
    }
}

/// True when `node` is a `while true { ... }` loop whose body
/// never breaks out via `break` (returns are fine — they exit the
/// whole function, making *everything* after the loop unreachable).
fn is_infinite_while(node: &Node) -> bool {
    let Node::WhileStatement {
        condition, body, ..
    } = node
    else {
        return false;
    };
    if !matches!(condition.as_ref(), Node::BooleanLiteral { value: true, .. }) {
        return false;
    }
    !l0025_body_has_break(body)
}

fn l0025_body_has_break(node: &Node) -> bool {
    match node {
        Node::Break { .. } => true,
        // Don't cross function boundaries.
        Node::Function { .. } => false,
        _ => {
            let mut found = false;
            recurse_children(node, &mut |child| {
                if !found {
                    found = l0025_body_has_break(child);
                }
            });
            found
        }
    }
}

/// Extract the source span from common statement nodes (best-effort).
fn node_span(node: &Node) -> Option<&Span> {
    match node {
        Node::LetStatement { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::ExpressionStatement { span, .. }
        | Node::Assignment { span, .. } => Some(span),
        _ => None,
    }
}

// ============================================================
// L0024: struct literal missing required fields
// ============================================================
//
// Collects all `StructDecl` definitions visible at program scope,
// then walks every `StructLiteral` and warns when a declared field
// is absent from the literal. This is a lint-level warning (the
// typechecker will also error); the lint fires first and lists the
// missing names so the user can see at a glance what to add.

fn run_l0024_struct_missing_fields(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    // Build struct-name → declared field names from top-level decls.
    let mut decls: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for spanned in stmts {
        if let Node::StructDecl { name, fields, .. } = &spanned.node {
            // `fields` is Vec<(type_name, field_name)>
            decls.insert(
                name.as_str(),
                fields.iter().map(|(_, fname)| fname.as_str()).collect(),
            );
        }
        // Descend into impl blocks — they don't contain StructDecls but
        // let the struct-collection pass stay consistent.
    }
    if decls.is_empty() {
        return;
    }
    walk_l0024(program, &decls, out);
}

fn walk_l0024<'a>(
    node: &'a Node,
    decls: &std::collections::HashMap<&str, Vec<&'a str>>,
    out: &mut Vec<Lint>,
) {
    if let Node::StructLiteral { name, fields, span } = node
        && let Some(declared) = decls.get(name.as_str())
    {
        let provided: std::collections::HashSet<&str> =
            fields.iter().map(|(fname, _)| fname.as_str()).collect();
        let missing: Vec<&str> = declared
            .iter()
            .filter(|f| !provided.contains(**f))
            .copied()
            .collect();
        if !missing.is_empty() {
            let list = missing
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push(Lint {
                code: "L0024".into(),
                severity: Severity::Warning,
                message: format!("struct literal `{name}` is missing required field(s): {list}"),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
    recurse_children(node, &mut |child| walk_l0024(child, decls, out));
}

// ============================================================
// L0026: duplicate literal key in map literal
// ============================================================
//
// When a map literal like `{ "a": 1, "b": 2, "a": 3 }` contains
// two entries with the same literal key, the first is silently
// overwritten at runtime. This is almost always a copy-paste
// mistake and never intentional.
//
// Only literal keys (string, integer, bool) are checked — dynamic
// expression keys can't be compared at lint time.

fn run_l0026_duplicate_map_key(program: &Node, out: &mut Vec<Lint>) {
    walk_l0026(program, out);
}

fn walk_l0026(node: &Node, out: &mut Vec<Lint>) {
    if let Node::MapLiteral { entries, span } = node {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (key, _) in entries {
            let repr = match key {
                Node::StringLiteral { value, .. } => Some(format!("\"{value}\"")),
                Node::IntegerLiteral { value, .. } => Some(value.to_string()),
                Node::BooleanLiteral { value, .. } => Some(value.to_string()),
                _ => None,
            };
            if let Some(k) = repr
                && !seen.insert(k.clone())
            {
                out.push(Lint {
                    code: "L0026".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "duplicate map key {k} — the earlier binding is silently overwritten"
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0026(child, out));
}

// ============================================================
// L0027: empty catch block silently swallows errors
// ============================================================
//
// An empty `catch` arm (`catch (E) { }`) silently discards the
// error. Code that intentionally swallows should add a comment
// or a `let _e = ...` binding; this lint surfaces the pattern
// so it's visible during review.

fn run_l0027_empty_catch_block(program: &Node, out: &mut Vec<Lint>) {
    walk_l0027(program, out);
}

fn walk_l0027(node: &Node, out: &mut Vec<Lint>) {
    if let Node::TryCatch { handlers, span, .. } = node {
        for (error_type, body) in handlers {
            if body.is_empty() {
                out.push(Lint {
                    code: "L0027".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "empty catch block for `{error_type}` silently discards the error — add a handler or re-raise"
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0027(child, out));
}

// ============================================================
// L0028: negation of boolean literal (`!true` / `!false`)
// ============================================================
//
// `!true` always evaluates to `false` and `!false` always evaluates
// to `true`. Using the negated literal instead of the result literal
// is confusing and almost always indicates a logic error.

fn run_l0028_negation_of_literal(program: &Node, out: &mut Vec<Lint>) {
    walk_l0028(program, out);
}

fn walk_l0028(node: &Node, out: &mut Vec<Lint>) {
    if let Node::PrefixExpression {
        operator,
        right,
        span,
    } = node
        && operator == "!"
        && let Node::BooleanLiteral { value, .. } = right.as_ref()
    {
        let result = if *value { "false" } else { "true" };
        let literal = if *value { "true" } else { "false" };
        out.push(Lint {
            code: "L0028".into(),
            severity: Severity::Warning,
            message: format!("`!{literal}` is always `{result}` — use `{result}` directly"),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0028(child, out));
}

// ============================================================
// L0029: comparison result discarded as statement
// ============================================================
//
// An expression statement like `a == b;` computes a boolean but
// immediately discards the result. This is almost always a typo
// for an assignment (`a = b;`) or a missed assertion
// (`assert(a == b);`). For safety-critical code this pattern is
// particularly dangerous because a postcondition check silently
// becomes a no-op.

fn run_l0029_comparison_result_discarded(program: &Node, out: &mut Vec<Lint>) {
    walk_l0029(program, out);
}

fn walk_l0029(node: &Node, out: &mut Vec<Lint>) {
    if let Node::ExpressionStatement { expr, span } = node
        && let Node::InfixExpression { operator, .. } = expr.as_ref()
        && matches!(operator.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=")
    {
        out.push(Lint {
            code: "L0029".into(),
            severity: Severity::Warning,
            message: format!(
                "comparison `{operator}` result is discarded — did you mean `assert(…)` or `=`?"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0029(child, out));
}

// ============================================================
// L0030: float equality comparison (`==` / `!=`)
// ============================================================
//
// Comparing floats with `==` or `!=` is almost always a bug in
// safety-critical embedded code: floating-point arithmetic
// accumulates rounding error, so two computations that are
// mathematically equal will often produce different bit patterns.
// Use an epsilon comparison: `abs(a - b) < epsilon`.
//
// We fire only when at least one operand is a float literal; this
// covers the most common patterns (`x == 0.0`, `result != 1.5`)
// without requiring full type inference on both operands.

fn run_l0030_float_equality(program: &Node, out: &mut Vec<Lint>) {
    walk_l0030(program, out);
}

fn walk_l0030(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && (operator == "==" || operator == "!=")
    {
        let left_is_float = matches!(left.as_ref(), Node::FloatLiteral { .. });
        let right_is_float = matches!(right.as_ref(), Node::FloatLiteral { .. });
        if left_is_float || right_is_float {
            out.push(Lint {
                code: "L0030".into(),
                severity: Severity::Warning,
                message: format!(
                    "float equality comparison `{operator}` is almost always a bug — \
                     use an epsilon comparison: `abs(a - b) < epsilon`"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
    recurse_children(node, &mut |child| walk_l0030(child, out));
}

// ============================================================
// L0031: double negation `!!x`
// ============================================================
//
// `!!x` is semantically identical to `x` for any boolean `x`.
// The double negation is redundant and obscures intent; replace
// with the un-negated expression.

fn run_l0031_double_negation(program: &Node, out: &mut Vec<Lint>) {
    walk_l0031(program, out);
}

fn walk_l0031(node: &Node, out: &mut Vec<Lint>) {
    if let Node::PrefixExpression {
        operator,
        right,
        span,
    } = node
        && operator == "!"
        && let Node::PrefixExpression {
            operator: inner_op, ..
        } = right.as_ref()
        && inner_op == "!"
    {
        out.push(Lint {
            code: "L0031".into(),
            severity: Severity::Warning,
            message: "double negation `!!x` is redundant — use `x` directly".to_string(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0031(child, out));
}

// ============================================================
// L0032: assignment used as boolean condition
// ============================================================
//
// `if x = value { }` computes the assignment and uses the result
// as the condition. This is almost always a typo for `if x == value`.
// In safety-critical code this silently changes the branch predicate.

fn run_l0032_assign_in_condition(program: &Node, out: &mut Vec<Lint>) {
    walk_l0032(program, out);
}

fn walk_l0032(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        condition, span, ..
    } = node
        && let Node::Assignment { name, .. } = condition.as_ref()
    {
        out.push(Lint {
            code: "L0032".into(),
            severity: Severity::Warning,
            message: format!(
                "assignment to `{name}` used as boolean condition — did you mean `{name} ==`?"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0032(child, out));
}

// ============================================================
// L0033: integer modulo by literal 1 (always 0)
// ============================================================
//
// `x % 1` is always 0 for any integer x. This is almost certainly
// a mistake — the programmer likely meant a different modulus.
// In embedded contexts the dead operation also wastes a clock cycle.

fn run_l0033_modulo_by_one(program: &Node, out: &mut Vec<Lint>) {
    walk_l0033(program, out);
}

fn walk_l0033(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        right,
        span,
        ..
    } = node
        && operator == "%"
        && matches!(right.as_ref(), Node::IntegerLiteral { value: 1, .. })
    {
        out.push(Lint {
            code: "L0033".into(),
            severity: Severity::Warning,
            message: "`x % 1` is always `0` — did you mean a different modulus?".to_string(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0033(child, out));
}

// ============================================================
// L0034: string concatenation with `+` inside a loop
// ============================================================
//
// Building a string by appending in a loop with `+` is O(N²) because
// each concatenation copies the entire accumulated string. In embedded
// systems with limited heap, this pattern can cause OOM. The fix is to
// accumulate parts in an array and join once outside the loop.
//
// We fire when at least one operand of a `+` inside a loop body is a
// string literal — the most common pattern (`result = result + chunk`
// where `chunk` or `result` was originally a string literal).

fn run_l0034_string_concat_in_loop(program: &Node, out: &mut Vec<Lint>) {
    walk_l0034_loop(program, false, out);
}

fn walk_l0034_loop(node: &Node, in_loop: bool, out: &mut Vec<Lint>) {
    match node {
        Node::WhileStatement { body, .. } => {
            walk_l0034_loop(body, true, out);
        }
        Node::ForInStatement { body, .. } => {
            walk_l0034_loop(body, true, out);
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } if in_loop && operator == "+" => {
            let left_is_str = matches!(left.as_ref(), Node::StringLiteral { .. });
            let right_is_str = matches!(right.as_ref(), Node::StringLiteral { .. });
            if left_is_str || right_is_str {
                out.push(Lint {
                    code: "L0034".into(),
                    severity: Severity::Warning,
                    message: "string concatenation `+` inside a loop is O(N²) — \
                              accumulate parts in an array and join outside the loop"
                        .to_string(),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
            // Recurse into operands with in_loop preserved.
            walk_l0034_loop(left, in_loop, out);
            walk_l0034_loop(right, in_loop, out);
        }
        // For other nodes: propagate in_loop flag to children via manual recursion.
        _ => {
            recurse_children(node, &mut |child| walk_l0034_loop(child, in_loop, out));
        }
    }
}

// ============================================================
// L0035: unreachable code after a diverging call (exit / abort)
// ============================================================
//
// A call to `exit()` or `abort()` never returns. Any statements in
// the same block after such a call are dead code. This is similar to
// L0007 (after `return`) but for diverging function calls.

fn run_l0035_unreachable_after_exit(program: &Node, out: &mut Vec<Lint>) {
    walk_l0035(program, out);
}

fn is_diverging_call(node: &Node) -> bool {
    let call = match node {
        Node::CallExpression { .. } => Some(node),
        Node::ExpressionStatement { expr, .. } => {
            if matches!(expr.as_ref(), Node::CallExpression { .. }) {
                Some(expr.as_ref())
            } else {
                None
            }
        }
        _ => None,
    };
    match call {
        Some(Node::CallExpression { function, .. }) => matches!(
            function.as_ref(),
            Node::Identifier { name, .. } if matches!(name.as_str(), "exit" | "abort")
        ),
        _ => false,
    }
}

fn walk_l0035(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Block { stmts, .. } = node {
        let mut saw_exit = false;
        for stmt in stmts {
            if saw_exit {
                if let Some(span) = span_of(stmt) {
                    out.push(Lint {
                        code: "L0035".into(),
                        severity: Severity::Warning,
                        message: "unreachable code after diverging call (`exit` / `abort`)"
                            .to_string(),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                }
                break;
            }
            if is_diverging_call(stmt) {
                saw_exit = true;
            }
            walk_l0035(stmt, out);
        }
    } else {
        recurse_children(node, &mut |child| walk_l0035(child, out));
    }
}

// ============================================================
// L0036: comparison of len(...) to negative literal
// ============================================================
//
// `len()` always returns a non-negative integer. Comparing it to a
// negative literal with `<`, `<=`, `==`, or `!=` yields a result
// that is always constant (false or true), making the branch dead.
//
// Patterns detected:
//   len(x) < 0       — always false
//   len(x) <= -1     — always false
//   len(x) == -N     — always false (N > 0)
//   0 > len(x)       — always false
//   -1 >= len(x)     — always false

fn run_l0036_len_negative_comparison(program: &Node, out: &mut Vec<Lint>) {
    walk_l0036(program, out);
}

fn is_len_call(node: &Node) -> bool {
    matches!(
        node,
        Node::CallExpression { function, .. }
        if matches!(function.as_ref(), Node::Identifier { name, .. } if name == "len")
    )
}

fn is_negative_int_literal(node: &Node) -> bool {
    match node {
        Node::IntegerLiteral { value, .. } => *value < 0,
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => {
            matches!(right.as_ref(), Node::IntegerLiteral { value, .. } if *value > 0)
        }
        _ => false,
    }
}

fn walk_l0036(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && matches!(operator.as_str(), "<" | "<=" | "==" | "!=" | ">" | ">=")
    {
        let matched = (is_len_call(left) && is_negative_int_literal(right))
            || (is_len_call(right) && is_negative_int_literal(left));
        if matched {
            out.push(Lint {
                code: "L0036".into(),
                severity: Severity::Warning,
                message: format!(
                    "comparison of `len(...)` to a negative literal — \
                     `len()` is always ≥ 0, so `len(...) {operator} <negative>` is always constant"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
    recurse_children(node, &mut |child| walk_l0036(child, out));
}

// ============================================================
// L0037: self-assignment `x = x` is a no-op
// ============================================================
//
// Assigning a variable to itself has no effect. The right-hand side
// should have been a different expression. This is almost always a
// typo (e.g. `x = y` where y was accidentally written as x).

fn run_l0037_self_assignment(program: &Node, out: &mut Vec<Lint>) {
    walk_l0037(program, out);
}

fn walk_l0037(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Assignment { name, value, span } = node
        && let Node::Identifier { name: rhs_name, .. } = value.as_ref()
        && name == rhs_name
    {
        out.push(Lint {
            code: "L0037".into(),
            severity: Severity::Warning,
            message: format!(
                "`{name} = {name}` is a self-assignment — did you mean a different right-hand side?"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0037(child, out));
}

// ============================================================
// L0038: panic!() call outside of #[cfg(test)] context
// ============================================================
//
// Resilient targets safety-critical embedded environments where
// panics are forbidden outside of test scaffolding. Any call to
// `panic(...)` in non-test production code is flagged.
//
// Detection: walk the AST looking for CallExpression nodes whose
// function is the identifier `panic`. Because the Resilient test
// infrastructure does not produce `#[cfg(test)]`-gated AST nodes
// (test code lives in separate files or is excluded from the main
// parse), every `panic()` visible to the lint pass is treated as
// production code.

fn run_l0038_panic_in_non_test(program: &Node, out: &mut Vec<Lint>) {
    walk_l0038(program, out);
}

fn walk_l0038(node: &Node, out: &mut Vec<Lint>) {
    if let Node::CallExpression { function, span, .. } = node
        && matches!(function.as_ref(), Node::Identifier { name, .. } if name == "panic")
    {
        out.push(Lint {
            code: "L0038".into(),
            severity: Severity::Warning,
            message: "`panic()` called in non-test code — panics are forbidden in \
                      safety-critical embedded systems; use a typed error return or `abort()`"
                .to_string(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0038(child, out));
}

// ============================================================
// L0039: unreachable code after call to @noreturn function
// ============================================================
//
// Functions annotated with a `// @noreturn` comment on the line
// immediately preceding their declaration never return to the
// caller. Any statements in the same block that follow a call to
// such a function are dead code.
//
// Detection:
//   1. Scan the source text for `// @noreturn` comment lines and
//      record the names of the function declarations on the
//      following line.
//   2. Walk every Block node; if a statement calls one of those
//      functions and there are statements after it, emit L0039 on
//      the first unreachable statement.

/// Collect the names of functions preceded by a `// @noreturn` comment.
fn collect_noreturn_functions(source: &str) -> std::collections::HashSet<String> {
    let mut noreturn_fns = std::collections::HashSet::new();
    let lines: Vec<&str> = source.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == "// @noreturn" || trimmed.starts_with("// @noreturn ") {
            // Look at the next non-blank line for a function declaration.
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            if j < lines.len() {
                let next = lines[j].trim();
                // Match `fn name(` or `@pure fn name(` etc.
                let fn_pos = next.find("fn ");
                if let Some(pos) = fn_pos {
                    let after_fn = next[pos + 3..].trim_start();
                    // Function name ends at the first non-identifier character.
                    let name_end = after_fn
                        .find(|c: char| !c.is_alphanumeric() && c != '_')
                        .unwrap_or(after_fn.len());
                    let fn_name = &after_fn[..name_end];
                    if !fn_name.is_empty() {
                        noreturn_fns.insert(fn_name.to_string());
                    }
                }
            }
        }
    }
    noreturn_fns
}

fn run_l0039_unreachable_after_noreturn(program: &Node, source: &str, out: &mut Vec<Lint>) {
    let noreturn_fns = collect_noreturn_functions(source);
    if noreturn_fns.is_empty() {
        return;
    }
    walk_l0039(program, &noreturn_fns, out);
}

fn is_noreturn_call(node: &Node, noreturn_fns: &std::collections::HashSet<String>) -> bool {
    let call = match node {
        Node::CallExpression { .. } => Some(node),
        Node::ExpressionStatement { expr, .. } => {
            if matches!(expr.as_ref(), Node::CallExpression { .. }) {
                Some(expr.as_ref())
            } else {
                None
            }
        }
        _ => None,
    };
    match call {
        Some(Node::CallExpression { function, .. }) => matches!(
            function.as_ref(),
            Node::Identifier { name, .. } if noreturn_fns.contains(name.as_str())
        ),
        _ => false,
    }
}

fn walk_l0039(node: &Node, noreturn_fns: &std::collections::HashSet<String>, out: &mut Vec<Lint>) {
    if let Node::Block { stmts, .. } = node {
        let mut saw_noreturn = false;
        for stmt in stmts {
            if saw_noreturn {
                if let Some(span) = span_of(stmt) {
                    out.push(Lint {
                        code: "L0039".into(),
                        severity: Severity::Warning,
                        message: "unreachable code after call to `@noreturn`-annotated function"
                            .to_string(),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                }
                break;
            }
            if is_noreturn_call(stmt, noreturn_fns) {
                saw_noreturn = true;
            }
            walk_l0039(stmt, noreturn_fns, out);
        }
        return;
    }
    recurse_children(node, &mut |child| walk_l0039(child, noreturn_fns, out));
}

// ============================================================
// L0040: magic number in safety-critical computation
// ============================================================
//
// An unnamed integer literal (other than 0, 1, or a power of two)
// used in an arithmetic expression (+, -, *, /, %) inside a function
// that has no `requires` or `ensures` contract is a "magic number".
//
// In safety-critical embedded code, all numeric constants must be
// named via `let` bindings so their intent is auditable during
// certification review. Functions without contracts are especially
// risky because there is no machine-checkable specification to
// cross-reference against.

fn run_l0040_magic_number(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function {
                requires,
                ensures,
                body,
                ..
            } => {
                let has_contract = !requires.is_empty() || !ensures.is_empty();
                if !has_contract {
                    walk_l0040_arith(body, out);
                }
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function {
                        requires,
                        ensures,
                        body,
                        ..
                    } = method
                    {
                        let has_contract = !requires.is_empty() || !ensures.is_empty();
                        if !has_contract {
                            walk_l0040_arith(body, out);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Returns true for integer values that are "trivial" in an
/// arithmetic context: 0, 1, -1, and powers of two up to 2^30.
fn is_trivial_int(value: i64) -> bool {
    if value == 0 || value == 1 || value == -1 {
        return true;
    }
    // Powers of two (positive): 2, 4, 8, 16, ...
    let abs = value.unsigned_abs();
    abs != 0 && abs.count_ones() == 1
}

/// Walk an expression tree rooted at `node`. Whenever we find an
/// arithmetic infix expression that contains a non-trivial integer
/// literal as a *direct* operand, emit L0040.
fn walk_l0040_arith(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        ..
    } = node
        && matches!(operator.as_str(), "+" | "-" | "*" | "/" | "%")
    {
        // Check left operand.
        if let Node::IntegerLiteral { value, span } = left.as_ref()
            && !is_trivial_int(*value)
        {
            out.push(Lint {
                code: "L0040".into(),
                severity: Severity::Warning,
                message: format!(
                    "magic number `{value}` in arithmetic — extract into a named constant"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
        // Check right operand.
        if let Node::IntegerLiteral { value, span } = right.as_ref()
            && !is_trivial_int(*value)
        {
            out.push(Lint {
                code: "L0040".into(),
                severity: Severity::Warning,
                message: format!(
                    "magic number `{value}` in arithmetic — extract into a named constant"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
    recurse_children(node, &mut |child| walk_l0040_arith(child, out));
}

// ============================================================
// L0041: redundant `else` when the `if` arm always returns
// ============================================================
//
// Fires on:
//   if cond { return x; } else { ... }
//
// When the consequence block always returns (ReturnStatement or
// expression-tail), the `else` branch is unreachable from above —
// any code in `else` could be written at the same indentation level
// without an `else`. This is a stricter cousin of L0022 and shares
// the `l0018_all_paths_return` helper.
//
// L0022 fires on all `if/else` nodes where the consequence returns;
// L0041 is distinct in naming and wording but also uses that predicate.
// We keep them as separate codes so users can silence one without
// silencing the other.

fn run_l0041_redundant_else(program: &Node, out: &mut Vec<Lint>) {
    walk_l0041(program, out);
}

fn walk_l0041(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        consequence,
        alternative: Some(_alt),
        span,
        ..
    } = node
        && l0018_all_paths_return(consequence)
    {
        out.push(Lint {
            code: "L0041".into(),
            severity: Severity::Warning,
            message: "`else` block is redundant — the `if` arm always returns; \
                      de-nest the body and drop the `else`"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0041(child, out));
}

// ============================================================
// L0042: dead code after `return` in the same block
// ============================================================
//
// Fires on blocks where a `ReturnStatement` is followed by one or
// more statements. Those trailing statements can never execute.
//
// Unlike L0007 (unreachable_code, which fires on the *second*
// statement after a return), L0042 fires on the `return` statement
// itself (pointing at the culprit, not the victim). The code range
// is the same node; having two codes lets users suppress one without
// silencing the other.

fn run_l0042_dead_code_after_return(program: &Node, out: &mut Vec<Lint>) {
    walk_l0042(program, out);
}

fn walk_l0042(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Block { stmts, .. } = node {
        for (i, stmt) in stmts.iter().enumerate() {
            if matches!(stmt, Node::ReturnStatement { .. }) && i + 1 < stmts.len() {
                let (line, column) = node_span(stmt)
                    .map(|s| (s.start.line as u32, s.start.column as u32))
                    .unwrap_or((0, 0));
                out.push(Lint {
                    code: "L0042".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "statements after `return` in the same block are dead code ({} unreachable statement{})",
                        stmts.len() - i - 1,
                        if stmts.len() - i - 1 == 1 { "" } else { "s" },
                    ),
                    line,
                    column,
                });
                break; // only one diagnostic per block
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0042(child, out));
}

// ============================================================
// L0043: `let` binding shadows an existing binding or parameter
// ============================================================
//
// Fires when a `let` declaration inside a function introduces a name
// that is already bound by:
//   (a) a function parameter, OR
//   (b) a prior `let` in the same lexical scope chain.
//
// We track a single flat name set per function body (no scope nesting)
// because the common beginner mistake is re-declaring the same variable
// name in the same function. Detecting inter-scope shadowing would
// require full scope-chain tracking — out of scope for a lint pass.
// The typechecker's L0017 already fires on nested-block shadows.

fn run_l0043_shadowed_binding(program: &Node, out: &mut Vec<Lint>) {
    walk_l0043_top(program, out);
}

fn walk_l0043_top(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Function {
        parameters, body, ..
    } = node
    {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (_ty, name) in parameters {
            seen.insert(name.clone());
        }
        walk_l0043_block(body, &mut seen, out);
    }
    recurse_children(node, &mut |child| walk_l0043_top(child, out));
}

fn walk_l0043_block(
    node: &Node,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<Lint>,
) {
    match node {
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                walk_l0043_block(stmt, seen, out);
            }
        }
        Node::LetStatement { name, span, .. } => {
            if seen.contains(name) {
                out.push(Lint {
                    code: "L0043".into(),
                    severity: Severity::Warning,
                    message: format!("`let {name}` shadows an existing binding with the same name"),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            } else {
                seen.insert(name.clone());
            }
        }
        // Don't recurse into nested function definitions — they have their own scope.
        Node::Function { .. } => {}
        _ => {
            recurse_children(node, &mut |child| walk_l0043_block(child, seen, out));
        }
    }
}

// ============================================================
// L0044: shift amount is a literal outside 0..63
// ============================================================
//
// Fires when a `<<` or `>>` expression has an integer literal as the
// right-hand operand and that literal is outside the range 0..=63.
// Any such shift is a guaranteed runtime error; catching it statically
// (as a lint rather than a compiler error) lets the checker report a
// friendly diagnostic before the program runs.
//
// This complements the VM/interpreter's ShiftOutOfRange runtime error
// by surfacing the problem at lint time when the shift amount is known.

fn run_l0044_shift_out_of_range(program: &Node, out: &mut Vec<Lint>) {
    walk_l0044(program, out);
}

fn walk_l0044(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        right,
        span,
        ..
    } = node
        && matches!(operator.as_str(), "<<" | ">>")
        && let Node::IntegerLiteral { value, .. } = right.as_ref()
        && !(0..64).contains(value)
    {
        out.push(Lint {
            code: "L0044".into(),
            severity: Severity::Warning,
            message: format!(
                "shift amount `{value}` is outside 0..63 — this is always a runtime error"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0044(child, out));
}

// ============================================================
// L0045: constant-false condition in `while` loop
// ============================================================
//
// `while false { ... }` — body never executes. Detects the same
// `try_const_bool` helper as L0016 (constant condition in `if`),
// but fires on `WhileStatement` conditions instead.

fn run_l0045_while_false(program: &Node, out: &mut Vec<Lint>) {
    walk_l0045(program, out);
}

fn walk_l0045(node: &Node, out: &mut Vec<Lint>) {
    if let Node::WhileStatement {
        condition, span, ..
    } = node
        && matches!(try_const_bool(condition), Some(false))
    {
        out.push(Lint {
            code: "L0045".into(),
            severity: Severity::Warning,
            message: "loop condition is always `false` — the body never executes; \
                      remove the dead loop or correct the condition"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0045(child, out));
}

// ============================================================
// L0046: empty `for` loop body
// ============================================================
//
// `for x in expr { }` — the body is an empty block so the iteration
// has no observable effect. The iterable is still evaluated (and any
// side effects there fire), but the loop variable is bound and
// discarded without ever being used.

fn run_l0046_empty_for_body(program: &Node, out: &mut Vec<Lint>) {
    walk_l0046(program, out);
}

fn walk_l0046(node: &Node, out: &mut Vec<Lint>) {
    if let Node::ForInStatement { body, span, .. } = node
        && let Node::Block { stmts, .. } = body.as_ref()
        && stmts.is_empty()
    {
        out.push(Lint {
            code: "L0046".into(),
            severity: Severity::Warning,
            message: "empty `for` loop body — the iteration has no effect; \
                      add the missing body or remove the loop"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0046(child, out));
}

// ============================================================
// L0047: vacuous or always-failing `assert`
// ============================================================
//
// `assert(true)` — satisfied unconditionally, provides no safety
// guarantee. `assert(false)` — always panics, equivalent to an
// unconditional abort but without a clear message.

fn run_l0047_vacuous_assert(program: &Node, out: &mut Vec<Lint>) {
    walk_l0047(program, out);
}

fn walk_l0047(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Assert {
        condition, span, ..
    } = node
        && let Some(val) = try_const_bool(condition)
    {
        let msg = if val {
            "assert(true) is always satisfied and provides no safety guarantee; \
             supply a meaningful predicate or remove the assert"
        } else {
            "assert(false) always panics at runtime — \
             use a named abort function or add a comment explaining the invariant"
        };
        out.push(Lint {
            code: "L0047".into(),
            severity: Severity::Warning,
            message: msg.into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0047(child, out));
}

// ============================================================
// L0048: bitwise XOR of a value with itself (`x ^ x`)
// ============================================================
//
// `x ^ x` is always 0 for any integer `x`. In a high-level language
// this pattern is almost always a copy-paste error (both sides of the
// XOR were meant to be different variables). At the source level, if
// zeroing was the intent, write `0` directly.
//
// Detects: `InfixExpression { op: "^", left: Identifier(n), right: Identifier(n) }`
// where both names are identical.

fn run_l0048_xor_with_self(program: &Node, out: &mut Vec<Lint>) {
    walk_l0048(program, out);
}

fn walk_l0048(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
        ..
    } = node
        && operator == "^"
        && let Node::Identifier { name: lname, .. } = left.as_ref()
        && let Node::Identifier { name: rname, .. } = right.as_ref()
        && lname == rname
    {
        out.push(Lint {
            code: "L0048".into(),
            severity: Severity::Warning,
            message: format!(
                "`{lname} ^ {lname}` is always 0 — XOR of a value with itself; \
                 likely a copy-paste bug (use the other operand) or replace with `0`"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0048(child, out));
}

// ============================================================
// L0049: empty `if` then-branch
// ============================================================
//
// `if condition { }` has an empty then-branch. In safety-critical code
// this is almost always either:
//   (a) a forgotten body — the statements were not yet written, or
//   (b) an inverted condition — the else branch should be the then branch.
//
// Fires only on `if` with a non-empty alternative OR a bare `if` with no
// alternative and empty body (both are suspicious). Skips `if cond { }`
// when the body is whitespace-only — but at the AST level an empty block
// has `stmts: []` regardless of whitespace, so we fire on all of them.

fn run_l0049_empty_if_body(program: &Node, out: &mut Vec<Lint>) {
    walk_l0049(program, out);
}

fn walk_l0049(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        consequence, span, ..
    } = node
        && let Node::Block { stmts, .. } = consequence.as_ref()
        && stmts.is_empty()
    {
        out.push(Lint {
            code: "L0049".into(),
            severity: Severity::Warning,
            message: "empty `if` then-branch — either the body is missing or the condition \
                      is inverted (negate the condition and drop the `else` if present)"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0049(child, out));
}

// ============================================================
// L0050: redundant `else` after `if` that always breaks/continues
// ============================================================
//
// When the `if` consequence always exits the current loop iteration via
// `break` or `continue`, the `else` block is dead under the if-taken
// path. The else body can be de-nested to the same level as the `if`.
//
// This is the loop-exit mirror of L0041 (redundant else after return).
// The check is conservative: fires only when the LAST statement of the
// consequence block is a bare `break` or `continue`.

fn run_l0050_redundant_else_after_loop_exit(program: &Node, out: &mut Vec<Lint>) {
    walk_l0050(program, out);
}

fn walk_l0050(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        consequence,
        alternative: Some(_),
        span,
        ..
    } = node
        && consequence_always_exits_loop(consequence)
    {
        out.push(Lint {
            code: "L0050".into(),
            severity: Severity::Warning,
            message: "`else` block is redundant — the `if` arm always `break`s or `continue`s; \
                      de-nest the body and drop the `else`"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0050(child, out));
}

/// Returns `true` when the last statement of `block` is `break` or
/// `continue`, meaning the loop iteration always exits at this point.
fn consequence_always_exits_loop(block: &Node) -> bool {
    let Node::Block { stmts, .. } = block else {
        return false;
    };
    matches!(
        stmts.last(),
        Some(Node::Break { .. }) | Some(Node::Continue { .. })
    )
}

// ============================================================
// L0051: comparison of two string literals
// ============================================================

fn run_l0051_string_literal_comparison(program: &Node, out: &mut Vec<Lint>) {
    walk_l0051(program, out);
}

fn walk_l0051(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
    } = node
        && matches!(operator.as_str(), "==" | "!=")
        && let Node::StringLiteral { value: lv, .. } = left.as_ref()
        && let Node::StringLiteral { value: rv, .. } = right.as_ref()
    {
        let result = if operator == "==" { lv == rv } else { lv != rv };
        out.push(Lint {
            code: "L0051".into(),
            severity: Severity::Warning,
            message: format!(
                "comparison of two string literals always evaluates to `{result}` — \
                 did you mean to compare against a variable?"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0051(child, out));
}

// ============================================================
// L0052: negation of a boolean literal
// ============================================================

fn run_l0052_negated_bool_literal(program: &Node, out: &mut Vec<Lint>) {
    walk_l0052(program, out);
}

fn walk_l0052(node: &Node, out: &mut Vec<Lint>) {
    if let Node::PrefixExpression {
        operator,
        right,
        span,
    } = node
        && operator == "!"
        && let Node::BooleanLiteral { value, .. } = right.as_ref()
    {
        let simplified = if *value { "false" } else { "true" };
        out.push(Lint {
            code: "L0052".into(),
            severity: Severity::Warning,
            message: format!(
                "`!{}` is always `{simplified}` — use `{simplified}` directly",
                value
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0052(child, out));
}

// ============================================================
// L0053: array index literal out of bounds
// ============================================================

fn run_l0053_out_of_bounds_literal_index(program: &Node, out: &mut Vec<Lint>) {
    walk_l0053(program, out);
}

fn walk_l0053(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IndexExpression {
        target,
        index,
        span,
    } = node
        && let Node::ArrayLiteral { items, .. } = target.as_ref()
    {
        // Resolve the index to an i64 if it's a compile-time constant.
        // `-1` is parsed as PrefixExpression(`-`, IntegerLiteral(1)).
        let const_idx: Option<i64> = match index.as_ref() {
            Node::IntegerLiteral { value, .. } => Some(*value),
            Node::PrefixExpression {
                operator, right, ..
            } if operator == "-" => {
                if let Node::IntegerLiteral { value, .. } = right.as_ref() {
                    Some(-(*value))
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(idx) = const_idx {
            let len = items.len() as i64;
            if idx < 0 || idx >= len {
                out.push(Lint {
                    code: "L0053".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "index `{idx}` is out of bounds for an array literal of length {len} — \
                         this will always panic at runtime"
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0053(child, out));
}

// ============================================================
// L0055 — redundant boolean `!=` check (`x != true` / `x != false`)
//
// Complements L0023 (which handles `==`) by catching the `!=` forms
// that L0023 intentionally skips.
// ============================================================

fn run_l0055_redundant_bool_neq(program: &Node, out: &mut Vec<Lint>) {
    walk_l0055(program, out);
}

fn walk_l0055(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
    } = node
        && operator == "!="
    {
        let bool_val = if let Node::BooleanLiteral { value, .. } = left.as_ref() {
            Some(*value)
        } else if let Node::BooleanLiteral { value, .. } = right.as_ref() {
            Some(*value)
        } else {
            None
        };
        if let Some(bv) = bool_val {
            let suggestion = if bv {
                "use `!expr` instead of `expr != true`"
            } else {
                "use `expr` directly instead of `expr != false`"
            };
            out.push(Lint {
                code: "L0055".into(),
                severity: Severity::Warning,
                message: format!(
                    "redundant `!= {}` comparison — {suggestion}",
                    if bv { "true" } else { "false" }
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
    recurse_children(node, &mut |child| walk_l0055(child, out));
}

// ============================================================
// L0057 — redundant addition of zero (`x + 0` / `0 + x`)
// ============================================================

fn run_l0057_add_zero(program: &Node, out: &mut Vec<Lint>) {
    walk_l0057(program, out);
}

fn walk_l0057(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
    } = node
        && operator == "+"
        && (matches!(left.as_ref(), Node::IntegerLiteral { value: 0, .. })
            || matches!(right.as_ref(), Node::IntegerLiteral { value: 0, .. }))
    {
        out.push(Lint {
            code: "L0057".into(),
            severity: Severity::Warning,
            message: "redundant addition of zero — simplify to `x`".into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0057(child, out));
}

// ============================================================
// L0058 — redundant subtraction of zero (`x - 0`)
// ============================================================

fn run_l0058_sub_zero(program: &Node, out: &mut Vec<Lint>) {
    walk_l0058(program, out);
}

fn walk_l0058(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        right,
        span,
        ..
    } = node
        && operator == "-"
        && matches!(right.as_ref(), Node::IntegerLiteral { value: 0, .. })
    {
        out.push(Lint {
            code: "L0058".into(),
            severity: Severity::Warning,
            message: "redundant subtraction of zero — simplify to `x`".into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0058(child, out));
}

// ============================================================
// L0059 — redundant multiplication by one (`x * 1` / `1 * x`)
// ============================================================

fn run_l0059_mul_one(program: &Node, out: &mut Vec<Lint>) {
    walk_l0059(program, out);
}

fn walk_l0059(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
    } = node
        && operator == "*"
        && (matches!(left.as_ref(), Node::IntegerLiteral { value: 1, .. })
            || matches!(right.as_ref(), Node::IntegerLiteral { value: 1, .. }))
    {
        out.push(Lint {
            code: "L0059".into(),
            severity: Severity::Warning,
            message: "redundant multiplication by one — simplify to `x`".into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0059(child, out));
}

// ============================================================
// L0060 — redundant division by one (`x / 1`)
// ============================================================

fn run_l0060_div_one(program: &Node, out: &mut Vec<Lint>) {
    walk_l0060(program, out);
}

fn walk_l0060(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        right,
        span,
        ..
    } = node
        && operator == "/"
        && matches!(right.as_ref(), Node::IntegerLiteral { value: 1, .. })
    {
        out.push(Lint {
            code: "L0060".into(),
            severity: Severity::Warning,
            message: "redundant division by one — simplify to `x`".into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0060(child, out));
}

// ============================================================
// L0061 — shift by zero is a no-op (`x << 0` / `x >> 0`)
// ============================================================

fn run_l0061_shift_zero(program: &Node, out: &mut Vec<Lint>) {
    walk_l0061(program, out);
}

fn walk_l0061(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        right,
        span,
        ..
    } = node
        && (operator == "<<" || operator == ">>")
        && matches!(right.as_ref(), Node::IntegerLiteral { value: 0, .. })
    {
        out.push(Lint {
            code: "L0061".into(),
            severity: Severity::Warning,
            message: format!("shifting by zero (`{operator} 0`) is a no-op — simplify to `x`"),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0061(child, out));
}

// ============================================================
// L0062 — tautological inequality comparison with self
// (`x < x` / `x > x` always false; `x <= x` / `x >= x` always true)
// ============================================================

fn run_l0062_inequality_with_self(program: &Node, out: &mut Vec<Lint>) {
    walk_l0062(program, out);
}

fn walk_l0062(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
    } = node
        && matches!(operator.as_str(), "<" | ">" | "<=" | ">=")
        && let Node::Identifier { name: lname, .. } = left.as_ref()
        && let Node::Identifier { name: rname, .. } = right.as_ref()
        && lname == rname
    {
        let result = if operator == "<" || operator == ">" {
            "always false"
        } else {
            "always true"
        };
        out.push(Lint {
            code: "L0062".into(),
            severity: Severity::Warning,
            message: format!(
                "`{lname} {operator} {lname}` is {result} — tautological self-comparison"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0062(child, out));
}

// ============================================================
// L0063 — dead code after `break` or `continue` statement
// ============================================================

fn run_l0063_dead_after_break_continue(program: &Node, out: &mut Vec<Lint>) {
    walk_l0063(program, out);
}

fn walk_l0063(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Block { stmts, .. } = node {
        let mut saw_exit = false;
        for stmt in stmts {
            if saw_exit {
                // Skip trivial bare `return;` — same convention as L0007.
                if matches!(stmt, Node::ReturnStatement { value: None, .. }) {
                    continue;
                }
                if let Some(span) = span_of(stmt) {
                    out.push(Lint {
                        code: "L0063".into(),
                        severity: Severity::Warning,
                        message: "dead code after `break`/`continue` statement".into(),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                }
                // Report only the first dead statement.
                break;
            }
            if matches!(stmt, Node::Break { .. } | Node::Continue { .. }) {
                saw_exit = true;
            }
            // Recurse into nested blocks regardless.
            walk_l0063(stmt, out);
        }
    } else {
        recurse_children(node, &mut |child| walk_l0063(child, out));
    }
}

// ============================================================
// L0064 — empty `else {}` block
// ============================================================

fn run_l0064_empty_else_block(program: &Node, out: &mut Vec<Lint>) {
    walk_l0064(program, out);
}

fn walk_l0064(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        alternative: Some(alt),
        span,
        ..
    } = node
        && let Node::Block { stmts, .. } = alt.as_ref()
        && stmts.is_empty()
    {
        out.push(Lint {
            code: "L0064".into(),
            severity: Severity::Warning,
            message: "empty `else {}` block can be removed — it has no effect".into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0064(child, out));
}

// ============================================================
// L0065 — `if cond { return true; } else { return false; }` simplifies to `return cond;`
// ============================================================

fn run_l0065_bool_identity_if(program: &Node, out: &mut Vec<Lint>) {
    walk_l0065(program, out);
}

fn walk_l0065(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        consequence,
        alternative: Some(alt),
        span,
        ..
    } = node
        && let Node::Block {
            stmts: cons_stmts, ..
        } = consequence.as_ref()
        && cons_stmts.len() == 1
        && matches!(
            &cons_stmts[0],
            Node::ReturnStatement {
                value: Some(v), ..
            } if matches!(v.as_ref(), Node::BooleanLiteral { value: true, .. })
        )
        && let Node::Block {
            stmts: alt_stmts, ..
        } = alt.as_ref()
        && alt_stmts.len() == 1
        && matches!(
            &alt_stmts[0],
            Node::ReturnStatement {
                value: Some(v), ..
            } if matches!(v.as_ref(), Node::BooleanLiteral { value: false, .. })
        )
    {
        out.push(Lint {
            code: "L0065".into(),
            severity: Severity::Warning,
            message:
                "`if cond { return true; } else { return false; }` simplifies to `return cond;`"
                    .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0065(child, out));
}

// ============================================================
// L0066 — `if cond { return false; } else { return true; }` simplifies to `return !cond;`
// ============================================================

fn run_l0066_bool_negation_if(program: &Node, out: &mut Vec<Lint>) {
    walk_l0066(program, out);
}

fn walk_l0066(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        consequence,
        alternative: Some(alt),
        span,
        ..
    } = node
        && let Node::Block {
            stmts: cons_stmts, ..
        } = consequence.as_ref()
        && cons_stmts.len() == 1
        && matches!(
            &cons_stmts[0],
            Node::ReturnStatement {
                value: Some(v), ..
            } if matches!(v.as_ref(), Node::BooleanLiteral { value: false, .. })
        )
        && let Node::Block {
            stmts: alt_stmts, ..
        } = alt.as_ref()
        && alt_stmts.len() == 1
        && matches!(
            &alt_stmts[0],
            Node::ReturnStatement {
                value: Some(v), ..
            } if matches!(v.as_ref(), Node::BooleanLiteral { value: true, .. })
        )
    {
        out.push(Lint {
            code: "L0066".into(),
            severity: Severity::Warning,
            message:
                "`if cond { return false; } else { return true; }` simplifies to `return !cond;`"
                    .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0066(child, out));
}

// ---- L0071: function with more than 5 parameters ----

const MAX_PARAM_COUNT: usize = 5;

fn run_l0071_too_many_params(program: &Node, out: &mut Vec<Lint>) {
    walk_l0071(program, out);
}

fn walk_l0071(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Function {
        name,
        parameters,
        span,
        ..
    } = node
        && parameters.len() > MAX_PARAM_COUNT
    {
        out.push(Lint {
            code: "L0071".into(),
            severity: Severity::Warning,
            message: format!(
                "function `{name}` has {} parameters (limit: {MAX_PARAM_COUNT}) — \
                 consider grouping related parameters into a struct",
                parameters.len()
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0071(child, out));
}

// ---- L0072: for-loop variable never used in body ----

fn run_l0072_unused_for_var(program: &Node, out: &mut Vec<Lint>) {
    walk_l0072(program, out);
}

fn walk_l0072(node: &Node, out: &mut Vec<Lint>) {
    if let Node::ForInStatement {
        name, body, span, ..
    } = node
        && !name.is_empty()
        && !ident_used_in(body, name)
    {
        out.push(Lint {
            code: "L0072".into(),
            severity: Severity::Warning,
            message: format!(
                "for-loop variable `{name}` is never used in the loop body — \
                 use `_` to signal intentional discard, or use the variable"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0072(child, out));
}

fn ident_used_in(node: &Node, target: &str) -> bool {
    crate::uniqueness_walk::any_node(
        node,
        |n| matches!(n, Node::Identifier { name, .. } if name == target),
    )
}

// ---- L0073: duplicate contract clause ----

fn run_l0073_duplicate_contract_clause(program: &Node, out: &mut Vec<Lint>) {
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::Function {
                requires,
                ensures,
                span,
                name,
                ..
            } = &s.node
            {
                check_duplicate_clauses(name, requires, "requires", *span, out);
                check_duplicate_clauses(name, ensures, "ensures", *span, out);
            }
        }
    }
}

fn clause_text(n: &Node) -> String {
    match n {
        Node::Identifier { name, .. } => name.clone(),
        Node::BooleanLiteral { value, .. } => value.to_string(),
        Node::IntegerLiteral { value, .. } => value.to_string(),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => format!("{} {operator} {}", clause_text(left), clause_text(right)),
        Node::PrefixExpression {
            operator, right, ..
        } => {
            format!("{operator}{}", clause_text(right))
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let args: Vec<String> = arguments.iter().map(clause_text).collect();
            format!("{}({})", clause_text(function), args.join(", "))
        }
        _ => format!("{:?}", n as *const _),
    }
}

fn check_duplicate_clauses(
    fn_name: &str,
    clauses: &[Node],
    kind: &str,
    span: Span,
    out: &mut Vec<Lint>,
) {
    if clauses.len() < 2 {
        return;
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for clause in clauses {
        let text = clause_text(clause);
        if !seen.insert(text.clone()) {
            out.push(Lint {
                code: "L0073".into(),
                severity: Severity::Warning,
                message: format!(
                    "function `{fn_name}` has duplicate `{kind}` clause `{text}` — \
                     remove the repeated clause"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
}

// ---- L0074: pure function call result discarded ----

fn run_l0074_pure_call_result_discarded(program: &Node, out: &mut Vec<Lint>) {
    // Phase 1: collect names of all `pure`-declared functions.
    let Node::Program(stmts) = program else {
        return;
    };
    let pure_fns: std::collections::HashSet<&str> = stmts
        .iter()
        .filter_map(|s| {
            if let Node::Function { name, pure, .. } = &s.node
                && *pure
            {
                return Some(name.as_str());
            }
            None
        })
        .collect();
    if pure_fns.is_empty() {
        return;
    }
    // Phase 2: find expression-statement calls to pure functions.
    for s in stmts {
        walk_l0074(&s.node, &pure_fns, out);
    }
}

fn walk_l0074(node: &Node, pure_fns: &std::collections::HashSet<&str>, out: &mut Vec<Lint>) {
    if let Node::ExpressionStatement { expr, span } = node
        && let Node::CallExpression { function, .. } = expr.as_ref()
        && let Node::Identifier { name, .. } = function.as_ref()
        && pure_fns.contains(name.as_str())
    {
        out.push(Lint {
            code: "L0074".into(),
            severity: Severity::Warning,
            message: format!(
                "result of `pure` function `{name}` is discarded — \
                 the call has no observable effect; assign the result \
                 or remove the call"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0074(child, pure_fns, out));
}

// ---- L0075: trivially-vacuous contract clause ----

fn run_l0075_vacuous_contract_clause(program: &Node, out: &mut Vec<Lint>) {
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::Function {
                name,
                requires,
                ensures,
                span,
                ..
            } = &s.node
            {
                check_vacuous_clauses(name, requires, "requires", *span, out);
                check_vacuous_clauses(name, ensures, "ensures", *span, out);
            }
        }
    }
}

fn check_vacuous_clauses(
    fn_name: &str,
    clauses: &[Node],
    kind: &str,
    span: Span,
    out: &mut Vec<Lint>,
) {
    for clause in clauses {
        if let Node::BooleanLiteral { value, .. } = clause {
            let desc = if *value {
                "trivially true"
            } else {
                "trivially false"
            };
            let hint = match (kind, *value) {
                ("requires", true) => "vacuous precondition — remove it or write a real constraint",
                ("requires", false) => "unsatisfiable precondition — function can never be called",
                ("ensures", true) => "vacuous postcondition — remove it or write a real constraint",
                ("ensures", false) => "impossible postcondition — function can never satisfy this",
                _ => "trivial clause",
            };
            out.push(Lint {
                code: "L0075".into(),
                severity: Severity::Warning,
                message: format!("function `{fn_name}` has `{kind} {value}` ({desc}): {hint}"),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
}

// ---- L0054: empty `while` loop body ----

fn run_l0054_empty_while_body(program: &Node, out: &mut Vec<Lint>) {
    walk_l0054(program, out);
}

fn walk_l0054(node: &Node, out: &mut Vec<Lint>) {
    if let Node::WhileStatement { body, span, .. } = node
        && let Node::Block { stmts, .. } = body.as_ref()
        && stmts.is_empty()
    {
        out.push(Lint {
            code: "L0054".into(),
            severity: Severity::Warning,
            message: "empty `while` loop body — the loop iterates but does nothing; \
                      add the missing body or replace with a yield mechanism"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0054(child, out));
}

// ---- L0056: `for x in []` — iterating over empty literal array ----

fn run_l0056_for_over_empty_array(program: &Node, out: &mut Vec<Lint>) {
    walk_l0056(program, out);
}

fn walk_l0056(node: &Node, out: &mut Vec<Lint>) {
    if let Node::ForInStatement { iterable, span, .. } = node
        && let Node::ArrayLiteral { items, .. } = iterable.as_ref()
        && items.is_empty()
    {
        out.push(Lint {
            code: "L0056".into(),
            severity: Severity::Warning,
            message: "`for` loop iterates over an empty array literal `[]` — \
                      the loop body is never executed; populate the array or remove the loop"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0056(child, out));
}

// ---- L0067: `x && true` / `true && x` — AND with true is identity ----

fn run_l0067_and_true(program: &Node, out: &mut Vec<Lint>) {
    walk_l0067(program, out);
}

fn walk_l0067(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
        ..
    } = node
        && operator == "&&"
        && (matches!(left.as_ref(), Node::BooleanLiteral { value: true, .. })
            || matches!(right.as_ref(), Node::BooleanLiteral { value: true, .. }))
    {
        out.push(Lint {
            code: "L0067".into(),
            severity: Severity::Warning,
            message: "`x && true` / `true && x` — AND with `true` is the identity; simplify to `x`"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0067(child, out));
}

// ---- L0068: `x && false` / `false && x` — AND with false is always false ----

fn run_l0068_and_false(program: &Node, out: &mut Vec<Lint>) {
    walk_l0068(program, out);
}

fn walk_l0068(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
        ..
    } = node
        && operator == "&&"
        && (matches!(left.as_ref(), Node::BooleanLiteral { value: false, .. })
            || matches!(right.as_ref(), Node::BooleanLiteral { value: false, .. }))
    {
        out.push(Lint {
            code: "L0068".into(),
            severity: Severity::Warning,
            message: "`x && false` / `false && x` — AND with `false` is always `false`; likely a logic error".into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0068(child, out));
}

// ---- L0069: `x || true` / `true || x` — OR with true is always true ----

fn run_l0069_or_true(program: &Node, out: &mut Vec<Lint>) {
    walk_l0069(program, out);
}

fn walk_l0069(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
        ..
    } = node
        && operator == "||"
        && (matches!(left.as_ref(), Node::BooleanLiteral { value: true, .. })
            || matches!(right.as_ref(), Node::BooleanLiteral { value: true, .. }))
    {
        out.push(Lint {
            code: "L0069".into(),
            severity: Severity::Warning,
            message:
                "`x || true` / `true || x` — OR with `true` is always `true`; likely a logic error"
                    .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0069(child, out));
}

// ---- L0070: `x || false` / `false || x` — OR with false is identity ----

fn run_l0070_or_false(program: &Node, out: &mut Vec<Lint>) {
    walk_l0070(program, out);
}

fn walk_l0070(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
        ..
    } = node
        && operator == "||"
        && (matches!(left.as_ref(), Node::BooleanLiteral { value: false, .. })
            || matches!(right.as_ref(), Node::BooleanLiteral { value: false, .. }))
    {
        out.push(Lint {
            code: "L0070".into(),
            severity: Severity::Warning,
            message:
                "`x || false` / `false || x` — OR with `false` is the identity; simplify to `x`"
                    .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0070(child, out));
}

// ============================================================
// L0076: `result` identifier used inside a `requires` clause
// ============================================================

fn contains_result_identifier(node: &Node) -> bool {
    if matches!(node, Node::Identifier { name, .. } if name == "result") {
        return true;
    }
    let mut found = false;
    recurse_children(node, &mut |child| {
        if contains_result_identifier(child) {
            found = true;
        }
    });
    found
}

fn run_l0076_result_in_requires(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for s in stmts {
        if let Node::Function {
            name,
            requires,
            span,
            ..
        } = &s.node
        {
            for req in requires {
                if contains_result_identifier(req) {
                    out.push(Lint {
                        code: "L0076".into(),
                        message: format!(
                            "`result` is not in scope in `requires` clauses — \
                             use `ensures` for postconditions (function `{name}`)"
                        ),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                        severity: Severity::Warning,
                    });
                    break;
                }
            }
        }
    }
}

// ============================================================
// L0077: `ensures result` in a function with no return type
// ============================================================

fn run_l0077_ensures_result_void(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for s in stmts {
        if let Node::Function {
            name,
            return_type: None,
            ensures,
            span,
            ..
        } = &s.node
        {
            for ens in ensures {
                if contains_result_identifier(ens) {
                    out.push(Lint {
                        code: "L0077".into(),
                        message: format!(
                            "`ensures result` on void function `{name}` — \
                             function has no return value to constrain"
                        ),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                        severity: Severity::Warning,
                    });
                    break;
                }
            }
        }
    }
}

// ============================================================
// L0078: Function parameter shadows a builtin function name
// ============================================================

const SHADOWED_BUILTINS: &[&str] = &[
    "len", "print", "println", "assert", "format", "abs", "min", "max", "sqrt", "panic", "abort",
    "push", "pop", "contains", "split", "join", "range", "keys", "values", "type_of", "exit",
];

fn run_l0078_param_shadows_builtin(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for s in stmts {
        if let Node::Function {
            name: fn_name,
            parameters,
            span,
            ..
        } = &s.node
        {
            for (_ty, param_name) in parameters {
                if SHADOWED_BUILTINS.contains(&param_name.as_str()) {
                    out.push(Lint {
                        code: "L0078".into(),
                        message: format!(
                            "parameter `{param_name}` in `{fn_name}` shadows \
                             builtin function `{param_name}()`"
                        ),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                        severity: Severity::Warning,
                    });
                }
            }
        }
    }
}

// ============================================================
// L0079: Empty function body
// ============================================================

fn run_l0079_empty_function_body(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for s in stmts {
        if let Node::Function {
            name, body, span, ..
        } = &s.node
            && let Node::Block {
                stmts: body_stmts, ..
            } = body.as_ref()
            && body_stmts.is_empty()
        {
            out.push(Lint {
                code: "L0079".into(),
                message: format!("function `{name}` has an empty body"),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    }
}

// ============================================================
// L0080: `let` binding immediately overwritten before first use
// ============================================================

fn run_l0080_dead_let_init(program: &Node, out: &mut Vec<Lint>) {
    walk_l0080(program, out);
}

fn walk_l0080(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Block { stmts, .. } = node {
        for i in 0..stmts.len().saturating_sub(1) {
            if let Node::LetStatement { name, span, .. } = &stmts[i]
                && let Node::Assignment {
                    name: asgn_name, ..
                } = &stmts[i + 1]
                && asgn_name == name
            {
                out.push(Lint {
                    code: "L0080".into(),
                    message: format!(
                        "initial value of `{name}` is overwritten before use — \
                         the `let` initialization is dead"
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                    severity: Severity::Warning,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0080(child, out));
}

// ============================================================
// L0081: Duplicate consecutive `assert` statements
// ============================================================

fn run_l0081_duplicate_assert(program: &Node, out: &mut Vec<Lint>) {
    walk_l0081(program, out);
}

fn walk_l0081(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Block { stmts, .. } = node {
        for i in 0..stmts.len().saturating_sub(1) {
            if let Node::Assert {
                condition: cond1,
                span: span1,
                ..
            } = &stmts[i]
                && let Node::Assert {
                    condition: cond2,
                    span,
                    ..
                } = &stmts[i + 1]
                && clause_text(cond1) == clause_text(cond2)
            {
                out.push(Lint {
                    code: "L0081".into(),
                    message: format!(
                        "duplicate `assert` — condition `{}` was already asserted \
                         on line {}",
                        clause_text(cond1),
                        span1.start.line
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                    severity: Severity::Warning,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0081(child, out));
}

// ============================================================
// L0082: Both branches of if/else are empty
// ============================================================

fn run_l0082_both_branches_empty(program: &Node, out: &mut Vec<Lint>) {
    walk_l0082(program, out);
}

fn walk_l0082(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        consequence,
        alternative: Some(alt),
        span,
        ..
    } = node
    {
        let then_empty =
            matches!(consequence.as_ref(), Node::Block { stmts, .. } if stmts.is_empty());
        let else_empty = matches!(alt.as_ref(), Node::Block { stmts, .. } if stmts.is_empty());
        if then_empty && else_empty {
            out.push(Lint {
                code: "L0082".into(),
                message: "both branches of this `if/else` are empty — the statement has no effect"
                    .into(),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    }
    recurse_children(node, &mut |child| walk_l0082(child, out));
}

// ============================================================
// L0083: `@noreturn`-annotated function has a declared return type
// ============================================================

fn run_l0083_noreturn_with_return_type(program: &Node, source: &str, out: &mut Vec<Lint>) {
    let noreturn_fns = collect_noreturn_functions(source);
    if noreturn_fns.is_empty() {
        return;
    }
    let Node::Program(stmts) = program else {
        return;
    };
    for s in stmts {
        if let Node::Function {
            name,
            return_type: Some(rt),
            span,
            ..
        } = &s.node
            && noreturn_fns.contains(name.as_str())
        {
            out.push(Lint {
                code: "L0083".into(),
                message: format!(
                    "`@noreturn` function `{name}` declares return type `{rt}` — \
                     `@noreturn` functions never return to the caller"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    }
}

// ============================================================
// L0084: Nested function definition
// ============================================================

fn run_l0084_nested_function(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for s in stmts {
        if let Node::Function {
            name: outer_name,
            body,
            ..
        } = &s.node
        {
            walk_l0084_in_body(body, outer_name, out);
        }
    }
}

fn walk_l0084_in_body(node: &Node, outer_name: &str, out: &mut Vec<Lint>) {
    if let Node::Function { name, span, .. } = node {
        out.push(Lint {
            code: "L0084".into(),
            message: format!(
                "nested function `{name}` defined inside `{outer_name}` — \
                 consider hoisting to top level"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
            severity: Severity::Warning,
        });
        // don't recurse deeper into nested-of-nested to avoid duplicate reports
        return;
    }
    recurse_children(node, &mut |child| {
        walk_l0084_in_body(child, outer_name, out)
    });
}

// ============================================================
// L0085: Struct with zero fields
// ============================================================

fn run_l0085_empty_struct(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for s in stmts {
        if let Node::StructDecl {
            name, fields, span, ..
        } = &s.node
            && fields.is_empty()
        {
            out.push(Lint {
                code: "L0085".into(),
                message: format!(
                    "struct `{name}` has no fields — add fields or replace with a type alias"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    }
}

// ============================================================
// L0086: string compared to empty literal `== ""` / `!= ""`
// ============================================================

fn run_l0086_empty_string_comparison(program: &Node, out: &mut Vec<Lint>) {
    walk_nodes(program, &mut |node| {
        if let Node::InfixExpression {
            operator,
            left,
            right,
            span,
        } = node
        {
            if !matches!(operator.as_str(), "==" | "!=") {
                return;
            }
            let is_empty_str =
                |n: &Node| matches!(n, Node::StringLiteral { value, .. } if value.is_empty());
            if is_empty_str(left) || is_empty_str(right) {
                let op = operator.clone();
                let suggestion = if op == "==" {
                    "is_empty(s)"
                } else {
                    "!is_empty(s)"
                };
                out.push(Lint {
                    code: "L0086".into(),
                    message: format!(
                        "comparing to empty string literal with `{op}`; \
                         prefer `{suggestion}` for clarity"
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                    severity: Severity::Warning,
                });
            }
        }
    });
}

fn walk_nodes<F: FnMut(&Node)>(node: &Node, f: &mut F) {
    f(node);
    recurse_children(node, &mut |child| walk_nodes(child, f));
}

// ============================================================
// L0087: pure function calls print/println
// ============================================================

fn run_l0087_pure_fn_prints(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for stmt in stmts {
        if let Node::Function {
            name,
            pure,
            body,
            span,
            ..
        } = &stmt.node
            && *pure
            && body_calls_print(body)
        {
            out.push(Lint {
                code: "L0087".into(),
                message: format!(
                    "function `{name}` is declared `pure` but calls `print` or `println`; \
                     pure functions must not produce side effects"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    }
}

fn body_calls_print(node: &Node) -> bool {
    match node {
        Node::CallExpression { function, .. } => {
            matches!(function.as_ref(), Node::Identifier { name, .. }
                if name == "print" || name == "println")
        }
        other => {
            let mut found = false;
            recurse_children(other, &mut |child| {
                if !found {
                    found = body_calls_print(child);
                }
            });
            found
        }
    }
}

// ============================================================
// L0088: `let _ = expr` wildcard discard binding
// ============================================================

fn run_l0088_wildcard_let_discard(program: &Node, out: &mut Vec<Lint>) {
    walk_nodes(program, &mut |node| {
        if let Node::LetStatement { name, span, .. } = node
            && name == "_"
        {
            out.push(Lint {
                code: "L0088".into(),
                message: "`let _ = expr;` wildcard discard — use a bare `expr;` statement instead"
                    .to_string(),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    });
}

// ============================================================
// L0089: exit/abort call inside a live recovery block
// ============================================================

fn run_l0089_exit_in_live_block(program: &Node, out: &mut Vec<Lint>) {
    walk_nodes(program, &mut |node| {
        if let Node::LiveBlock { body, span, .. } = node
            && body_calls_diverging(body)
        {
            out.push(Lint {
                code: "L0089".into(),
                message: "`exit()` or `abort()` inside a `live` recovery block defeats recovery; \
                           use a controlled error path instead"
                    .to_string(),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    });
}

const DIVERGING_CALLS: &[&str] = &["exit", "abort"];

fn body_calls_diverging(node: &Node) -> bool {
    match node {
        Node::CallExpression { function, .. } => {
            matches!(function.as_ref(), Node::Identifier { name, .. }
                if DIVERGING_CALLS.contains(&name.as_str()))
        }
        other => {
            let mut found = false;
            recurse_children(other, &mut |child| {
                if !found {
                    found = body_calls_diverging(child);
                }
            });
            found
        }
    }
}

// ============================================================
// L0090: both if/else arms return the same literal
// ============================================================

fn run_l0090_both_arms_same_return(program: &Node, out: &mut Vec<Lint>) {
    walk_nodes(program, &mut |node| {
        if let Node::IfStatement {
            consequence,
            alternative: Some(alt),
            span,
            ..
        } = node
            && let (Some(a), Some(b)) = (
                extract_sole_return_literal(consequence),
                extract_sole_return_literal(alt),
            )
            && a == b
        {
            out.push(Lint {
                code: "L0090".into(),
                message: format!(
                    "both `if` and `else` branches return the same value `{a}`; \
                     the condition is irrelevant — simplify to `return {a};`"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    });
}

fn extract_sole_return_literal(node: &Node) -> Option<String> {
    match node {
        Node::ReturnStatement { value: Some(v), .. } => literal_text(v),
        Node::Block { stmts, .. } if stmts.len() == 1 => extract_sole_return_literal(&stmts[0]),
        _ => None,
    }
}

fn literal_text(node: &Node) -> Option<String> {
    match node {
        Node::IntegerLiteral { value, .. } => Some(value.to_string()),
        Node::BooleanLiteral { value, .. } => Some(value.to_string()),
        Node::StringLiteral { value, .. } => Some(format!("\"{value}\"")),
        _ => None,
    }
}

// ============================================================
// L0091: for-range with equal start and end (0..0)
// ============================================================

fn run_l0091_empty_range_for(program: &Node, out: &mut Vec<Lint>) {
    walk_nodes(program, &mut |node| {
        if let Node::ForInStatement { iterable, span, .. } = node
            && let Node::Range { lo, hi, .. } = iterable.as_ref()
            && let (Node::IntegerLiteral { value: lv, .. }, Node::IntegerLiteral { value: hv, .. }) =
                (lo.as_ref(), hi.as_ref())
            && lv == hv
        {
            out.push(Lint {
                code: "L0091".into(),
                message: format!(
                    "for-loop range `{lv}..{hv}` has equal start and end — \
                     the loop body will never execute"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    });
}

// ============================================================
// L0092: `ensures false` — function claims it always diverges
// ============================================================

fn run_l0092_ensures_false(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for stmt in stmts {
        if let Node::Function {
            name,
            ensures,
            span,
            ..
        } = &stmt.node
            && ensures
                .iter()
                .any(|e| matches!(e, Node::BooleanLiteral { value: false, .. }))
        {
            out.push(Lint {
                code: "L0092".into(),
                message: format!(
                    "function `{name}` has `ensures false` — this claims the function \
                     never returns normally. If intentional, use `// @noreturn` instead."
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    }
}

// ============================================================
// L0093: function parameter named "result"
// ============================================================

fn run_l0093_param_named_result(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for stmt in stmts {
        if let Node::Function {
            name,
            parameters,
            span,
            ..
        } = &stmt.node
            && parameters.iter().any(|(_, n)| n == "result")
        {
            out.push(Lint {
                code: "L0093".into(),
                message: format!(
                    "function `{name}` has a parameter named `result`; \
                     this shadows the postcondition pseudo-variable in `ensures` clauses — \
                     rename the parameter"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    }
}

// ============================================================
// L0094: consecutive break/continue (second is unreachable)
// ============================================================

fn run_l0094_consecutive_break_continue(program: &Node, out: &mut Vec<Lint>) {
    walk_nodes(program, &mut |node| {
        if let Node::Block { stmts, .. } = node {
            let mut last_jump_span: Option<&Span> = None;
            for s in stmts {
                match s {
                    Node::Break { span, .. } | Node::Continue { span, .. } => {
                        if last_jump_span.is_some() {
                            out.push(Lint {
                                code: "L0094".into(),
                                message: "unreachable `break`/`continue` — a jump statement \
                                          already precedes this one in the same block"
                                    .to_string(),
                                line: span.start.line as u32,
                                column: span.start.column as u32,
                                severity: Severity::Warning,
                            });
                        }
                        last_jump_span = Some(span);
                    }
                    _ => {
                        last_jump_span = None;
                    }
                }
            }
        }
    });
}

// ============================================================
// L0095: match with a single wildcard arm
// ============================================================

fn run_l0095_single_wildcard_match(program: &Node, out: &mut Vec<Lint>) {
    walk_nodes(program, &mut |node| {
        if let Node::Match { arms, span, .. } = node
            && arms.len() == 1
            && matches!(arms[0].0, crate::Pattern::Wildcard)
        {
            out.push(Lint {
                code: "L0095".into(),
                message: "`match` with a single `_ =>` arm is equivalent to the arm's body \
                           directly — remove the `match` or add meaningful patterns"
                    .to_string(),
                line: span.start.line as u32,
                column: span.start.column as u32,
                severity: Severity::Warning,
            });
        }
    });
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn lint(src: &str) -> Vec<Lint> {
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        check(&program, src)
    }

    fn codes(src: &str) -> Vec<String> {
        lint(src).into_iter().map(|l| l.code).collect()
    }

    // ---------- L0001: unused local binding ----------

    #[test]
    fn l0001_fires_on_unused_local() {
        let src = "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n";
        assert!(codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_silent_when_local_is_used() {
        let src = "fn f(int a) {\n    let used = a + 1;\n    return used;\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_silent_for_underscore_prefix() {
        let src = "fn f(int a) {\n    let _ignored = 42;\n    return a;\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_suppressed_by_allow_comment() {
        let src = "fn f(int a) {\n    // resilient: allow L0001\n    let unused = 42;\n    return a;\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_fires_on_unused_for_in_loop_variable() {
        let src = "fn f() {\n    let arr = [1, 2, 3];\n    for item in arr {\n        return 1;\n    }\n}\n";
        assert!(codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_silent_when_for_in_loop_variable_is_used() {
        let src = "fn f() {\n    let arr = [1, 2, 3];\n    for item in arr {\n        return item;\n    }\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_silent_for_underscore_prefixed_for_in_variable() {
        let src = "fn f() {\n    let arr = [1, 2, 3];\n    for _item in arr {\n        return 1;\n    }\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    // ---------- L0002: unreachable arm after _ ----------

    #[test]
    fn l0002_fires_on_arm_after_wildcard() {
        let src =
            "fn f(int n) {\n    return match n {\n        _ => 0,\n        1 => 1,\n    };\n}\n";
        assert!(codes(src).contains(&"L0002".to_string()));
    }

    #[test]
    fn l0002_silent_when_wildcard_is_last() {
        let src =
            "fn f(int n) {\n    return match n {\n        1 => 1,\n        _ => 0,\n    };\n}\n";
        assert!(!codes(src).contains(&"L0002".to_string()));
    }

    #[test]
    fn l0002_suppressed_by_allow_comment() {
        // The lint reports at the unreachable arm's body span,
        // so the allow comment goes on the line just above THAT
        // arm, not above the `match` keyword.
        let src = "fn f(int n) {\n    return match n {\n        _ => 0,\n        // resilient: allow L0002\n        1 => 1,\n    };\n}\n";
        assert!(!codes(src).contains(&"L0002".to_string()));
    }

    // ---------- L0002 / RES-232: Pattern::Bind as catch-all ----------

    #[test]
    fn l0002_fires_on_bind_with_wildcard_inner() {
        // `n @ _` is a catch-all; the arm after it is unreachable.
        let src = "fn f(int n) {\n    return match n {\n        n @ _ => 1,\n        0 => 2,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0002".to_string()),
            "expected L0002 for bind-with-wildcard-inner"
        );
    }

    #[test]
    fn l0002_fires_on_bind_with_identifier_inner() {
        // `n @ m` — inner is an identifier, also a catch-all.
        let src = "fn f(int n) {\n    return match n {\n        n @ m => 1,\n        0 => 2,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0002".to_string()),
            "expected L0002 for bind-with-identifier-inner"
        );
    }

    #[test]
    fn l0002_silent_on_bind_with_literal_inner() {
        // `n @ 5` is NOT a catch-all — it only matches the value 5.
        let src = "fn f(int n) {\n    return match n {\n        n @ 5 => 1,\n        0 => 2,\n    };\n}\n";
        assert!(
            !codes(src).contains(&"L0002".to_string()),
            "L0002 must not fire for bind-with-literal-inner"
        );
    }

    // ---------- L0003: x == x ----------

    #[test]
    fn l0003_fires_on_self_eq() {
        let src = "fn f(int x) {\n    if x == x { return 1; }\n    return 0;\n}\n";
        assert!(codes(src).contains(&"L0003".to_string()));
    }

    #[test]
    fn l0003_fires_on_self_ne() {
        let src = "fn f(int x) {\n    if x != x { return 1; }\n    return 0;\n}\n";
        assert!(codes(src).contains(&"L0003".to_string()));
    }

    #[test]
    fn l0003_silent_on_distinct_operands() {
        let src = "fn f(int x, int y) {\n    if x == y { return 1; }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0003".to_string()));
    }

    #[test]
    fn l0003_suppressed_by_allow_comment() {
        let src = "fn f(int x) {\n    // resilient: allow L0003\n    if x == x { return 1; }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0003".to_string()));
    }

    // ---------- L0004: mixed && / || ----------

    #[test]
    fn l0004_fires_on_and_or_mix() {
        let src =
            "fn f(bool a, bool b, bool c) {\n    if a && b || c { return 1; }\n    return 0;\n}\n";
        assert!(codes(src).contains(&"L0004".to_string()));
    }

    #[test]
    fn l0004_silent_on_same_op() {
        let src =
            "fn f(bool a, bool b, bool c) {\n    if a && b && c { return 1; }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0004".to_string()));
    }

    #[test]
    fn l0004_suppressed_by_allow_comment() {
        let src = "fn f(bool a, bool b, bool c) {\n    // resilient: allow L0004\n    if a && b || c { return 1; }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0004".to_string()));
    }

    // ---------- L0005: redundant trailing return ----------

    #[test]
    fn l0005_fires_on_trailing_bare_return() {
        let src = "fn f() {\n    let x = 1;\n    return;\n}\n";
        assert!(codes(src).contains(&"L0005".to_string()));
    }

    #[test]
    fn l0005_silent_when_return_has_value() {
        let src = "fn f() {\n    return 1;\n}\n";
        assert!(!codes(src).contains(&"L0005".to_string()));
    }

    #[test]
    fn l0005_silent_when_no_return_stmt() {
        let src = "fn f() {\n    let x = 1;\n}\n";
        assert!(!codes(src).contains(&"L0005".to_string()));
    }

    #[test]
    fn l0005_suppressed_by_allow_comment() {
        let src = "fn f() {\n    let x = 1;\n    // resilient: allow L0005\n    return;\n}\n";
        // The allow is on the line directly above the bare `return;`.
        assert!(!codes(src).contains(&"L0005".to_string()));
    }

    // ---------- allow-comment parsing ----------

    #[test]
    fn allow_comment_accepts_multiple_codes_per_line() {
        let src = "fn f(int a) {\n    // resilient: allow L0001, L0005\n    let unused = 42;\n    return;\n}\n";
        // Both L0001 and L0005 should be silenced.
        let c = codes(src);
        // L0001 would fire at the `let` line (line 3).
        assert!(!c.contains(&"L0001".to_string()));
    }

    #[test]
    fn allow_comment_ignores_non_l_codes() {
        // "E0008" or "W0001" shouldn't be treated as an L code.
        let allows = collect_allow_comments("// resilient: allow E0008\n");
        assert!(allows.is_empty());
    }

    // ---------- format_lint ----------

    #[test]
    fn format_lint_uses_path_line_col_format() {
        let l = Lint {
            code: "L0001".into(),
            severity: Severity::Warning,
            message: "unused".into(),
            line: 5,
            column: 9,
        };
        let s = format_lint(&l, "src/thing.rs");
        assert_eq!(s, "src/thing.rs:5:9: warning[L0001]: unused");
    }

    #[test]
    fn known_codes_contains_all_five() {
        for code in ["L0001", "L0002", "L0003", "L0004", "L0005"] {
            assert!(KNOWN_CODES.contains(&code), "missing code: {code}");
        }
    }

    // ---------- composite ----------

    #[test]
    fn lints_sorted_by_line_column() {
        let src =
            "fn f(int x) {\n    if x == x { return 1; }\n    let unused = 42;\n    return 0;\n}\n";
        let out = lint(src);
        for pair in out.windows(2) {
            assert!(
                (pair[0].line, pair[0].column) <= (pair[1].line, pair[1].column),
                "lint order: {:?}",
                out,
            );
        }
    }

    #[test]
    fn empty_program_produces_no_lints() {
        assert!(lint("").is_empty());
    }

    // ---------- L0006: assume(false) vacuous discharge ----------

    #[test]
    fn l0006_fires_on_assume_false() {
        let src = "fn f() {\n    assume(false);\n}\n";
        assert!(codes(src).contains(&"L0006".to_string()));
    }

    #[test]
    fn l0006_silent_on_assume_true() {
        let src = "fn f() {\n    assume(true);\n}\n";
        assert!(!codes(src).contains(&"L0006".to_string()));
    }

    #[test]
    fn l0006_silent_on_assume_expr() {
        let src = "fn f(int x) {\n    assume(x > 0);\n}\n";
        assert!(!codes(src).contains(&"L0006".to_string()));
    }

    #[test]
    fn l0006_suppressed_by_allow_comment() {
        let src = "fn f() {\n    // resilient: allow L0006\n    assume(false);\n}\n";
        assert!(!codes(src).contains(&"L0006".to_string()));
    }

    #[test]
    fn known_codes_contains_l0006() {
        assert!(
            KNOWN_CODES.contains(&"L0006"),
            "L0006 missing from KNOWN_CODES"
        );
    }

    #[test]
    fn known_codes_contains_l0007() {
        assert!(
            KNOWN_CODES.contains(&"L0007"),
            "L0007 missing from KNOWN_CODES"
        );
    }

    // ---------- L0007: unreachable code after return ----------

    #[test]
    fn l0007_fires_on_stmt_after_return() {
        // Two statements follow the return; only the first is flagged.
        let src = "fn f(int x) {\n    return x;\n    let a = 1;\n    let b = 2;\n}\n";
        let hits: Vec<_> = lint(src)
            .into_iter()
            .filter(|l| l.code == "L0007")
            .collect();
        assert_eq!(hits.len(), 1, "expected exactly one L0007 warning");
        assert_eq!(
            hits[0].line, 3,
            "warning should point to the first unreachable statement"
        );
    }

    #[test]
    fn l0007_silent_on_normal_flow() {
        let src = "fn f(int x) {\n    let a = x + 1;\n    return a;\n}\n";
        assert!(!codes(src).contains(&"L0007".to_string()));
    }

    #[test]
    fn l0007_silent_when_return_is_last() {
        let src = "fn f() {\n    return;\n}\n";
        assert!(!codes(src).contains(&"L0007".to_string()));
    }

    #[test]
    fn l0007_suppressed_by_allow_comment() {
        // The allow comment goes on the line above the first unreachable statement.
        let src =
            "fn f(int x) {\n    return x;\n    // resilient: allow L0007\n    let a = 1;\n}\n";
        assert!(!codes(src).contains(&"L0007".to_string()));
    }

    #[test]
    fn l0007_does_not_fire_for_return_inside_nested_block() {
        // A `return` inside an `if` branch does not make code after the `if` unreachable.
        let src = "fn f(int x) {\n    if x > 0 {\n        return x;\n    }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0007".to_string()));
    }

    // ---------- L0008: duplicate identical struct literal match arm (RES-369) ----------

    #[test]
    fn l0008_fires_on_duplicate_struct_literal_arm() {
        // Two arms with the same struct + same literal field values — the
        // second can never fire.
        let src = "\
            struct Point { int x, int y }\n\
            fn f(int _d) -> int {\n\
                let p = new Point { x: 0, y: 0 };\n\
                return match p {\n\
                    Point { x: 0, y: 0 } => 1,\n\
                    Point { x: 0, y: 0 } => 2,\n\
                    Point { .. } => 3,\n\
                };\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0008".to_string()),
            "L0008 must fire when two arms have identical struct literal patterns; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0008_silent_when_arms_differ() {
        // Two arms with the same struct but different field values do not
        // overlap.
        let src = "\
            struct Point { int x, int y }\n\
            fn f(int _d) -> int {\n\
                let p = new Point { x: 1, y: 2 };\n\
                return match p {\n\
                    Point { x: 0, y: 0 } => 1,\n\
                    Point { x: 1, y: 1 } => 2,\n\
                    Point { .. } => 3,\n\
                };\n\
            }\n\
        ";
        assert!(
            !codes(src).contains(&"L0008".to_string()),
            "L0008 must not fire when struct literal arms have different field values"
        );
    }

    #[test]
    fn l0008_silent_for_rest_pattern() {
        // `Point { .. }` is a wildcard, not a duplicate literal pattern —
        // two `Point { .. }` arms do NOT trigger L0008; the second is
        // caught by L0002 (arm after catch-all) instead.
        let src = "\
            struct Point { int x, int y }\n\
            fn f(int _d) -> int {\n\
                let p = new Point { x: 0, y: 0 };\n\
                return match p {\n\
                    Point { x: 0, y: 0 } => 1,\n\
                    Point { .. } => 2,\n\
                    Point { .. } => 3,\n\
                };\n\
            }\n\
        ";
        assert!(
            !codes(src).contains(&"L0008".to_string()),
            "L0008 must not fire for rest/wildcard struct arms"
        );
    }

    #[test]
    fn known_codes_contains_l0008() {
        assert!(
            KNOWN_CODES.contains(&"L0008"),
            "L0008 missing from KNOWN_CODES"
        );
    }

    // ---------- RES-237: L0001 false-positives for Assume / MapLiteral /
    // SetLiteral / LetDestructureStruct ----------

    #[test]
    fn l0001_no_false_positive_in_assume_condition() {
        // `x` is read inside assume() — must not fire L0001.
        let src = "fn f(int x) {\n    let y = x + 1;\n    assume(y > 0);\n    return y;\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when local is used inside assume()"
        );
    }

    #[test]
    fn l0001_no_false_positive_in_map_literal_key() {
        // `key` is a let binding that is used only as a map key.
        // Before RES-237 this fired a false L0001 because MapLiteral
        // was not visited by collect_identifier_reads_in.
        let src = "fn f(int n) -> Int {\n    let key = n + 1;\n    let m = {key -> 0};\n    return map_len(m);\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when let binding is used as a map key"
        );
    }

    #[test]
    fn l0001_no_false_positive_in_set_literal_item() {
        // `elem` is a let binding that is used only inside a set literal.
        // Before RES-237 this fired a false L0001 because SetLiteral
        // was not visited by collect_identifier_reads_in.
        let src = "fn f(int n) -> Int {\n    let elem = n + 1;\n    let s = #{elem};\n    return set_len(s);\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when let binding is used inside a set literal"
        );
    }

    #[test]
    fn l0001_fires_for_unused_struct_destructure_binding() {
        // `b` is bound by destructure but never read.
        let src = "\
            struct Pt { int x, int y }\n\
            fn f(int d) -> Int {\n\
                let p = new Pt { x: 1, y: 2 };\n\
                let Pt { x: a, y: b } = p;\n\
                return a;\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire for unused struct-destructure binding"
        );
    }

    #[test]
    fn l0001_silent_for_used_struct_destructure_binding() {
        // Both `a` and `b` are read after destructuring.
        let src = "\
            struct Pt { int x, int y }\n\
            fn f(int d) -> Int {\n\
                let p = new Pt { x: 3, y: 4 };\n\
                let Pt { x: a, y: b } = p;\n\
                return a + b;\n\
            }\n\
        ";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when all struct-destructure bindings are used"
        );
    }

    // ---------- RES-239: lint passes walk impl block methods ----------

    #[test]
    fn l0001_fires_for_unused_binding_in_impl_method() {
        // `unused` is declared but never read inside a method body.
        let src = "\
            struct Counter { int n }\n\
            impl Counter {\n\
                fn tick(self) -> int {\n\
                    let unused = 99;\n\
                    return self.n;\n\
                }\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire for unused binding inside an impl method"
        );
    }

    // ---------- RES-259: L0001 fires on unused match-arm bindings ----------

    #[test]
    fn l0001_fires_on_unused_match_arm_binding() {
        // `y` is bound by the pattern but never used in the arm body.
        let src = "fn f(int x) -> int {\n    return match x {\n        y => 1,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire when a match-arm pattern binding is never used"
        );
    }

    #[test]
    fn l0001_silent_when_match_arm_binding_is_used() {
        // `y` is bound and then returned from the arm body.
        let src = "fn f(int x) -> int {\n    return match x {\n        y => y,\n    };\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when a match-arm pattern binding is used"
        );
    }

    #[test]
    fn l0001_silent_for_underscore_prefixed_match_arm_binding() {
        // `_y` starts with `_` — explicitly silenced per convention.
        let src = "fn f(int x) -> int {\n    return match x {\n        _y => 1,\n    };\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire for underscore-prefixed match-arm binding"
        );
    }

    #[test]
    fn l0001_fires_on_unused_bind_pattern_name() {
        // `n @ _`: `n` is bound but never used in the arm body.
        let src = "fn f(int x) -> int {\n    return match x {\n        n @ _ => 1,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire when the name in a bind pattern (name @ inner) is unused"
        );
    }

    #[test]
    fn l0002_fires_for_unreachable_arm_in_impl_method() {
        // An arm after `_` inside a method is unreachable.
        let src = "\
            struct Wrapper { int v }\n\
            impl Wrapper {\n\
                fn kind(self) -> int {\n\
                    return match self.v {\n\
                        _ => 0,\n\
                        1 => 1,\n\
                    };\n\
                }\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0002".to_string()),
            "L0002 must fire for unreachable arm inside an impl method"
        );
    }

    // ---------- RES-350: L0009 integer division by zero ----------

    #[test]
    fn l0009_fires_on_literal_integer_divisor() {
        // The non-Z3 baseline: literal 0 always fires.
        let src = "fn f(int a) -> int {\n    return a / 0;\n}\n";
        assert!(
            codes(src).contains(&"L0009".to_string()),
            "L0009 must fire on literal-zero integer divisor; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0009_fires_on_literal_modulo_divisor() {
        let src = "fn f(int a) -> int {\n    return a % 0;\n}\n";
        assert!(
            codes(src).contains(&"L0009".to_string()),
            "L0009 must fire on literal-zero modulo divisor"
        );
    }

    #[test]
    fn l0009_silent_on_literal_nonzero_divisor() {
        let src = "fn f(int a) -> int {\n    return a / 2;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must not fire on literal-nonzero divisor"
        );
    }

    #[test]
    #[cfg(not(feature = "z3"))]
    fn l0009_silent_on_identifier_divisor_without_z3() {
        // Without Z3, identifier divisors are silent — we only
        // flag statically-obvious literal-zero bugs.
        let src = "fn f(int a, int b) -> int {\n    return a / b;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must stay silent on identifier divisors without the z3 feature"
        );
    }

    #[test]
    fn l0009_suppressed_by_allow_comment() {
        let src = "fn f(int a) -> int {\n    // resilient: allow L0009\n    return a / 0;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must be suppressed by allow comment"
        );
    }

    #[test]
    fn known_codes_contains_l0009() {
        assert!(
            KNOWN_CODES.contains(&"L0009"),
            "L0009 missing from KNOWN_CODES"
        );
    }

    #[test]
    #[cfg(feature = "z3")]
    fn l0009_fires_on_unconstrained_identifier_divisor_with_z3() {
        // Under the z3 feature, a divisor with no precondition is
        // flagged because the solver cannot prove it non-zero.
        let src = "fn f(int a, int b) -> int {\n    return a / b;\n}\n";
        assert!(
            codes(src).contains(&"L0009".to_string()),
            "L0009 must fire when z3 cannot prove divisor non-zero"
        );
    }

    #[test]
    #[cfg(feature = "z3")]
    fn l0009_silent_when_precondition_guarantees_nonzero() {
        // `requires b != 0;` gives Z3 enough to prove the obligation.
        let src = "fn f(int a, int b) -> int requires b != 0 {\n    return a / b;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must stay silent when preconditions prove divisor non-zero; got {:?}",
            codes(src)
        );
    }

    #[test]
    #[cfg(feature = "z3")]
    fn l0009_silent_when_precondition_forces_strictly_positive() {
        // `requires b > 0;` also implies `b != 0`.
        let src = "fn f(int a, int b) -> int requires b > 0 {\n    return a / b;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must stay silent when b > 0 implies b != 0"
        );
    }

    #[test]
    fn l0005_fires_for_trailing_return_in_impl_method() {
        // A trailing bare `return;` inside a method is redundant.
        let src = "\
            struct Noop { int x }\n\
            impl Noop {\n\
                fn run(self) {\n\
                    let _v = self.x;\n\
                    return;\n\
                }\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0005".to_string()),
            "L0005 must fire for trailing bare return inside an impl method"
        );
    }

    // ---------- L0010: no requires/ensures contract ----------

    #[test]
    fn l0010_fires_on_fn_with_no_contract() {
        let src = "fn f(int x) { return x; }\n";
        assert!(
            codes(src).contains(&"L0010".to_string()),
            "L0010 must fire when a function has no requires/ensures contract"
        );
    }

    #[test]
    fn l0010_silent_when_requires_present() {
        let src = "fn f(int x) requires x > 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0010".to_string()),
            "L0010 must stay silent when function has a requires clause"
        );
    }

    #[test]
    fn l0010_silent_when_ensures_present() {
        let src = "fn f(int x) -> int ensures result >= 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0010".to_string()),
            "L0010 must stay silent when function has an ensures clause"
        );
    }

    #[test]
    fn l0010_silent_for_underscore_prefixed_fns() {
        let src = "fn _helper(int x) { return x; }\n";
        assert!(
            !codes(src).contains(&"L0010".to_string()),
            "L0010 must not fire on underscore-prefixed function names"
        );
    }

    #[test]
    fn l0010_allow_comment_suppresses() {
        let src = "// resilient: allow L0010\nfn f(int x) { return x; }\n";
        assert!(
            !codes(src).contains(&"L0010".to_string()),
            "L0010 must be suppressible with allow comment"
        );
    }

    #[test]
    fn l0010_in_known_codes() {
        assert!(
            KNOWN_CODES.contains(&"L0010"),
            "L0010 must appear in KNOWN_CODES"
        );
    }

    // ---------- RES-308 / L0011: unused variable warning ----------

    #[test]
    fn l0011_fires_on_unused_let() {
        // `unused` is bound and never read — must produce L0011.
        let src = "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n";
        assert!(
            codes(src).contains(&"L0011".to_string()),
            "L0011 must fire on a `let` binding whose name is never read"
        );
    }

    #[test]
    fn l0011_silent_when_let_is_used() {
        // Used `let` — no L0011.
        let src = "fn f(int a) {\n    let used = a + 1;\n    return used;\n}\n";
        assert!(
            !codes(src).contains(&"L0011".to_string()),
            "L0011 must not fire when the let binding is read"
        );
    }

    #[test]
    fn l0011_silent_for_underscore_prefix() {
        // Underscore-prefixed names are exempt by convention.
        let src = "fn f(int a) {\n    let _temp = 42;\n    return a;\n}\n";
        assert!(
            !codes(src).contains(&"L0011".to_string()),
            "L0011 must not fire for `_`-prefixed bindings"
        );
    }

    #[test]
    fn l0011_message_matches_ticket_format() {
        // RES-308 specifies the exact rustc-style phrasing.
        let src = "fn f(int a) {\n    let zzz = 42;\n    return a;\n}\n";
        let lints = lint(src);
        let l0011 = lints
            .iter()
            .find(|l| l.code == "L0011")
            .expect("expected L0011 to fire");
        assert_eq!(
            l0011.message, "variable `zzz` is assigned but never used",
            "L0011 message must match the RES-308 acceptance criteria"
        );
        assert_eq!(l0011.severity, Severity::Warning);
    }

    #[test]
    fn l0011_reports_at_let_span() {
        // `let unused = 42;` is on line 2, indent 4 — column 5.
        let src = "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n";
        let lints = lint(src);
        let l0011 = lints
            .iter()
            .find(|l| l.code == "L0011")
            .expect("expected L0011 to fire");
        assert_eq!(l0011.line, 2, "L0011 must report at the let line");
    }

    #[test]
    fn l0011_suppressed_by_allow_comment() {
        let src = "fn f(int a) {\n    // resilient: allow L0011\n    let unused = 42;\n    return a;\n}\n";
        assert!(
            !codes(src).contains(&"L0011".to_string()),
            "L0011 must be suppressible with `// resilient: allow L0011`"
        );
    }

    #[test]
    fn l0011_in_known_codes() {
        // The CLI validates --deny / --allow against KNOWN_CODES;
        // missing L0011 here would silently reject `--deny L0011`.
        assert!(
            KNOWN_CODES.contains(&"L0011"),
            "L0011 must appear in KNOWN_CODES so --deny/--allow accept it"
        );
    }

    #[test]
    fn l0011_deny_escalates_to_error() {
        // Mirrors the L0001 escalation path — `--deny L0011` should
        // bump severity to Error. This unit test simulates the flag
        // by mutating severity directly; the `lint_smoke.rs`
        // integration test exercises the CLI plumbing end-to-end.
        let src = "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n";
        let mut lints = lint(src);
        for l in lints.iter_mut() {
            if l.code == "L0011" {
                l.severity = Severity::Error;
            }
        }
        let l0011 = lints
            .iter()
            .find(|l| l.code == "L0011")
            .expect("expected L0011 to fire");
        assert_eq!(
            l0011.severity,
            Severity::Error,
            "L0011 must escalate to Error under --deny L0011"
        );
    }

    #[test]
    fn l0011_fires_inside_impl_method() {
        // Same as RES-239 coverage for L0001 — impl-block methods
        // must be walked.
        let src = "struct S {}\nimpl S {\n    fn m(self) {\n        let unused = 42;\n        return;\n    }\n}\n";
        assert!(
            codes(src).contains(&"L0011".to_string()),
            "L0011 must fire for unused let inside an impl method"
        );
    }

    #[test]
    fn l0011_silent_when_used_inside_live_block() {
        // The ticket explicitly notes that vars used only inside a
        // `live` block retry path are NOT exempt — but a var that
        // IS read inside a `live` body must NOT fire L0011.
        let src = "fn f() {\n    let x = 1;\n    live { return x; }\n}\n";
        assert!(
            !codes(src).contains(&"L0011".to_string()),
            "L0011 must treat reads inside a `live` body as uses"
        );
    }

    // ---------- RES-397 / L0012: spec provenance ----------

    #[test]
    fn l0012_in_known_codes() {
        assert!(
            KNOWN_CODES.contains(&"L0012"),
            "L0012 must appear in KNOWN_CODES so --deny/--allow accept it"
        );
    }

    #[test]
    fn l0012_fires_on_function_with_requires_but_no_source() {
        let src = "fn f(int x) requires x > 0 { return x; }\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must fire on a fn with requires but no `// source:` comment"
        );
    }

    #[test]
    fn l0012_fires_on_function_with_ensures_but_no_source() {
        let src = "fn f(int x) ensures result > 0 { return x; }\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must fire on a fn with ensures but no `// source:` comment"
        );
    }

    #[test]
    fn l0012_silent_when_source_comment_present() {
        let src = "// source: RFC 9293 §3.5\nfn f(int x) requires x > 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must not fire when `// source:` precedes the spec-bearing fn"
        );
    }

    #[test]
    fn l0012_silent_for_function_without_spec() {
        // A fn with neither requires nor ensures is L0010's territory,
        // not L0012's. L0012 only fires when there IS a spec.
        let src = "fn f(int x) { return x; }\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must not fire on a fn without any spec annotation"
        );
    }

    #[test]
    fn l0012_silent_for_underscore_prefixed_function() {
        let src = "fn _helper(int x) requires x > 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must not fire on `_`-prefixed helper fns"
        );
    }

    #[test]
    fn l0012_fires_on_assume_without_source() {
        let src = "fn f(int x) {\n    assume(x > 0);\n    return x;\n}\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must fire on `assume()` without a preceding `// source:` comment"
        );
    }

    #[test]
    fn l0012_silent_when_source_comment_precedes_assume() {
        let src = "fn f(int x) {\n    // source: derived from caller's domain\n    assume(x > 0);\n    return x;\n}\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must not fire on `assume()` with a `// source:` line above"
        );
    }

    #[test]
    fn l0012_suppressed_by_allow_comment() {
        let src = "// resilient: allow L0012\nfn f(int x) requires x > 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must be suppressible with `// resilient: allow L0012`"
        );
    }

    #[test]
    fn l0012_empty_source_comment_does_not_satisfy() {
        // `// source:` with nothing after the colon must not
        // count — the whole point is to require a real reference.
        let src = "// source:   \nfn f(int x) requires x > 0 { return x; }\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must still fire when `// source:` is empty"
        );
    }

    #[test]
    fn l0012_fires_on_recovers_to() {
        // RES-387: fns with `fails` + `recovers_to:` are spec-bearing too.
        let src = "fn f(int x) fails Bad recovers_to: x > 0; { return x; }\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must fire on a fn with `recovers_to:` but no `// source:` comment"
        );
    }

    // ---- L0014 tests ----

    #[test]
    fn l0014_defined_but_never_called() {
        let src = "fn helper(int x) -> int { return x; }\nfn main() { let _y = 1; }\nmain();\n";
        assert!(
            codes(src).contains(&"L0014".to_string()),
            "L0014 must fire for `helper` which is defined but never called"
        );
    }

    #[test]
    fn l0014_called_function_not_flagged() {
        let src =
            "fn helper(int x) -> int { return x; }\nfn main() { let _y = helper(1); }\nmain();\n";
        assert!(
            !codes(src).contains(&"L0014".to_string()),
            "L0014 must not fire when the function is actually called"
        );
    }

    #[test]
    fn l0014_underscore_prefix_not_flagged() {
        let src = "fn _unused(int x) -> int { return x; }\nfn main() {}\nmain();\n";
        assert!(
            !codes(src).contains(&"L0014".to_string()),
            "L0014 must not fire for `_`-prefixed functions"
        );
    }

    #[test]
    fn l0014_main_not_flagged_even_if_only_at_top_level() {
        // `main` called at top level (as a statement) should not be flagged.
        let src = "fn main() { let _x = 1; }\nmain();\n";
        assert!(
            !codes(src).contains(&"L0014".to_string()),
            "L0014 must not fire for `main` when called at top level"
        );
    }

    // ---- L0015: constant integer overflow ----

    #[test]
    fn l0015_fires_on_addition_overflow() {
        // 9223372036854775807 + 1 overflows i64.
        let src = "fn f() -> int { return 9223372036854775807 + 1; }\nf();\n";
        assert!(
            codes(src).contains(&"L0015".to_string()),
            "L0015 must fire when literal addition overflows i64; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0015_fires_on_multiplication_overflow() {
        let src = "fn f() -> int { return 1000000000 * 1000000000000000; }\nf();\n";
        assert!(
            codes(src).contains(&"L0015".to_string()),
            "L0015 must fire on multiplication overflow"
        );
    }

    #[test]
    fn l0015_silent_for_non_overflowing_expression() {
        let src = "fn f() -> int { return 100 + 200; }\nf();\n";
        assert!(
            !codes(src).contains(&"L0015".to_string()),
            "L0015 must not fire for non-overflowing constant arithmetic"
        );
    }

    #[test]
    fn l0015_silent_when_operand_is_variable() {
        // `x + 1` — not fully constant, so overflow cannot be proven.
        let src = "fn f(int x) -> int { return x + 1; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0015".to_string()),
            "L0015 must not fire when an operand is a variable"
        );
    }

    // ---- L0016: constant boolean condition ----

    #[test]
    fn l0016_fires_on_literal_true_condition() {
        let src = "fn f() { if true { let _x = 1; } }\nf();\n";
        assert!(
            codes(src).contains(&"L0016".to_string()),
            "L0016 must fire when `if` condition is literal `true`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0016_fires_on_literal_false_condition() {
        let src = "fn f() { if false { let _x = 1; } }\nf();\n";
        assert!(
            codes(src).contains(&"L0016".to_string()),
            "L0016 must fire when `if` condition is literal `false`"
        );
    }

    #[test]
    fn l0016_fires_on_constant_comparison() {
        // `1 < 2` is always true — equivalent to a literal `true`.
        let src = "fn f() { if 1 < 2 { let _x = 1; } }\nf();\n";
        assert!(
            codes(src).contains(&"L0016".to_string()),
            "L0016 must fire when condition folds to a constant bool"
        );
    }

    #[test]
    fn l0016_silent_for_variable_condition() {
        let src = "fn f(int x) -> int { if x > 0 { return 1; } return 0; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0016".to_string()),
            "L0016 must not fire when condition involves a variable"
        );
    }

    // ---- L0017: variable shadowing ----

    #[test]
    fn l0017_fires_when_let_shadows_outer_let() {
        // Inner `let x` shadows outer `let x`.
        let src = "fn f(int n) -> int {\n    let x = n;\n    if n > 0 {\n        let x = n + 1;\n        return x;\n    }\n    return x;\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0017".to_string()),
            "L0017 must fire when inner let shadows outer let; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0017_fires_when_let_shadows_parameter() {
        // `let n` shadows parameter `n`.
        let src = "fn f(int n) -> int {\n    let n = n + 1;\n    return n;\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0017".to_string()),
            "L0017 must fire when let shadows a parameter"
        );
    }

    #[test]
    fn l0017_silent_for_underscore_prefix() {
        // `_x` is exempt — underscore prefix signals intentional shadowing.
        let src = "fn f(int n) -> int {\n    let x = n;\n    if n > 0 {\n        let _x = n + 1;\n        return _x;\n    }\n    return x;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0017".to_string()),
            "L0017 must not fire for `_`-prefixed bindings"
        );
    }

    #[test]
    fn l0017_silent_when_no_shadowing() {
        // `y` is a new name, not a shadow.
        let src = "fn f(int n) -> int {\n    let x = n;\n    if n > 0 {\n        let y = n + 1;\n        return y;\n    }\n    return x;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0017".to_string()),
            "L0017 must not fire when names are distinct"
        );
    }

    // ---- L0018: missing return on all paths ----

    #[test]
    fn l0018_fires_when_if_without_else_is_last() {
        // Return type is `int` but the if-without-else path falls through.
        let src = "fn f(int x) -> int {\n    if x > 0 { return 1; }\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0018".to_string()),
            "L0018 must fire when fn with return type lacks an else branch; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0018_silent_when_all_paths_return() {
        // Both branches of the if/else return, so all paths are covered.
        let src = "fn f(int x) -> int {\n    if x > 0 { return 1; } else { return 0; }\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0018".to_string()),
            "L0018 must not fire when every path ends with return"
        );
    }

    #[test]
    fn l0018_silent_for_void_function() {
        // No return type annotation — void function, L0018 does not apply.
        let src = "fn f(int x) {\n    if x > 0 { let _y = 1; }\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0018".to_string()),
            "L0018 must not fire for functions with no return type"
        );
    }

    #[test]
    fn l0018_silent_when_return_at_end_of_body() {
        // Unconditional return at end of body covers all paths.
        let src = "fn f(int x) -> int {\n    let y = x + 1;\n    return y;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0018".to_string()),
            "L0018 must not fire when body ends with an unconditional return"
        );
    }

    // ---- L0019: format() arity mismatch ----

    #[test]
    fn l0019_fires_on_missing_args_array() {
        // format() called with only 1 argument (missing args array).
        let src = "fn f() { let _s = format(\"hello\"); }\nf();\n";
        assert!(
            codes(src).contains(&"L0019".to_string()),
            "L0019 must fire when format() has only 1 argument; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0019_fires_on_too_many_toplevel_args() {
        // format() called with 3 arguments instead of 2.
        let src = "fn f() { let _s = format(\"hello\", [], []); }\nf();\n";
        assert!(
            codes(src).contains(&"L0019".to_string()),
            "L0019 must fire when format() has 3+ arguments"
        );
    }

    #[test]
    fn l0019_fires_on_placeholder_array_mismatch() {
        // Template `\{} \{}` has 2 placeholders but array has 1 element.
        // Rust string "\{} \{}" encodes the Resilient source `\{} \{}`.
        let src = "fn f() { let _s = format(\"\\{} \\{}\", [1]); }\nf();\n";
        assert!(
            codes(src).contains(&"L0019".to_string()),
            "L0019 must fire when placeholder count != array length; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0019_silent_on_exact_placeholder_match() {
        // Template `\{}` has 1 placeholder and array has 1 element.
        let src = "fn f() { let _s = format(\"\\{}\", [42]); }\nf();\n";
        assert!(
            !codes(src).contains(&"L0019".to_string()),
            "L0019 must not fire when placeholder count matches array length"
        );
    }

    #[test]
    fn l0019_silent_for_no_placeholders_empty_array() {
        // Plain string with no placeholders, empty args array — clean.
        let src = "fn f() { let _s = format(\"hello world\", []); }\nf();\n";
        assert!(
            !codes(src).contains(&"L0019".to_string()),
            "L0019 must not fire for template with no placeholders and empty array"
        );
    }

    // ---------- L0020: unused function parameter ----------

    #[test]
    fn l0020_fires_on_unused_parameter() {
        let src = "fn f(int a, int b) -> int { return a; }\nf(1, 2);\n";
        assert!(
            codes(src).contains(&"L0020".to_string()),
            "L0020 must fire when parameter `b` is never used"
        );
    }

    #[test]
    fn l0020_silent_when_all_params_used() {
        let src = "// source: test\nfn f(int a, int b) -> int requires a > 0 && b > 0 { return a + b; }\nf(1, 2);\n";
        assert!(
            !codes(src).contains(&"L0020".to_string()),
            "L0020 must not fire when all parameters are used"
        );
    }

    #[test]
    fn l0020_silent_for_underscore_prefix() {
        let src = "// source: test\nfn f(int a, int _unused) -> int requires a > 0 { return a; }\nf(1, 2);\n";
        assert!(
            !codes(src).contains(&"L0020".to_string()),
            "L0020 must not fire for `_`-prefixed parameters"
        );
    }

    #[test]
    fn l0020_silent_when_param_used_only_in_requires() {
        // Parameter only in `requires` clause counts as used.
        let src =
            "// source: test\nfn f(int a, int b) -> int requires b > 0 { return a; }\nf(1, 2);\n";
        assert!(
            !codes(src).contains(&"L0020".to_string()),
            "L0020 must not fire when parameter used in requires clause"
        );
    }

    // ---------- L0021: redundant boolean sub-expression ----------

    #[test]
    fn l0021_fires_on_x_and_x() {
        let src = "fn f(bool x) -> bool { return x && x; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0021".to_string()),
            "L0021 must fire for `x && x`"
        );
    }

    #[test]
    fn l0021_fires_on_x_or_x() {
        let src = "fn f(bool x) -> bool { return x || x; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0021".to_string()),
            "L0021 must fire for `x || x`"
        );
    }

    #[test]
    fn l0021_silent_for_distinct_operands() {
        let src = "// source: test\nfn f(bool x, bool y) -> bool requires true { return x && y; }\nf(true, false);\n";
        assert!(
            !codes(src).contains(&"L0021".to_string()),
            "L0021 must not fire when operands differ"
        );
    }

    // ---------- L0022: needless else after return ----------

    #[test]
    fn l0022_fires_on_else_after_return() {
        let src = "fn f(int x) -> int { if x > 0 { return 1; } else { return 0; } }\nf(1);\n";
        assert!(
            codes(src).contains(&"L0022".to_string()),
            "L0022 must fire when if-consequence always returns and else is present"
        );
    }

    #[test]
    fn l0022_silent_when_consequence_may_fall_through() {
        // Consequence doesn't always return (loop without return).
        let src = "// source: test\nfn f(int x) -> int requires x > 0 { if x > 0 { let _y = 1; } else { return 0; } return 1; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0022".to_string()),
            "L0022 must not fire when consequence doesn't always return"
        );
    }

    #[test]
    fn l0022_silent_when_no_else() {
        let src = "// source: test\nfn f(int x) -> int requires x > 0 { if x > 0 { return 1; } return 0; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0022".to_string()),
            "L0022 must not fire when there is no else branch"
        );
    }

    // ---------- L0023: tautological comparison with boolean literal ----------

    #[test]
    fn l0023_fires_on_eq_true() {
        let src = "fn f(bool x) -> bool { return x == true; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0023".to_string()),
            "L0023 must fire for `x == true`"
        );
    }

    #[test]
    fn l0023_fires_on_eq_false() {
        let src = "fn f(bool x) -> bool { return x == false; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0023".to_string()),
            "L0023 must fire for `x == false`"
        );
    }

    #[test]
    fn l0023_fires_on_literal_left() {
        let src = "fn f(bool x) -> bool { return true == x; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0023".to_string()),
            "L0023 must fire when literal is on the left side"
        );
    }

    #[test]
    fn l0023_silent_for_non_bool_comparison() {
        let src = "// source: test\nfn f(int x) -> bool requires x > 0 { return x == 1; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0023".to_string()),
            "L0023 must not fire for integer comparisons"
        );
    }

    // ---------- L0025: unreachable code after infinite while-true loop ----------

    #[test]
    fn l0025_fires_on_code_after_while_true() {
        let src =
            "fn f() {\n    while true {\n        let _x = 1;\n    }\n    let dead = 2;\n}\nf();\n";
        assert!(
            codes(src).contains(&"L0025".to_string()),
            "L0025 must fire when code follows while-true with no break"
        );
    }

    #[test]
    fn l0025_silent_when_loop_has_break() {
        let src = "// source: test\nfn f() -> int requires true {\n    while true {\n        break;\n    }\n    return 0;\n}\nf();\n";
        assert!(
            !codes(src).contains(&"L0025".to_string()),
            "L0025 must not fire when while-true loop contains a break"
        );
    }

    #[test]
    fn l0025_silent_for_conditional_while() {
        let src = "// source: test\nfn f(int x) -> int requires x > 0 {\n    while x > 0 {\n        let _y = 1;\n    }\n    return 0;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0025".to_string()),
            "L0025 must not fire for non-literal while condition"
        );
    }

    // ---------- L0024: struct literal missing required fields ----------

    #[test]
    fn l0024_fires_when_field_missing() {
        let src = "struct Point { int x, int y, int z }\nlet _p = new Point { x: 1, y: 2 };\n";
        assert!(
            codes(src).contains(&"L0024".to_string()),
            "L0024 must fire when a struct literal omits a declared field; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0024_silent_when_all_fields_present() {
        let src = "struct Point { int x, int y }\nlet _p = new Point { x: 1, y: 2 };\n";
        assert!(
            !codes(src).contains(&"L0024".to_string()),
            "L0024 must not fire when all declared fields are provided"
        );
    }

    #[test]
    fn l0024_message_names_missing_fields() {
        let src = "struct Rect { int w, int h, int depth }\nlet _r = new Rect { w: 10 };\n";
        let lints = lint(src);
        let l = lints.iter().find(|l| l.code == "L0024");
        assert!(l.is_some(), "L0024 must fire; got {:?}", lints);
        let msg = &l.unwrap().message;
        assert!(
            msg.contains("`h`") && msg.contains("`depth`"),
            "L0024 message must name missing fields; got: {msg}"
        );
    }

    #[test]
    fn l0024_silent_for_unknown_struct_name() {
        // If the struct isn't declared in this program, don't fire.
        let src = "let _p = new Unknown { x: 1 };\n";
        assert!(
            !codes(src).contains(&"L0024".to_string()),
            "L0024 must not fire for unknown struct type"
        );
    }

    // ---------- L0026: duplicate key in map literal ----------

    #[test]
    fn l0026_fires_on_duplicate_string_key() {
        let src = "fn f() { let _m = {\"a\" -> 1, \"b\" -> 2, \"a\" -> 3}; }\nf();\n";
        assert!(
            codes(src).contains(&"L0026".to_string()),
            "L0026 must fire when a string key appears twice; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0026_fires_on_duplicate_integer_key() {
        let src = "fn f() { let _m = {1 -> \"x\", 2 -> \"y\", 1 -> \"z\"}; }\nf();\n";
        assert!(
            codes(src).contains(&"L0026".to_string()),
            "L0026 must fire when an integer key appears twice"
        );
    }

    #[test]
    fn l0026_silent_when_keys_unique() {
        let src = "fn f() { let _m = {\"a\" -> 1, \"b\" -> 2, \"c\" -> 3}; }\nf();\n";
        assert!(
            !codes(src).contains(&"L0026".to_string()),
            "L0026 must not fire when all keys are distinct"
        );
    }

    // ---------- L0027: empty catch block ----------

    #[test]
    fn l0027_fires_on_empty_catch() {
        let src = "fn risky() fails Bad { }\ntry { risky(); } catch Bad { }\n";
        assert!(
            codes(src).contains(&"L0027".to_string()),
            "L0027 must fire for an empty catch block; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0027_silent_when_catch_has_body() {
        let src = "fn risky() fails Bad { }\ntry { risky(); } catch Bad { let _x = 1; }\n";
        assert!(
            !codes(src).contains(&"L0027".to_string()),
            "L0027 must not fire when the catch block has statements"
        );
    }

    // ---------- L0028: negation of boolean literal ----------

    #[test]
    fn l0028_fires_on_not_true() {
        let src = "let _x = !true;\n";
        assert!(
            codes(src).contains(&"L0028".to_string()),
            "L0028 must fire for `!true`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0028_fires_on_not_false() {
        let src = "let _x = !false;\n";
        assert!(
            codes(src).contains(&"L0028".to_string()),
            "L0028 must fire for `!false`"
        );
    }

    #[test]
    fn l0028_silent_for_not_identifier() {
        let src = "fn f(bool x) -> bool { return !x; }\nf(true);\n";
        assert!(
            !codes(src).contains(&"L0028".to_string()),
            "L0028 must not fire for `!identifier`"
        );
    }

    // ---------- L0029: comparison result discarded ----------

    #[test]
    fn l0029_fires_on_discarded_eq() {
        let src = "fn f(int x, int y) { x == y; }\nf(1, 2);\n";
        assert!(
            codes(src).contains(&"L0029".to_string()),
            "L0029 must fire when comparison result is discarded; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0029_fires_on_discarded_lt() {
        let src = "fn f(int x, int y) { x < y; }\nf(1, 2);\n";
        assert!(
            codes(src).contains(&"L0029".to_string()),
            "L0029 must fire when `<` result is discarded"
        );
    }

    #[test]
    fn l0029_silent_when_used_in_if() {
        let src =
            "fn f(int x, int y) -> bool { if x == y { return true; } return false; }\nf(1, 2);\n";
        assert!(
            !codes(src).contains(&"L0029".to_string()),
            "L0029 must not fire when comparison is used as condition"
        );
    }

    // ---------- L0030: float equality comparison ----------

    #[test]
    fn l0030_fires_on_float_eq_zero() {
        let src = "fn f(float x) -> bool { return x == 0.0; }\nf(1.0);\n";
        assert!(
            codes(src).contains(&"L0030".to_string()),
            "L0030 must fire for float == literal; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0030_fires_on_float_neq() {
        let src = "fn f(float x) -> bool { return 1.5 != x; }\nf(1.0);\n";
        assert!(
            codes(src).contains(&"L0030".to_string()),
            "L0030 must fire for float literal != expression"
        );
    }

    #[test]
    fn l0030_silent_for_int_equality() {
        let src = "fn f(int x) -> bool { return x == 0; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0030".to_string()),
            "L0030 must not fire for integer equality"
        );
    }

    // ---------- L0031: double negation ----------

    #[test]
    fn l0031_fires_on_double_not() {
        let src = "fn f(bool x) -> bool { return !!x; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0031".to_string()),
            "L0031 must fire for `!!x`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0031_silent_for_single_not() {
        let src = "fn f(bool x) -> bool { return !x; }\nf(true);\n";
        assert!(
            !codes(src).contains(&"L0031".to_string()),
            "L0031 must not fire for a single negation"
        );
    }

    // ---------- L0033: modulo by literal 1 ----------

    #[test]
    fn l0033_fires_on_modulo_by_one() {
        let src = "fn f(int x) -> int { return x % 1; }\nf(5);\n";
        assert!(
            codes(src).contains(&"L0033".to_string()),
            "L0033 must fire for `x % 1`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0033_silent_for_modulo_by_two() {
        let src = "fn f(int x) -> int { return x % 2; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0033".to_string()),
            "L0033 must not fire for `x % 2`"
        );
    }

    #[test]
    fn l0033_silent_for_modulo_by_zero() {
        // x % 0 is a different lint (L0009 division by zero); L0033 must not double-fire.
        let src = "fn f(int x) -> int { return x % 0; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0033".to_string()),
            "L0033 must not fire for `x % 0` (that is L0009 territory)"
        );
    }

    // ---------- L0034: string concat in loop ----------

    #[test]
    fn l0034_fires_on_string_concat_in_while() {
        let src = "fn f() { let _r = \"\"; while true { let _r = _r + \"chunk\"; } }\nf();\n";
        assert!(
            codes(src).contains(&"L0034".to_string()),
            "L0034 must fire for string `+` inside while; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0034_fires_on_string_concat_in_for() {
        let src = "fn f(Array items) { let _r = \"\"; for x in items { let _r = _r + \"x\"; } }\nf([]);\n";
        assert!(
            codes(src).contains(&"L0034".to_string()),
            "L0034 must fire for string `+` inside for; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0034_silent_for_string_concat_outside_loop() {
        let src = "fn f() -> string { return \"hello\" + \" world\"; }\nf();\n";
        assert!(
            !codes(src).contains(&"L0034".to_string()),
            "L0034 must not fire for string `+` outside any loop"
        );
    }

    #[test]
    fn l0034_silent_for_int_add_in_loop() {
        let src = "fn f(int n) -> int { let _s = 0; while n > 0 { let _s = _s + 1; } return _s; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0034".to_string()),
            "L0034 must not fire for integer `+` in a loop"
        );
    }

    // ---------- L0035: unreachable code after diverging call ----------

    #[test]
    fn l0035_fires_after_exit_call() {
        let src = "fn f(int x) { exit(); let _y = x + 1; }\nf(5);\n";
        assert!(
            codes(src).contains(&"L0035".to_string()),
            "L0035 must fire when code follows exit(); got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0035_fires_after_abort_call() {
        let src = "fn f(int x) { abort(); let _y = x + 1; }\nf(5);\n";
        assert!(
            codes(src).contains(&"L0035".to_string()),
            "L0035 must fire when code follows abort(); got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0035_silent_when_exit_is_last() {
        let src = "fn f(int x) { let _y = x + 1; exit(); }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0035".to_string()),
            "L0035 must not fire when exit() is the last statement in the block"
        );
    }

    #[test]
    fn l0035_silent_for_normal_function_call() {
        let src = "fn g() { } fn f(int x) { g(); let _y = x + 1; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0035".to_string()),
            "L0035 must not fire for a normal (non-diverging) function call"
        );
    }

    // ---------- L0036: len() compared to negative literal ----------

    #[test]
    fn l0036_fires_on_len_lt_negative() {
        let src = "fn f(Array a) -> bool { return len(a) < -1; }\nf([]);\n";
        assert!(
            codes(src).contains(&"L0036".to_string()),
            "L0036 must fire for len(a) < -1; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0036_fires_on_len_eq_negative() {
        let src = "fn f(Array a) -> bool { return len(a) == -1; }\nf([]);\n";
        assert!(
            codes(src).contains(&"L0036".to_string()),
            "L0036 must fire for len(a) == -1; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0036_fires_on_reversed_operands() {
        let src = "fn f(Array a) -> bool { return -1 > len(a); }\nf([]);\n";
        assert!(
            codes(src).contains(&"L0036".to_string()),
            "L0036 must fire when len() is on the right side and negative literal on the left; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0036_silent_for_len_lt_positive() {
        let src = "fn f(Array a) -> bool { return len(a) < 10; }\nf([]);\n";
        assert!(
            !codes(src).contains(&"L0036".to_string()),
            "L0036 must not fire when len() is compared to a positive literal"
        );
    }

    #[test]
    fn l0036_silent_for_len_lt_zero_non_len_call() {
        let src = "fn size(Array a) -> int { return len(a); }\nfn f(Array a) -> bool { return size(a) < 0; }\nf([]);\n";
        assert!(
            !codes(src).contains(&"L0036".to_string()),
            "L0036 must not fire when the function is not literally `len`"
        );
    }

    // ---------- L0037: self-assignment x = x ----------

    #[test]
    fn l0037_fires_on_self_assignment() {
        let src = "fn f(int x) -> int { let y = x; y = y; return y; }\nf(5);\n";
        assert!(
            codes(src).contains(&"L0037".to_string()),
            "L0037 must fire for `y = y`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0037_silent_for_normal_assignment() {
        let src = "fn f(int x) -> int { let y = 0; y = x; return y; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0037".to_string()),
            "L0037 must not fire for `y = x` (different names)"
        );
    }

    #[test]
    fn l0037_silent_for_arithmetic_self_update() {
        let src = "fn f(int x) -> int { let y = x; y = y + 1; return y; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0037".to_string()),
            "L0037 must not fire for `y = y + 1` (not a bare identifier on rhs)"
        );
    }

    // ---------- L0038: panic() in non-test code ----------

    #[test]
    fn l0038_fires_on_panic_call() {
        let src = "fn f(int x) { panic(42); }\nf(1);\n";
        assert!(
            codes(src).contains(&"L0038".to_string()),
            "L0038 must fire when panic() is called; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0038_fires_on_panic_no_args() {
        let src = "fn f(int x) { if x < 0 { panic(0); } }\nf(1);\n";
        assert!(
            codes(src).contains(&"L0038".to_string()),
            "L0038 must fire for panic() inside a branch; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0038_silent_for_non_panic_call() {
        let src = "fn g() { } fn f(int x) { g(); }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0038".to_string()),
            "L0038 must not fire for a normal function call"
        );
    }

    #[test]
    fn l0038_silent_for_abort_call() {
        let src = "fn f(int x) { abort(); }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0038".to_string()),
            "L0038 must not fire for abort() — that is handled by L0035"
        );
    }

    // ---------- L0039: unreachable after @noreturn call ----------

    #[test]
    fn l0039_fires_after_noreturn_call() {
        // The function `die` is marked @noreturn via a comment on the preceding line.
        let src = "// @noreturn\nfn die(int code) { abort(); }\nfn f(int x) { die(1); let _y = x + 1; }\nf(5);\n";
        assert!(
            codes(src).contains(&"L0039".to_string()),
            "L0039 must fire when code follows a @noreturn call; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0039_silent_when_noreturn_call_is_last() {
        let src = "// @noreturn\nfn die(int code) { abort(); }\nfn f(int x) { let _y = x + 1; die(1); }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0039".to_string()),
            "L0039 must not fire when @noreturn call is the last statement in the block"
        );
    }

    #[test]
    fn l0039_silent_for_normal_call_even_if_followed_by_stmts() {
        let src = "fn g(int x) { }\nfn f(int x) { g(x); let _y = x + 1; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0039".to_string()),
            "L0039 must not fire for non-@noreturn function"
        );
    }

    #[test]
    fn l0039_silent_when_no_noreturn_annotation() {
        // No `// @noreturn` comment — even if function is named `die`.
        let src = "fn die(int code) { abort(); }\nfn f(int x) { die(1); let _y = x + 1; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0039".to_string()),
            "L0039 must not fire without @noreturn comment annotation"
        );
    }

    // ---------- L0040: magic number in safety-critical computation ----------

    #[test]
    fn l0040_fires_on_magic_number_in_uncontracted_fn() {
        // 42 is not 0, 1, or a power of two, and the function has no contract.
        let src = "fn f(int x) -> int { return x * 42; }\nf(1);\n";
        assert!(
            codes(src).contains(&"L0040".to_string()),
            "L0040 must fire for magic number 42 in arithmetic; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0040_silent_for_trivial_literal_zero() {
        let src = "fn f(int x) -> int { return x + 0; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0040".to_string()),
            "L0040 must not fire for literal 0"
        );
    }

    #[test]
    fn l0040_silent_for_trivial_literal_one() {
        let src = "fn f(int x) -> int { return x + 1; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0040".to_string()),
            "L0040 must not fire for literal 1"
        );
    }

    #[test]
    fn l0040_silent_for_power_of_two() {
        let src = "fn f(int x) -> int { return x * 8; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0040".to_string()),
            "L0040 must not fire for power-of-two literal 8"
        );
    }

    #[test]
    fn l0040_silent_when_function_has_ensures() {
        // Function has an `ensures` contract — magic number rule is suppressed.
        let src = "fn f(int x) -> int ensures result > 0 { return x * 42; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0040".to_string()),
            "L0040 must not fire when the function has an ensures contract"
        );
    }

    #[test]
    fn l0040_fires_on_non_power_of_two_large_literal() {
        let src = "fn f(int x) -> int { return x + 100; }\nf(1);\n";
        assert!(
            codes(src).contains(&"L0040".to_string()),
            "L0040 must fire for magic number 100; got {:?}",
            codes(src)
        );
    }

    // ---------- L0041: redundant `else` after `if` that always returns ----------

    #[test]
    fn l0041_fires_when_if_always_returns_and_else_present() {
        let src = "fn f(int x) -> int {\n  if x > 0 { return 1; } else { return 0; }\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0041".to_string()),
            "L0041 must fire when if-consequence always returns and else is present; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0041_silent_when_if_does_not_always_return() {
        // consequence doesn't always return — L0041 must not fire
        let src = "fn f(int x) -> int {\n  if x > 0 { let y = 1; } else { return 0; }\n  return 42;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0041".to_string()),
            "L0041 must not fire when consequence doesn't always return"
        );
    }

    #[test]
    fn l0041_silent_when_no_else_branch() {
        let src = "fn f(int x) -> int {\n  if x > 0 { return 1; }\n  return 0;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0041".to_string()),
            "L0041 must not fire when there is no else branch"
        );
    }

    // ---------- L0042: dead code after `return` in same block ----------

    #[test]
    fn l0042_fires_on_statements_after_return() {
        let src = "fn f(int x) -> int {\n  return 1;\n  let y = 2;\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0042".to_string()),
            "L0042 must fire when statements follow a return; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0042_silent_when_return_is_last() {
        let src = "fn f(int x) -> int { return x + 1; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0042".to_string()),
            "L0042 must not fire when return is the last statement"
        );
    }

    #[test]
    fn l0042_silent_with_no_return() {
        let src = "fn f(int x) -> int { let y = x + 1; y }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0042".to_string()),
            "L0042 must not fire when no return statement is present"
        );
    }

    // ---------- L0043: `let` binding shadows parameter or earlier `let` ----------

    #[test]
    fn l0043_fires_when_let_shadows_parameter() {
        let src = "fn f(int x) -> int {\n  let x = 42;\n  return x;\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0043".to_string()),
            "L0043 must fire when let shadows a parameter; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0043_fires_when_let_shadows_earlier_let() {
        let src = "fn f(int n) -> int {\n  let y = 1;\n  let y = 2;\n  return y;\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0043".to_string()),
            "L0043 must fire when let re-declares the same name; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0043_silent_for_distinct_names() {
        let src =
            "fn f(int x) -> int {\n  let y = x + 1;\n  let z = y + 1;\n  return z;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0043".to_string()),
            "L0043 must not fire for distinct binding names"
        );
    }

    // ---------- L0044: shift amount out of range ----------

    #[test]
    fn l0044_fires_on_shift_amount_64() {
        let src = "let x = 1 << 64;\n";
        assert!(
            codes(src).contains(&"L0044".to_string()),
            "L0044 must fire for shift amount 64; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0044_fires_on_shift_amount_100() {
        let src = "let x = 1 >> 100;\n";
        assert!(
            codes(src).contains(&"L0044".to_string()),
            "L0044 must fire for shift amount 100; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0044_fires_on_shift_amount_zero_boundary() {
        // 0 is a valid shift (x << 0 == x), so L0044 must NOT fire.
        let src = "let x = 1 << 0;\n";
        assert!(
            !codes(src).contains(&"L0044".to_string()),
            "L0044 must not fire for shift amount 0"
        );
    }

    #[test]
    fn l0044_silent_for_valid_shift_amount() {
        let src = "let x = 1 << 7;\n";
        assert!(
            !codes(src).contains(&"L0044".to_string()),
            "L0044 must not fire for shift amount 7"
        );
    }

    #[test]
    fn l0044_silent_for_shift_amount_63() {
        let src = "let x = 1 << 63;\n";
        assert!(
            !codes(src).contains(&"L0044".to_string()),
            "L0044 must not fire for shift amount 63 (max valid)"
        );
    }

    #[test]
    fn l0044_silent_when_rhs_is_not_literal() {
        // When the shift amount is a variable, L0044 must not fire.
        let src = "fn f(int n, int s) -> int { return n << s; }\nf(1, 2);\n";
        assert!(
            !codes(src).contains(&"L0044".to_string()),
            "L0044 must not fire when shift amount is not a literal"
        );
    }

    // ---------- L0045: constant-false while condition ----------

    #[test]
    fn l0045_fires_on_while_false() {
        let src = "while false {\n    let x = 1;\n}\n";
        assert!(
            codes(src).contains(&"L0045".to_string()),
            "L0045 must fire for `while false`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0045_fires_on_while_not_true() {
        let src = "while !true {\n    let x = 1;\n}\n";
        assert!(
            codes(src).contains(&"L0045".to_string()),
            "L0045 must fire for `while !true`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0045_silent_for_while_true() {
        // `while true` is NOT constant-false; L0025 handles that separately.
        let src = "while true {\n    let x = 1;\n}\n";
        assert!(
            !codes(src).contains(&"L0045".to_string()),
            "L0045 must not fire for `while true`"
        );
    }

    #[test]
    fn l0045_silent_for_while_variable_condition() {
        let src = "fn f(bool b) {\n    while b {\n        let x = 1;\n    }\n}\n";
        assert!(
            !codes(src).contains(&"L0045".to_string()),
            "L0045 must not fire when condition is not a literal"
        );
    }

    // ---------- L0046: empty for loop body ----------

    #[test]
    fn l0046_fires_on_empty_for_body() {
        let src = "let arr = [1, 2, 3];\nfor x in arr {\n}\n";
        assert!(
            codes(src).contains(&"L0046".to_string()),
            "L0046 must fire for empty for body; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0046_silent_when_for_body_has_statements() {
        let src = "let arr = [1, 2, 3];\nfor x in arr {\n    let y = x;\n}\n";
        assert!(
            !codes(src).contains(&"L0046".to_string()),
            "L0046 must not fire when body has statements"
        );
    }

    // ---------- L0047: vacuous or always-failing assert ----------

    #[test]
    fn l0047_fires_on_assert_true() {
        let src = "assert(true);\n";
        assert!(
            codes(src).contains(&"L0047".to_string()),
            "L0047 must fire for assert(true); got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0047_fires_on_assert_false() {
        let src = "assert(false);\n";
        assert!(
            codes(src).contains(&"L0047".to_string()),
            "L0047 must fire for assert(false); got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0047_silent_for_non_literal_assert() {
        let src = "fn f(int x) {\n    assert(x > 0);\n}\n";
        assert!(
            !codes(src).contains(&"L0047".to_string()),
            "L0047 must not fire for non-literal assert condition"
        );
    }

    #[test]
    fn l0047_fires_on_assert_with_constant_folded_true() {
        // `1 == 1` folds to `true` via try_const_bool → L0047 should fire.
        let src = "assert(1 == 1);\n";
        assert!(
            codes(src).contains(&"L0047".to_string()),
            "L0047 must fire for assert with constant-true condition; got {:?}",
            codes(src)
        );
    }

    // ---------- L0048: bitwise XOR with self ----------

    #[test]
    fn l0048_fires_on_xor_with_same_identifier() {
        let src = "fn f(int x) -> int { return x ^ x; }\nf(5);\n";
        assert!(
            codes(src).contains(&"L0048".to_string()),
            "L0048 must fire for `x ^ x`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0048_silent_for_xor_with_different_identifiers() {
        let src = "fn f(int x, int y) -> int { return x ^ y; }\nf(3, 5);\n";
        assert!(
            !codes(src).contains(&"L0048".to_string()),
            "L0048 must not fire for `x ^ y` (different names)"
        );
    }

    #[test]
    fn l0048_silent_for_xor_with_literal() {
        let src = "fn f(int x) -> int { return x ^ 0; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0048".to_string()),
            "L0048 must not fire for `x ^ 0` (literal rhs)"
        );
    }

    #[test]
    fn l0048_silent_for_non_xor_bitwise_self() {
        // x & x and x | x are also redundant but covered by L0021 (bool) or
        // not at all (int) — L0048 is specifically for XOR.
        let src = "fn f(int x) -> int { return x & x; }\nf(5);\n";
        assert!(
            !codes(src).contains(&"L0048".to_string()),
            "L0048 must not fire for `x & x`; that's a different rule"
        );
    }

    // ---------- L0049: empty if then-branch ----------

    #[test]
    fn l0049_fires_on_empty_if_body() {
        let src = "fn f(int x) {\n    if x > 0 { }\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0049".to_string()),
            "L0049 must fire for empty if body; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0049_fires_on_empty_if_with_else() {
        let src = "fn f(int x) {\n    if x < 0 { } else { let y = x + 1; }\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0049".to_string()),
            "L0049 must fire for empty if body with else; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0049_silent_when_if_body_has_statements() {
        let src = "fn f(int x) {\n    if x > 0 { let y = x; }\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0049".to_string()),
            "L0049 must not fire when if body has statements"
        );
    }

    // ---------- L0050: redundant else after break/continue ----------

    #[test]
    fn l0050_fires_on_else_after_break() {
        let src = "let arr = [1, 2, 3];\nfor x in arr {\n    if x < 0 { break; } else { let y = x; }\n}\n";
        assert!(
            codes(src).contains(&"L0050".to_string()),
            "L0050 must fire for else after break; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0050_fires_on_else_after_continue() {
        let src = "let arr = [1, 2, 3];\nfor x in arr {\n    if x < 0 { continue; } else { let y = x; }\n}\n";
        assert!(
            codes(src).contains(&"L0050".to_string()),
            "L0050 must fire for else after continue; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0050_silent_when_if_has_no_else() {
        let src = "let arr = [1, 2, 3];\nfor x in arr {\n    if x < 0 { break; }\n}\n";
        assert!(
            !codes(src).contains(&"L0050".to_string()),
            "L0050 must not fire when there is no else branch"
        );
    }

    #[test]
    fn l0050_silent_when_consequence_does_not_break() {
        // If the if-body does not end with break/continue, L0050 must not fire.
        let src = "let arr = [1, 2, 3];\nfor x in arr {\n    if x > 0 { let y = x + 1; } else { let z = 0; }\n}\n";
        assert!(
            !codes(src).contains(&"L0050".to_string()),
            "L0050 must not fire when consequence has no break/continue"
        );
    }

    // ---------- L0051: comparison of two string literals ----------

    #[test]
    fn l0051_fires_on_two_string_literal_eq() {
        let src = r#"let x = "a" == "b";"#;
        assert!(
            codes(src).contains(&"L0051".to_string()),
            "L0051 must fire on string-literal == string-literal"
        );
    }

    #[test]
    fn l0051_fires_on_two_string_literal_ne() {
        let src = r#"let x = "hello" != "world";"#;
        assert!(
            codes(src).contains(&"L0051".to_string()),
            "L0051 must fire on string-literal != string-literal"
        );
    }

    #[test]
    fn l0051_silent_on_var_vs_literal() {
        let src = r#"let s = "hello"; let ok = s == "hello";"#;
        assert!(
            !codes(src).contains(&"L0051".to_string()),
            "L0051 must not fire when one operand is a variable"
        );
    }

    // ---------- L0052: negation of boolean literal ----------

    #[test]
    fn l0052_fires_on_not_true() {
        let src = "let x = !true;";
        assert!(
            codes(src).contains(&"L0052".to_string()),
            "L0052 must fire on `!true`"
        );
    }

    #[test]
    fn l0052_fires_on_not_false() {
        let src = "let x = !false;";
        assert!(
            codes(src).contains(&"L0052".to_string()),
            "L0052 must fire on `!false`"
        );
    }

    #[test]
    fn l0052_silent_on_not_variable() {
        let src = "let b = true; let x = !b;";
        assert!(
            !codes(src).contains(&"L0052".to_string()),
            "L0052 must not fire when operand is a variable"
        );
    }

    // ---------- L0053: array index out of bounds ----------

    #[test]
    fn l0053_fires_on_index_past_end() {
        let src = "let x = [1, 2, 3][5];";
        assert!(
            codes(src).contains(&"L0053".to_string()),
            "L0053 must fire when index >= array length"
        );
    }

    #[test]
    fn l0053_fires_on_negative_index() {
        let src = "let x = [1, 2, 3][-1];";
        assert!(
            codes(src).contains(&"L0053".to_string()),
            "L0053 must fire on negative index"
        );
    }

    #[test]
    fn l0053_silent_on_valid_index() {
        let src = "let x = [1, 2, 3][2];";
        assert!(
            !codes(src).contains(&"L0053".to_string()),
            "L0053 must not fire when index is within bounds"
        );
    }

    #[test]
    fn l0053_silent_on_zero_index() {
        let src = "let x = [10, 20, 30][0];";
        assert!(
            !codes(src).contains(&"L0053".to_string()),
            "L0053 must not fire on valid index 0"
        );
    }

    // ---- L0071: too many parameters ----

    #[test]
    fn l0071_fires_on_six_parameters() {
        let src = r#"fn f(int a, int b, int c, int d, int e, int g) { return a; }"#;
        assert!(
            codes(src).contains(&"L0071".to_string()),
            "L0071 must fire for 6 parameters"
        );
    }

    #[test]
    fn l0071_silent_on_five_parameters() {
        let src = r#"fn f(int a, int b, int c, int d, int e) { return a; }"#;
        assert!(
            !codes(src).contains(&"L0071".to_string()),
            "L0071 must not fire for exactly 5 parameters"
        );
    }

    // ---- L0072: unused for-loop variable ----

    #[test]
    fn l0072_fires_on_unused_for_var() {
        let src = r#"fn f(IntArr xs) { for x in xs { return 1; } }"#;
        assert!(
            codes(src).contains(&"L0072".to_string()),
            "L0072 must fire when for-loop variable `x` is never used"
        );
    }

    #[test]
    fn l0072_silent_when_for_var_is_used() {
        let src = r#"fn f(IntArr xs) { for x in xs { return x; } }"#;
        assert!(
            !codes(src).contains(&"L0072".to_string()),
            "L0072 must not fire when for-loop variable is used"
        );
    }

    #[test]
    fn l0072_silent_on_nested_use() {
        // The loop var `item` is used inside a nested if — still counts as used.
        let src = r#"fn f(IntArr xs) { for item in xs { if item > 0 { return item; } } }"#;
        assert!(
            !codes(src).contains(&"L0072".to_string()),
            "L0072 must not fire when for-loop variable is used in nested expr"
        );
    }

    // ── L0073 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0073_fires_on_duplicate_requires() {
        let src = "fn f(int x) requires x > 0 requires x > 0 { return x; }";
        assert!(
            codes(src).contains(&"L0073".to_string()),
            "L0073 must fire for duplicate requires clause"
        );
    }

    #[test]
    fn l0073_no_fire_on_distinct_requires() {
        let src = "fn f(int x) requires x > 0 requires x < 100 { return x; }";
        assert!(
            !codes(src).contains(&"L0073".to_string()),
            "L0073 must not fire when requires clauses are distinct"
        );
    }

    #[test]
    fn l0073_fires_on_duplicate_ensures() {
        let src = "fn f(int x) -> int ensures result > 0 ensures result > 0 { return x + 1; }";
        assert!(
            codes(src).contains(&"L0073".to_string()),
            "L0073 must fire for duplicate ensures clause"
        );
    }

    // ── L0074 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0074_fires_on_pure_call_result_discarded() {
        let src = "@pure fn sq(int x) -> int { return x * x; } fn caller(int x) { sq(x); }";
        assert!(
            codes(src).contains(&"L0074".to_string()),
            "L0074 must fire when pure fn result is discarded"
        );
    }

    #[test]
    fn l0074_no_fire_when_result_used() {
        let src = "@pure fn sq(int x) -> int { return x * x; } fn caller(int x) { let r = sq(x); }";
        assert!(
            !codes(src).contains(&"L0074".to_string()),
            "L0074 must not fire when result is assigned"
        );
    }

    #[test]
    fn l0074_no_fire_on_non_pure_call_discarded() {
        let src = "fn sq(int x) -> int { return x * x; } fn caller(int x) { sq(x); }";
        assert!(
            !codes(src).contains(&"L0074".to_string()),
            "L0074 must not fire for non-pure functions"
        );
    }

    // ── L0075 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0075_fires_on_requires_true() {
        let src = "fn f(int x) requires true { return x; }";
        assert!(
            codes(src).contains(&"L0075".to_string()),
            "L0075 must fire for requires true"
        );
    }

    #[test]
    fn l0075_fires_on_requires_false() {
        let src = "fn f(int x) requires false { return x; }";
        assert!(
            codes(src).contains(&"L0075".to_string()),
            "L0075 must fire for requires false"
        );
    }

    #[test]
    fn l0075_fires_on_ensures_false() {
        let src = "fn f(int x) -> int ensures false { return x; }";
        assert!(
            codes(src).contains(&"L0075".to_string()),
            "L0075 must fire for ensures false"
        );
    }

    #[test]
    fn l0075_no_fire_on_normal_contract() {
        let src = "fn f(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        assert!(
            !codes(src).contains(&"L0075".to_string()),
            "L0075 must not fire for real contracts"
        );
    }

    // ---- L0054: empty while loop body ----

    #[test]
    fn l0054_fires_on_empty_while_body() {
        let src = r#"fn f(bool ready) { while ready {} }"#;
        assert!(
            codes(src).contains(&"L0054".to_string()),
            "L0054 must fire for empty while body"
        );
    }

    #[test]
    fn l0054_silent_when_while_has_body() {
        let src = r#"fn f(int x) { while x > 0 { x = x - 1; } }"#;
        assert!(
            !codes(src).contains(&"L0054".to_string()),
            "L0054 must not fire when while body is non-empty"
        );
    }

    // ---- L0055: redundant boolean `!=` check (complements L0023 which handles `==`) ----

    #[test]
    fn l0055_fires_on_neq_true() {
        let src = "fn f(bool b) -> bool { return b != true; }";
        assert!(
            codes(src).contains(&"L0055".to_string()),
            "L0055 must fire for `x != true`"
        );
    }

    #[test]
    fn l0055_fires_on_neq_false() {
        let src = "fn f(bool b) -> bool { return b != false; }";
        assert!(
            codes(src).contains(&"L0055".to_string()),
            "L0055 must fire for `x != false`"
        );
    }

    #[test]
    fn l0055_silent_on_eq_true() {
        // `==` cases are handled by L0023, not L0055.
        let src = "fn f(bool b) -> bool { return b == true; }";
        assert!(
            !codes(src).contains(&"L0055".to_string()),
            "L0055 must not fire for `x == true` (that is L0023)"
        );
    }

    #[test]
    fn l0055_silent_on_non_bool_neq() {
        let src = "fn f(int x) -> bool { return x != 42; }";
        assert!(
            !codes(src).contains(&"L0055".to_string()),
            "L0055 must not fire for integer `!=`"
        );
    }

    // ---- L0056: for over empty array literal ----

    #[test]
    fn l0056_fires_on_for_over_empty_array() {
        let src = r#"fn f(int x) { for item in [] { return item; } }"#;
        assert!(
            codes(src).contains(&"L0056".to_string()),
            "L0056 must fire when iterating over empty array literal"
        );
    }

    #[test]
    fn l0056_silent_on_for_over_non_empty_array() {
        let src = r#"fn f(int x) { for item in [1, 2, 3] { return item; } }"#;
        assert!(
            !codes(src).contains(&"L0056".to_string()),
            "L0056 must not fire when array has elements"
        );
    }

    // ---- L0057: redundant addition of zero ----

    #[test]
    fn l0057_fires_on_add_zero_rhs() {
        let src = "fn f(int x) -> int { let y = x + 0; return y; }";
        assert!(
            codes(src).contains(&"L0057".to_string()),
            "L0057 must fire for `x + 0`"
        );
    }

    #[test]
    fn l0057_fires_on_add_zero_lhs() {
        let src = "fn f(int x) -> int { let y = 0 + x; return y; }";
        assert!(
            codes(src).contains(&"L0057".to_string()),
            "L0057 must fire for `0 + x`"
        );
    }

    #[test]
    fn l0057_silent_on_nonzero_add() {
        let src = "fn f(int x) -> int { let y = x + 1; return y; }";
        assert!(
            !codes(src).contains(&"L0057".to_string()),
            "L0057 must not fire for `x + 1`"
        );
    }

    // ---- L0058: redundant subtraction of zero ----

    #[test]
    fn l0058_fires_on_sub_zero() {
        let src = "fn f(int x) -> int { let y = x - 0; return y; }";
        assert!(
            codes(src).contains(&"L0058".to_string()),
            "L0058 must fire for `x - 0`"
        );
    }

    #[test]
    fn l0058_silent_on_nonzero_sub() {
        let src = "fn f(int x) -> int { let y = x - 1; return y; }";
        assert!(
            !codes(src).contains(&"L0058".to_string()),
            "L0058 must not fire for `x - 1`"
        );
    }

    // ---- L0059: redundant multiplication by one ----

    #[test]
    fn l0059_fires_on_mul_one_rhs() {
        let src = "fn f(int x) -> int { let y = x * 1; return y; }";
        assert!(
            codes(src).contains(&"L0059".to_string()),
            "L0059 must fire for `x * 1`"
        );
    }

    #[test]
    fn l0059_fires_on_mul_one_lhs() {
        let src = "fn f(int x) -> int { let y = 1 * x; return y; }";
        assert!(
            codes(src).contains(&"L0059".to_string()),
            "L0059 must fire for `1 * x`"
        );
    }

    #[test]
    fn l0059_silent_on_mul_two() {
        let src = "fn f(int x) -> int { let y = x * 2; return y; }";
        assert!(
            !codes(src).contains(&"L0059".to_string()),
            "L0059 must not fire for `x * 2`"
        );
    }

    // ---- L0060: redundant division by one ----

    #[test]
    fn l0060_fires_on_div_one() {
        let src = "fn f(int x) -> int { let y = x / 1; return y; }";
        assert!(
            codes(src).contains(&"L0060".to_string()),
            "L0060 must fire for `x / 1`"
        );
    }

    #[test]
    fn l0060_silent_on_div_two() {
        let src = "fn f(int x) -> int { let y = x / 2; return y; }";
        assert!(
            !codes(src).contains(&"L0060".to_string()),
            "L0060 must not fire for `x / 2`"
        );
    }

    // ---- L0061: shift by zero is a no-op ----

    #[test]
    fn l0061_fires_on_shl_zero() {
        let src = "fn f(int x) -> int { let y = x << 0; return y; }";
        assert!(
            codes(src).contains(&"L0061".to_string()),
            "L0061 must fire for `x << 0`"
        );
    }

    #[test]
    fn l0061_fires_on_shr_zero() {
        let src = "fn f(int x) -> int { let y = x >> 0; return y; }";
        assert!(
            codes(src).contains(&"L0061".to_string()),
            "L0061 must fire for `x >> 0`"
        );
    }

    #[test]
    fn l0061_silent_on_nonzero_shift() {
        let src = "fn f(int x) -> int { let y = x << 1; return y; }";
        assert!(
            !codes(src).contains(&"L0061".to_string()),
            "L0061 must not fire for `x << 1`"
        );
    }

    // ---- L0062: tautological inequality comparison with self ----

    #[test]
    fn l0062_fires_on_lt_self() {
        let src = "fn f(int x) -> bool { return x < x; }";
        assert!(
            codes(src).contains(&"L0062".to_string()),
            "L0062 must fire for `x < x`"
        );
    }

    #[test]
    fn l0062_fires_on_gt_self() {
        let src = "fn f(int x) -> bool { return x > x; }";
        assert!(
            codes(src).contains(&"L0062".to_string()),
            "L0062 must fire for `x > x`"
        );
    }

    #[test]
    fn l0062_fires_on_lte_self() {
        let src = "fn f(int x) -> bool { return x <= x; }";
        assert!(
            codes(src).contains(&"L0062".to_string()),
            "L0062 must fire for `x <= x`"
        );
    }

    #[test]
    fn l0062_fires_on_gte_self() {
        let src = "fn f(int x) -> bool { return x >= x; }";
        assert!(
            codes(src).contains(&"L0062".to_string()),
            "L0062 must fire for `x >= x`"
        );
    }

    #[test]
    fn l0062_silent_on_distinct_operands() {
        let src = "fn f(int x, int y) -> bool { return x < y; }";
        assert!(
            !codes(src).contains(&"L0062".to_string()),
            "L0062 must not fire for `x < y` (distinct operands)"
        );
    }

    // ---- L0063: dead code after break/continue ----

    #[test]
    fn l0063_fires_on_code_after_break() {
        let src = r#"fn f(IntArr xs) { for x in xs { break; let y = 1; } }"#;
        assert!(
            codes(src).contains(&"L0063".to_string()),
            "L0063 must fire for dead code after `break`"
        );
    }

    #[test]
    fn l0063_silent_when_no_break() {
        let src = r#"fn f(int x) -> int { return x; }"#;
        assert!(
            !codes(src).contains(&"L0063".to_string()),
            "L0063 must not fire when there is no break/continue"
        );
    }

    // ---- L0064: empty else block ----

    #[test]
    fn l0064_fires_on_empty_else() {
        let src = r#"fn f(int x) -> int { if x > 0 { return x; } else {} return 0; }"#;
        assert!(
            codes(src).contains(&"L0064".to_string()),
            "L0064 must fire for empty else block"
        );
    }

    #[test]
    fn l0064_silent_when_else_has_content() {
        let src = r#"fn f(int x) -> int { if x > 0 { return x; } else { return 0; } }"#;
        assert!(
            !codes(src).contains(&"L0064".to_string()),
            "L0064 must not fire when else block has content"
        );
    }

    // ---- L0065: if cond { return true; } else { return false; } ----

    #[test]
    fn l0065_fires_on_return_true_else_false() {
        let src =
            r#"fn is_pos(int x) -> bool { if x > 0 { return true; } else { return false; } }"#;
        assert!(
            codes(src).contains(&"L0065".to_string()),
            "L0065 must fire for `if cond {{ return true; }} else {{ return false; }}`"
        );
    }

    #[test]
    fn l0065_silent_when_not_bool_identity() {
        let src = r#"fn f(int x) -> bool { if x > 0 { return true; } else { return true; } }"#;
        assert!(
            !codes(src).contains(&"L0065".to_string()),
            "L0065 must not fire when both branches return true"
        );
    }

    // ---- L0066: if cond { return false; } else { return true; } ----

    #[test]
    fn l0066_fires_on_return_false_else_true() {
        let src =
            r#"fn is_neg(int x) -> bool { if x < 0 { return false; } else { return true; } }"#;
        assert!(
            codes(src).contains(&"L0066".to_string()),
            "L0066 must fire for `if cond {{ return false; }} else {{ return true; }}`"
        );
    }

    #[test]
    fn l0066_silent_when_not_bool_negation() {
        let src = r#"fn f(int x) -> bool { if x > 0 { return false; } else { return false; } }"#;
        assert!(
            !codes(src).contains(&"L0066".to_string()),
            "L0066 must not fire when both branches return false"
        );
    }

    // ---- L0067: x && true ----

    #[test]
    fn l0067_fires_on_and_true_rhs() {
        let src = r#"fn f(bool x) -> bool { return x && true; }"#;
        assert!(
            codes(src).contains(&"L0067".to_string()),
            "L0067 must fire for `x && true`"
        );
    }

    #[test]
    fn l0067_fires_on_true_and_lhs() {
        let src = r#"fn f(bool x) -> bool { return true && x; }"#;
        assert!(
            codes(src).contains(&"L0067".to_string()),
            "L0067 must fire for `true && x`"
        );
    }

    #[test]
    fn l0067_silent_on_and_false() {
        let src = r#"fn f(bool x) -> bool { return x && false; }"#;
        assert!(
            !codes(src).contains(&"L0067".to_string()),
            "L0067 must not fire for `x && false`"
        );
    }

    // ---- L0068: x && false ----

    #[test]
    fn l0068_fires_on_and_false_rhs() {
        let src = r#"fn f(bool x) -> bool { return x && false; }"#;
        assert!(
            codes(src).contains(&"L0068".to_string()),
            "L0068 must fire for `x && false`"
        );
    }

    #[test]
    fn l0068_fires_on_false_and_lhs() {
        let src = r#"fn f(bool x) -> bool { return false && x; }"#;
        assert!(
            codes(src).contains(&"L0068".to_string()),
            "L0068 must fire for `false && x`"
        );
    }

    #[test]
    fn l0068_silent_on_and_true() {
        let src = r#"fn f(bool x) -> bool { return x && true; }"#;
        assert!(
            !codes(src).contains(&"L0068".to_string()),
            "L0068 must not fire for `x && true`"
        );
    }

    // ---- L0069: x || true ----

    #[test]
    fn l0069_fires_on_or_true_rhs() {
        let src = r#"fn f(bool x) -> bool { return x || true; }"#;
        assert!(
            codes(src).contains(&"L0069".to_string()),
            "L0069 must fire for `x || true`"
        );
    }

    #[test]
    fn l0069_fires_on_true_or_lhs() {
        let src = r#"fn f(bool x) -> bool { return true || x; }"#;
        assert!(
            codes(src).contains(&"L0069".to_string()),
            "L0069 must fire for `true || x`"
        );
    }

    #[test]
    fn l0069_silent_on_or_false() {
        let src = r#"fn f(bool x) -> bool { return x || false; }"#;
        assert!(
            !codes(src).contains(&"L0069".to_string()),
            "L0069 must not fire for `x || false`"
        );
    }

    // ---- L0070: x || false ----

    #[test]
    fn l0070_fires_on_or_false_rhs() {
        let src = r#"fn f(bool x) -> bool { return x || false; }"#;
        assert!(
            codes(src).contains(&"L0070".to_string()),
            "L0070 must fire for `x || false`"
        );
    }

    #[test]
    fn l0070_fires_on_false_or_lhs() {
        let src = r#"fn f(bool x) -> bool { return false || x; }"#;
        assert!(
            codes(src).contains(&"L0070".to_string()),
            "L0070 must fire for `false || x`"
        );
    }

    #[test]
    fn l0070_silent_on_or_true() {
        let src = r#"fn f(bool x) -> bool { return x || true; }"#;
        assert!(
            !codes(src).contains(&"L0070".to_string()),
            "L0070 must not fire for `x || true`"
        );
    }

    // ── L0076 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0076_fires_on_result_in_requires() {
        let src = "fn f(int x) -> int requires result > 0 { return x; }\n";
        assert!(
            codes(src).contains(&"L0076".to_string()),
            "L0076 must fire when `result` appears in a requires clause; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0076_silent_for_result_in_ensures() {
        let src = "fn f(int x) -> int ensures result > 0 { return x + 1; }\n";
        assert!(
            !codes(src).contains(&"L0076".to_string()),
            "L0076 must not fire when `result` appears in an ensures clause"
        );
    }

    // ── L0077 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0077_fires_on_ensures_result_in_void_fn() {
        let src = "fn f(int x) ensures result > 0 { println(x); }\n";
        assert!(
            codes(src).contains(&"L0077".to_string()),
            "L0077 must fire for ensures result in void function; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0077_silent_for_ensures_result_in_returning_fn() {
        let src = "fn f(int x) -> int ensures result > 0 { return x + 1; }\n";
        assert!(
            !codes(src).contains(&"L0077".to_string()),
            "L0077 must not fire when function has a return type"
        );
    }

    // ── L0078 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0078_fires_on_builtin_shadow() {
        let src = "fn f(int len) { return len; }\n";
        assert!(
            codes(src).contains(&"L0078".to_string()),
            "L0078 must fire when parameter shadows builtin `len`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0078_silent_for_normal_param_name() {
        let src = "fn f(int size) { return size; }\n";
        assert!(
            !codes(src).contains(&"L0078".to_string()),
            "L0078 must not fire for normal parameter names"
        );
    }

    // ── L0079 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0079_fires_on_empty_body() {
        let src = "fn f(int x) { }\n";
        assert!(
            codes(src).contains(&"L0079".to_string()),
            "L0079 must fire for function with empty body; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0079_silent_for_non_empty_body() {
        let src = "fn f(int x) { return x; }\n";
        assert!(
            !codes(src).contains(&"L0079".to_string()),
            "L0079 must not fire when function has statements"
        );
    }

    // ── L0080 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0080_fires_on_dead_let_init() {
        let src = "fn f(int x) { let y = x + 1; y = x * 2; return y; }\n";
        assert!(
            codes(src).contains(&"L0080".to_string()),
            "L0080 must fire when let binding is immediately overwritten; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0080_silent_when_let_is_used_first() {
        let src = "fn f(int x) { let y = x + 1; let _z = y * 2; y = 0; return y; }\n";
        assert!(
            !codes(src).contains(&"L0080".to_string()),
            "L0080 must not fire when the let binding is used before overwrite"
        );
    }

    // ── L0081 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0081_fires_on_duplicate_assert() {
        let src = "fn f(int x) requires x > 0 { assert(x > 0); assert(x > 0); return x; }\n";
        assert!(
            codes(src).contains(&"L0081".to_string()),
            "L0081 must fire for duplicate consecutive assert; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0081_silent_for_different_asserts() {
        let src = "fn f(int x) requires x > 0 { assert(x > 0); assert(x < 100); return x; }\n";
        assert!(
            !codes(src).contains(&"L0081".to_string()),
            "L0081 must not fire for different assert conditions"
        );
    }

    // ── L0082 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0082_fires_on_both_empty_branches() {
        let src = "fn f(bool cond) { if cond { } else { } }\n";
        assert!(
            codes(src).contains(&"L0082".to_string()),
            "L0082 must fire when both if/else branches are empty; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0082_silent_when_then_has_stmt() {
        let src = "fn f(bool cond) { if cond { let _x = 1; } else { } }\n";
        assert!(
            !codes(src).contains(&"L0082".to_string()),
            "L0082 must not fire when then-branch has statements"
        );
    }

    // ── L0083 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0083_fires_on_noreturn_with_return_type() {
        let src = "// @noreturn\nfn die(int code) -> int { abort(); }\n";
        assert!(
            codes(src).contains(&"L0083".to_string()),
            "L0083 must fire when @noreturn function has a return type; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0083_silent_for_noreturn_void_fn() {
        let src = "// @noreturn\nfn die(int code) { abort(); }\n";
        assert!(
            !codes(src).contains(&"L0083".to_string()),
            "L0083 must not fire for @noreturn void function"
        );
    }

    // ── L0084 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0084_fires_on_nested_function() {
        let src = "fn outer(int x) { fn inner(int y) { return y; } return x; }\n";
        assert!(
            codes(src).contains(&"L0084".to_string()),
            "L0084 must fire for nested function definition; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0084_silent_for_top_level_functions() {
        let src = "fn f(int x) { return x; }\nfn g(int y) { return y; }\n";
        assert!(
            !codes(src).contains(&"L0084".to_string()),
            "L0084 must not fire for separate top-level functions"
        );
    }

    // ── L0085 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0085_fires_on_empty_struct() {
        let src = "struct Empty { }\nfn f(int x) { return x; }\n";
        assert!(
            codes(src).contains(&"L0085".to_string()),
            "L0085 must fire for struct with zero fields; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0085_silent_for_non_empty_struct() {
        let src = "struct Point { int x, int y }\nfn f(int x) { return x; }\n";
        assert!(
            !codes(src).contains(&"L0085".to_string()),
            "L0085 must not fire for struct with fields"
        );
    }

    // ── L0086 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0086_fires_on_eq_empty_string() {
        let src = "fn f(int x) { if x == \"\" { return 1; } return 0; }\n";
        assert!(
            codes(src).contains(&"L0086".to_string()),
            "L0086 must fire when comparing with `==` to empty string literal; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0086_silent_for_non_empty_string_comparison() {
        let src = "fn f(int x) { if x == \"hello\" { return 1; } return 0; }\n";
        assert!(
            !codes(src).contains(&"L0086".to_string()),
            "L0086 must not fire when comparing to a non-empty string literal"
        );
    }

    // ── L0087 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0087_fires_on_pure_fn_with_println() {
        let src = "@pure fn square(int x) -> int { println(x); return x * x; }\n";
        assert!(
            codes(src).contains(&"L0087".to_string()),
            "L0087 must fire when a pure function calls println; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0087_silent_for_pure_fn_without_print() {
        let src = "@pure fn square(int x) -> int { return x * x; }\n";
        assert!(
            !codes(src).contains(&"L0087".to_string()),
            "L0087 must not fire when pure function has no print/println call"
        );
    }

    // ── L0088 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0088_explain_is_registered() {
        // The parser currently rejects `let _` syntax, so the lint fires only
        // once parser support for wildcard bindings is added. Until then, verify
        // the code is registered and has an explanation.
        assert!(
            KNOWN_CODES.contains(&"L0088"),
            "L0088 must be in KNOWN_CODES"
        );
        assert!(explain("L0088").is_some(), "L0088 must have an explanation");
    }

    #[test]
    fn l0088_silent_for_named_let() {
        let src = "fn f(int x) -> int { let y = x + 1; return y; }\n";
        assert!(
            !codes(src).contains(&"L0088".to_string()),
            "L0088 must not fire for a normal named let binding"
        );
    }

    // ── L0089 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0089_fires_on_exit_in_live_block() {
        let src = "fn f(int x) -> int { live { exit(1); } return x; }\n";
        assert!(
            codes(src).contains(&"L0089".to_string()),
            "L0089 must fire when exit() is called inside a live block; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0089_silent_for_live_block_without_exit() {
        let src = "fn f(int x) -> int { live { return x; } return x; }\n";
        assert!(
            !codes(src).contains(&"L0089".to_string()),
            "L0089 must not fire when live block has no exit/abort"
        );
    }

    // ── L0090 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0090_fires_when_both_arms_return_same_literal() {
        let src = "fn f(int x) -> int { if x > 0 { return 1; } else { return 1; } }\n";
        assert!(
            codes(src).contains(&"L0090".to_string()),
            "L0090 must fire when both if/else arms return the same literal; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0090_silent_when_arms_return_different_values() {
        let src = "fn f(int x) -> int { if x > 0 { return 1; } else { return 0; } }\n";
        assert!(
            !codes(src).contains(&"L0090".to_string()),
            "L0090 must not fire when if/else arms return different values"
        );
    }

    // ── L0091 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0091_fires_on_equal_range_bounds() {
        let src = "fn f() { for i in 5..5 { return 1; } }\n";
        assert!(
            codes(src).contains(&"L0091".to_string()),
            "L0091 must fire for for-range with equal bounds; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0091_silent_for_non_equal_range_bounds() {
        let src = "fn f() { for i in 0..5 { return 1; } }\n";
        assert!(
            !codes(src).contains(&"L0091".to_string()),
            "L0091 must not fire when range bounds differ"
        );
    }

    // ── L0092 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0092_fires_on_ensures_false() {
        let src = "fn f(int x) -> int ensures false { return x; }\n";
        assert!(
            codes(src).contains(&"L0092".to_string()),
            "L0092 must fire when function has `ensures false`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0092_silent_for_normal_ensures() {
        let src = "fn f(int x) -> int ensures result > 0 { return x + 1; }\n";
        assert!(
            !codes(src).contains(&"L0092".to_string()),
            "L0092 must not fire for a meaningful ensures clause"
        );
    }

    // ── L0093 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0093_fires_on_param_named_result() {
        let src = "fn f(int result) -> int { return result; }\n";
        assert!(
            codes(src).contains(&"L0093".to_string()),
            "L0093 must fire when a parameter is named `result`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0093_silent_for_normal_param_names() {
        let src = "fn f(int value) -> int { return value; }\n";
        assert!(
            !codes(src).contains(&"L0093".to_string()),
            "L0093 must not fire for parameters with normal names"
        );
    }

    // ── L0094 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0094_fires_on_consecutive_break() {
        let src = "fn f() { for i in 0..5 { break; break; } }\n";
        assert!(
            codes(src).contains(&"L0094".to_string()),
            "L0094 must fire for consecutive break statements; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0094_silent_for_single_break() {
        let src = "fn f() { for i in 0..5 { break; } }\n";
        assert!(
            !codes(src).contains(&"L0094".to_string()),
            "L0094 must not fire for a single break statement"
        );
    }

    // ── L0095 tests ──────────────────────────────────────────────────────────

    #[test]
    fn l0095_fires_on_single_wildcard_match() {
        let src = "fn f(int x) -> int { return match x { _ => 1, }; }\n";
        assert!(
            codes(src).contains(&"L0095".to_string()),
            "L0095 must fire for match with single wildcard arm; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0095_silent_for_multi_arm_match() {
        let src = "fn f(int x) -> int { return match x { 0 => 0, _ => 1, }; }\n";
        assert!(
            !codes(src).contains(&"L0095".to_string()),
            "L0095 must not fire for match with multiple arms"
        );
    }
}
