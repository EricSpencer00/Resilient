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

const EXAMPLES_FALLBACK = [
  {
    name: "hello.rz",
    source:
      'fn main() {\n    println("Hello, Resilient world!");\n}\nmain();\n',
  },
  {
    name: "factorial.rz",
    source:
      "fn fact(int n) {\n    if n <= 1 { return 1; }\n    return n * fact(n - 1);\n}\nprintln(fact(7));\n",
  },
  {
    name: "loop_invariant.rz",
    source:
      "fn sum_to(int n)\n  requires n >= 0\n  ensures result >= 0\n{\n    let total = 0;\n    let i = 0;\n    while i < n\n      invariant total >= 0\n      invariant i >= 0\n    {\n        total = total + i;\n        i = i + 1;\n    }\n    return total;\n}\nprintln(sum_to(10));\n",
  },
];

const editorEl = document.getElementById("editor");
const outputEl = document.getElementById("output");
const footerEl = document.getElementById("output-footer");
const runBtn = document.getElementById("run-btn");
const clearBtn = document.getElementById("clear-btn");
const select = document.getElementById("example-select");
const banner = document.getElementById("scaffold-banner");

editorEl.value = EXAMPLES_FALLBACK[0].source;

const cm = CodeMirror.fromTextArea(editorEl, {
  mode: "rust",
  lineNumbers: true,
  theme: "idea",
  indentUnit: 4,
  smartIndent: true,
  autofocus: true,
});

let wasmReady = false;

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
  let languageVersion = null;
  let languageDate = null;
  try {
    const res = await fetch("./examples.json", { cache: "no-store" });
    if (res.ok) {
      const data = await res.json();
      // Support both old flat array format and new manifest format
      if (Array.isArray(data)) {
        manifest = data;
      } else if (data.examples && Array.isArray(data.examples)) {
        manifest = data.examples;
        languageVersion = data.language_version;
        languageDate = data.language_date;
      }
    }
  } catch (_err) {
    // fall through to fallback
  }
  const list = manifest && manifest.length ? manifest : EXAMPLES_FALLBACK;
  for (const ex of list) {
    const opt = document.createElement("option");
    opt.value = ex.name;
    opt.textContent = ex.name;
    opt.dataset.source = ex.source;
    select.appendChild(opt);
  }
  // Display language version if available
  if (languageVersion) {
    const shortCommit = languageVersion.substring(0, 8);
    footerEl.textContent = `examples pinned to language ${shortCommit} (${languageDate})`;
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
});

runBtn.addEventListener("click", async () => {
  if (!wasmReady) return;
  runBtn.disabled = true;
  outputEl.classList.remove("error");
  outputEl.textContent = "Running…";
  footerEl.textContent = "";

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

  if (stderr) {
    outputEl.classList.add("error");
    outputEl.textContent = stderr + (stdout ? "\n--\n" + stdout : "");
  } else {
    outputEl.textContent = stdout || "(no output)";
  }
  footerEl.textContent = `exit ${code} • ${ms} ms • ${flavor}`;
  runBtn.disabled = false;
});

runBtn.disabled = true;
loadExamples();
bootstrap();
