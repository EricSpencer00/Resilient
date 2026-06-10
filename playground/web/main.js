// RES-368: browser-side glue for the Resilient WASM playground.
//
// Wires three things:
//   1. CodeMirror editor in the left pane.
//   2. Examples dropdown — populated from `examples.json` produced
//      at build time (or a fallback list of known examples if the
//      manifest is missing, which keeps the page useful when served
//      directly from the `playground/web/` directory in dev).
//   3. Run button — invokes `compile_and_run(source, "")` from the
//      WASM module and renders the structured result into the
//      output pane.

import init, {
  compile_and_run,
  playground_version,
} from "./pkg/resilient_playground.js";

// ---------------------------------------------------------------------------
// RES-2625: custom CodeMirror mode for Resilient syntax highlighting
// ---------------------------------------------------------------------------
CodeMirror.defineMode("resilient", function () {
  const KEYWORDS = new Set([
    "fn", "let", "struct", "if", "else", "return", "while", "for", "match",
    "in", "true", "false", "new", "impl", "trait", "type", "use", "mod",
    "pub", "static", "live", "linear", "requires", "ensures", "invariant",
    "enum", "forall", "exists", "assert", "spawn", "send", "receive",
    "and", "or", "not", "is_err", "is_ok", "unwrap", "unwrap_err",
  ]);

  return {
    startState() {
      return { inString: false, inComment: false };
    },
    token(stream, state) {
      // Line comments
      if (stream.match("//")) {
        stream.skipToEnd();
        return "comment";
      }
      // String literals
      if (stream.match('"')) {
        state.inString = true;
      }
      if (state.inString) {
        while (!stream.eol()) {
          const ch = stream.next();
          if (ch === "\\") {
            stream.next(); // skip escaped char
          } else if (ch === '"') {
            state.inString = false;
            break;
          }
        }
        return "string";
      }
      // Numbers (int and float)
      if (stream.match(/^[0-9]+(\.[0-9]+)?/)) {
        return "number";
      }
      // Identifiers and keywords
      if (stream.match(/^[A-Za-z_][A-Za-z0-9_]*/)) {
        const word = stream.current();
        if (KEYWORDS.has(word)) return "keyword";
        // Type names start uppercase
        if (/^[A-Z]/.test(word)) return "type";
        return "variable";
      }
      // Operators / punctuation
      if (stream.match(/^[+\-*/%<>=!&|^~?:;.,(){}\[\]]/)) {
        return "operator";
      }
      stream.next();
      return null;
    },
  };
});

// Fallback examples shown when examples.json is not present (e.g., dev mode).
// These are the curated top picks that best illustrate what makes Resilient
// distinct. The full list (350+ examples) is baked into examples.json at
// build time by playground/build.sh.
const EXAMPLES_FALLBACK = [
  {
    name: "sensor_monitor.rz",
    source: `// Flagship example: contracts + Result + live self-healing block.
// Classifies sensor readings into buckets and counts each category.

struct Reading { int id, int value, }
struct Counts { int low, int mid, int high, int alert, }

fn bucket_of(int v) -> string
    requires v >= 0
{
    if v < 25  { return "low"; }
    if v < 75  { return "mid"; }
    if v < 100 { return "high"; }
    return "alert";
}

fn validate(int v) -> Result {
    if v < 0    { return Err("negative reading"); }
    if v > 1000 { return Err("overflow reading"); }
    return Ok(v);
}

fn process(int v) -> Result {
    let safe = validate(v)?;
    return Ok(bucket_of(safe));
}

fn main() {
    let readings = [
        new Reading { id: 1, value: 10  },
        new Reading { id: 2, value: 50  },
        new Reading { id: 3, value: 90  },
        new Reading { id: 4, value: 200 },
        new Reading { id: 5, value: 0   },
    ];
    let counts = new Counts { low: 0, mid: 0, high: 0, alert: 0 };
    let total = 0;
    live invariant total >= 0 {
        for r in readings {
            let result = process(r.value);
            if is_err(result) {
                println("rejecting " + r.id + ": " + unwrap_err(result));
            } else {
                let label = unwrap(result);
                if label == "low"   { counts.low   = counts.low   + 1; }
                if label == "mid"   { counts.mid   = counts.mid   + 1; }
                if label == "high"  { counts.high  = counts.high  + 1; }
                if label == "alert" { counts.alert = counts.alert + 1; }
                total = total + 1;
            }
        }
    }
    println("low:   " + counts.low);
    println("mid:   " + counts.mid);
    println("high:  " + counts.high);
    println("alert: " + counts.alert);
    println("processed: " + total);
}
main();
`,
  },
  {
    name: "showcase_contracts.rz",
    source: `// Contracts: machine-checked pre/postconditions.
// requires guards the caller; ensures guarantees the callee.
// With --features z3, these discharge statically — zero runtime cost.

fn safe_divide(int n, int d) -> int
    requires d != 0
{
    return n / d;
}

fn clamp(int x, int lo, int hi) -> int
    requires lo <= hi
    ensures result >= lo
    ensures result <= hi
{
    if x < lo { return lo; }
    if x > hi { return hi; }
    return x;
}

fn factorial(int n) -> int
    requires n >= 0
    ensures result >= 1
{
    if n <= 1 { return 1; }
    return n * factorial(n - 1);
}

fn main() {
    println(safe_divide(100, 7));   // 14
    println(safe_divide(42, 6));    // 7

    println(clamp(-5, 0, 10));     // 0  (below lo)
    println(clamp(15, 0, 10));     // 10 (above hi)
    println(clamp(5,  0, 10));     // 5  (in range)

    println(factorial(5));          // 120
    println(factorial(0));          // 1
}
main();
`,
  },
  {
    name: "showcase_linear_types.rz",
    source: `// Linear types: the compiler tracks resource ownership.
// A linear value must be consumed exactly once.
// Double-close and use-after-close are compile-time errors.

struct UartConn { int port }

fn uart_open(int port) -> linear UartConn {
    println("uart: opened port " + port);
    return new UartConn { port: port };
}

fn uart_write(linear UartConn c, string msg) -> linear UartConn {
    println("uart: tx " + msg);
    return c;   // ownership returned — caller must consume it
}

fn uart_close(linear UartConn c) {
    println("uart: closed port " + c.port);
    // c consumed here; the compiler forbids any further use
}

fn main() {
    let c  = uart_open(2);
    let c2 = uart_write(c,  "boot ok");
    let c3 = uart_write(c2, "sensor ready");
    uart_close(c3);
    println("handle released exactly once");
}
main();
`,
  },
  {
    name: "showcase_live_invariant.rz",
    source: `// \`live invariant\` is atomic rollback-on-failure with bounded retry.
// When the body's exit state violates the invariant (or the body
// throws), the runtime restores the pre-block environment and
// re-runs the body. On exhaustion the error propagates.
//
// This demo exercises the machinery for real:
//   - the body is non-idempotent (a two-step money transfer),
//   - a transient fault aborts mid-transfer, and
//   - the invariant \`total funds preserved\` fails on partial state.
// Without rollback, repeated debits would compound and corrupt the
// books. live_retries() lets the simulated fault clear by attempt 3.

fn main() {
    let bal_a = 100;
    let bal_b = 50;
    let total_before = bal_a + bal_b;

    println("before: a=" + bal_a + " b=" + bal_b);

    live invariant bal_a + bal_b == total_before {
        bal_a = bal_a - 50;                       // partial mutation
        if live_retries() < 2 {
            assert(false, "transient fault — must roll back");
        }
        bal_b = bal_b + 50;                       // completes transfer
    }

    println("after:  a=" + bal_a + " b=" + bal_b);
    println("total preserved: " + (bal_a + bal_b));
    println("retries used: " + live_total_retries());
}
main();
`,
  },
  {
    name: "showcase_actors.rz",
    source: `// Actor model: isolated processes communicate via messages.
// spawn() creates an actor; send/receive pass values between them.
// No shared state — data races are structurally impossible.

fn summer() {
    let a = receive();
    let b = receive();
    let c = receive();
    println("sum = " + (a + b + c));
}

fn main() {
    let pid = spawn(summer);
    send(pid, 10);
    send(pid, 20);
    send(pid, 12);   // 10 + 20 + 12 = 42
    println("messages sent");
}
main();
`,
  },
  {
    name: "showcase_quantifiers.rz",
    source: `// Quantifiers as runnable code and static proofs.
// forall / exists evaluate at runtime and, with --features z3,
// discharge as SMT lemmas — no loop unrolling needed.

fn is_even(int n) -> bool { return n % 2 == 0; }

fn main() {
    // Every square in 0..8 is non-negative.
    println(forall i in 0..8: i * i >= 0);     // true

    // There exists an even number in 1..10.
    println(exists i in 1..10: is_even(i));    // true

    // Not all numbers in 0..5 are even.
    println(forall i in 0..5: is_even(i));     // false

    // Vacuously true: empty range, no counterexample.
    println(forall i in 5..5: false);          // true

    // Embed quantifiers directly in assertions.
    assert(forall i in 0..10: i * i >= 0);
    assert(exists i in 0..10: i == 7);
    println("all proofs passed");
}
main();
`,
  },
  {
    name: "showcase_result.rz",
    source: `// Result<T>: principled error propagation without exceptions.
// Ok(v) carries a success value; Err(e) carries a failure reason.
// Pattern matching forces every caller to handle both cases.

fn safe_div(int n, int d) -> Result {
    if d == 0 { return Err("division by zero"); }
    return Ok(n / d);
}

fn safe_sqrt(int n) -> Result {
    if n < 0 { return Err("negative input"); }
    return Ok(n);
}

fn main() {
    let r1 = safe_div(100, 5);
    let msg1 = match r1 {
        Ok(v)  => "100 / 5 = " + v,
        Err(e) => "error: " + e,
    };
    println(msg1);

    let r2 = safe_div(42, 0);
    let msg2 = match r2 {
        Ok(v)  => "ok: " + v,
        Err(e) => "caught: " + e,
    };
    println(msg2);

    let r3 = safe_sqrt(-1);
    let msg3 = match r3 {
        Ok(v)  => "ok: " + v,
        Err(e) => "caught: " + e,
    };
    println(msg3);
}
main();
`,
  },
  {
    name: "sum_types_match.rz",
    source: `// Algebraic data types with exhaustive pattern matching.
// Each variant carries its own fields; match enforces coverage.

enum Shape {
    Circle { r: int },
    Square { side: int },
    Rect   { w: int, h: int },
}

enum Color { Red, Green, Blue }

fn area(Shape s) -> int {
    return match s {
        Shape::Circle { r }    => 3 * r * r,
        Shape::Square { side } => side * side,
        Shape::Rect   { w, h } => w * h,
    };
}

fn name(Color c) -> string {
    return match c {
        Color::Red   => "red",
        Color::Green => "green",
        Color::Blue  => "blue",
    };
}

fn main() -> int {
    println(area(new Shape::Circle { r: 2 }));       // 12
    println(area(new Shape::Square { side: 4 }));    // 16
    println(area(new Shape::Rect { w: 3, h: 5 }));  // 15
    println(name(Color::Red));                        // red
    println(name(Color::Green));                      // green
    println(name(Color::Blue));                       // blue
    return 0;
}
main();
`,
  },
  {
    name: "operator_overload.rz",
    source: `// Operator overloading: implement Add, Sub, Mul for a custom type.

struct Vec2 { float x, float y, }

impl Add for Vec2 {
    fn add(Vec2 self, Vec2 other) -> Vec2 {
        return new Vec2 { x: self.x + other.x, y: self.y + other.y };
    }
}

impl Sub for Vec2 {
    fn sub(Vec2 self, Vec2 other) -> Vec2 {
        return new Vec2 { x: self.x - other.x, y: self.y - other.y };
    }
}

impl Mul for Vec2 {
    fn mul(Vec2 self, Vec2 other) -> Vec2 {
        return new Vec2 { x: self.x * other.x, y: self.y * other.y };
    }
}

fn main() -> int {
    let a = new Vec2 { x: 1.0, y: 2.0 };
    let b = new Vec2 { x: 3.0, y: 4.0 };
    let sum  = a + b;  println(sum.x);   println(sum.y);   // 4, 6
    let diff = b - a;  println(diff.x);  println(diff.y);  // 2, 2
    let prod = a * b;  println(prod.x);  println(prod.y);  // 3, 8
    return 0;
}
main();
`,
  },
  {
    name: "pipe_operator.rz",
    source: `// The |> pipe operator: x |> f desugars to f(x).
// Chains read top-to-bottom instead of inside-out.

fn double(int n) -> int  { return n * 2; }
fn add_one(int n) -> int { return n + 1; }

fn main(int _d) {
    // Without pipes: nested calls read inside-out.
    println(add_one(double(add_one(3))));        // 9

    // With pipes: reads left-to-right.
    println(3 |> add_one |> double |> add_one);  // 9

    // String pipeline: trim then uppercase.
    println("  resilient  " |> trim |> to_upper);  // RESILIENT

    // Pipe with stdlib.
    println(-42 |> abs);    // 42
    println("ab" |> repeat(3));  // ababab
    return 0;
}
main(0);
`,
  },
  {
    name: "invariant_proven_demo.rz",
    source: `// SMT-discharged loop invariant.
// The verifier statically proves both base case and inductive step
// without executing the loop — no runtime check needed.

fn main() {
    let i = 0;
    let n = 5;
    while i < n {
        invariant i >= 0;
        invariant i <= n;
        i = i + 1;
    }
    println("ok");
}
main();
`,
  },
  {
    name: "comprehension_demo.rz",
    source: `// Array comprehensions: [expr for x in xs (if guard)?]

fn main(int _d) {
    let xs = [1, 2, 3, 4, 5, 6];

    // Map: double each element.
    let doubled = [x * 2 for x in xs];
    println(doubled);               // [2, 4, 6, 8, 10, 12]

    // Filter-map: square the even numbers only.
    let even_sq = [x * x for x in xs if x % 2 == 0];
    println(even_sq);               // [4, 16, 36]

    return 0;
}
main(0);
`,
  },
  {
    name: "hello.rz",
    source: `fn main() {
    println("Hello, Resilient world!");
}
main();
`,
  },
];

const editorEl = document.getElementById("editor");
const outputEl = document.getElementById("output");
const footerEl = document.getElementById("output-footer");
const runBtn = document.getElementById("run-btn");
const shareBtn = document.getElementById("share-btn");
const clearBtn = document.getElementById("clear-btn");
const select = document.getElementById("example-select");
const banner = document.getElementById("fallback-banner");

// RES-2624: restore code from URL hash before initialising the editor
function getHashCode() {
  const hash = window.location.hash;
  if (hash.startsWith("#code=")) {
    try {
      return atob(hash.slice(6));
    } catch (_) {
      return null;
    }
  }
  return null;
}

const initialSource = getHashCode() ?? EXAMPLES_FALLBACK[0].source;
editorEl.value = initialSource;

const cm = CodeMirror.fromTextArea(editorEl, {
  mode: "resilient",
  lineNumbers: true,
  theme: "idea",
  indentUnit: 4,
  smartIndent: true,
  autofocus: true,
});

let wasmReady = false;

// ---------------------------------------------------------------------------
// RES-2625: error underline helpers using CodeMirror text markers
// ---------------------------------------------------------------------------
let errorMarks = [];

function clearErrorMarks() {
  for (const m of errorMarks) m.clear();
  errorMarks = [];
}

// Parse error positions from the compiler's stderr lines.
// Supports the two common formats the Resilient compiler emits:
//   "error at line 3, col 5: ..."
//   "3:5: error: ..."
function parseErrorPositions(stderr) {
  if (!stderr) return [];
  const positions = [];
  // Format 1: "error at line N, col M"  (optional trailing colon/message)
  const re1 = /(?:error|warning)[^:]*?line\s+(\d+)[,\s]+col(?:umn)?\s+(\d+)/gi;
  // Format 2: "N:M: error/warning"
  const re2 = /^(\d+):(\d+):\s*(?:error|warning)/gim;
  for (const re of [re1, re2]) {
    let m;
    while ((m = re.exec(stderr)) !== null) {
      positions.push({ line: parseInt(m[1], 10) - 1, col: parseInt(m[2], 10) - 1 });
    }
  }
  return positions;
}

function applyErrorMarks(stderr) {
  clearErrorMarks();
  const positions = parseErrorPositions(stderr);
  for (const { line, col } of positions) {
    const lineText = cm.getLine(line);
    if (lineText == null) continue;
    // Underline from col to end-of-token (or end-of-line)
    const end = lineText.slice(col).search(/[\s,;(){}\[\]]/) + col;
    const to = end <= col ? { line, ch: lineText.length } : { line, ch: end };
    const mark = cm.markText(
      { line, ch: col },
      to,
      { className: "rz-error-underline" }
    );
    errorMarks.push(mark);
  }
}

// ---------------------------------------------------------------------------
// RES-2625: debounced auto-compile (fires 500 ms after last keystroke)
// ---------------------------------------------------------------------------
let autoCompileTimer = null;

cm.on("change", () => {
  clearTimeout(autoCompileTimer);
  autoCompileTimer = setTimeout(() => {
    if (wasmReady) runCode(/* auto */ true);
  }, 500);
});

// ---------------------------------------------------------------------------
// RES-2624: share button — encode current code as base64 URL hash
// ---------------------------------------------------------------------------
function showNotification(msg) {
  const el = document.createElement("span");
  el.className = "share-toast";
  el.textContent = msg;
  document.querySelector("nav").appendChild(el);
  setTimeout(() => el.remove(), 2000);
}

async function bootstrap() {
  try {
    await init();
    wasmReady = true;
    const v = playground_version();
    if (v.endsWith("-stub")) {
      banner.hidden = false;
    }
    footerEl.textContent = `playground ${v} ready`;
    runBtn.disabled = false;
  } catch (err) {
    runBtn.disabled = true;
    outputEl.classList.add("error");
    outputEl.textContent =
      "Failed to load WASM module:\n" +
      (err && err.message ? err.message : String(err));
    footerEl.textContent = "WASM init failed";
  }
}

async function loadExamples() {
  let manifest = null;
  try {
    const res = await fetch("./examples.json", { cache: "no-store" });
    if (res.ok) manifest = await res.json();
  } catch (_err) {
    // fall through to fallback
  }
  const list = Array.isArray(manifest) && manifest.length ? manifest : EXAMPLES_FALLBACK;
  for (const ex of list) {
    const opt = document.createElement("option");
    opt.value = ex.name;
    opt.textContent = ex.name;
    opt.dataset.source = ex.source;
    select.appendChild(opt);
  }
}

select.addEventListener("change", () => {
  const opt = select.selectedOptions[0];
  if (opt && opt.dataset.source) {
    cm.setValue(opt.dataset.source);
    cm.focus();
  }
});

clearBtn.addEventListener("click", () => {
  outputEl.textContent = "";
  outputEl.classList.remove("error");
  footerEl.textContent = "";
  clearErrorMarks();
});

// RES-2624: share button
shareBtn.addEventListener("click", () => {
  const encoded = btoa(unescape(encodeURIComponent(cm.getValue())));
  window.location.hash = "#code=" + encoded;
  try {
    navigator.clipboard.writeText(window.location.href);
    showNotification("Link copied!");
  } catch (_) {
    showNotification("URL updated!");
  }
});

// ---------------------------------------------------------------------------
// Core run logic — used by both the Run button and debounced auto-compile
// ---------------------------------------------------------------------------
async function runCode(isAuto = false) {
  if (!wasmReady) return;
  if (!isAuto) runBtn.disabled = true;
  outputEl.classList.remove("error");
  if (!isAuto) outputEl.textContent = "Running…";
  if (!isAuto) footerEl.textContent = "";

  // Yield to the event loop so the "Running…" placeholder paints
  // before the (potentially blocking) WASM call. Stubs are fast,
  // but the full interpreter will benefit from this once integrated.
  await new Promise((r) => setTimeout(r, 0));

  let result;
  try {
    result = compile_and_run(cm.getValue(), "");
  } catch (err) {
    outputEl.classList.add("error");
    outputEl.textContent =
      "WASM trapped while running:\n" +
      (err && err.message ? err.message : String(err));
    footerEl.textContent = "trap";
    runBtn.disabled = false;
    return;
  }

  const stdout = result?.stdout ?? "";
  const stderr = result?.stderr ?? null;
  const code = result?.exit_code ?? -1;
  const ms = Math.max(0, Math.round(result?.duration_ms ?? 0));
  const flavor = result?.flavor ?? "unknown";

  clearErrorMarks();
  if (stderr) {
    outputEl.classList.add("error");
    outputEl.textContent = stderr + (stdout ? "\n--\n" + stdout : "");
    applyErrorMarks(stderr);
  } else {
    outputEl.classList.remove("error");
    outputEl.textContent = stdout || "(no output)";
  }
  footerEl.textContent = `exit ${code} • ${ms} ms • ${flavor}${isAuto ? " • auto" : ""}`;
  runBtn.disabled = false;
}

runBtn.addEventListener("click", () => runCode(false));

runBtn.disabled = true;
loadExamples();
bootstrap();
