---
title: Home
layout: home
nav_order: 1
description: "Resilient — a statically-typed compiled language for safety-critical embedded systems. Contracts proven at compile time, self-healing live blocks, bare-metal no_std runtime."
permalink: /
---

<div class="rl-hero">
  <div class="rl-hero__left">
    <p class="rl-hero__eyebrow">safety-critical &nbsp;·&nbsp; embedded &nbsp;·&nbsp; verified</p>
    <h1>Prove it.<br>Ship it.<br><em>Trust it.</em></h1>
    <p class="rl-hero__sub">A compiled language where failure is a first-class concern. Contracts are proven at compile time, hardware faults self-heal, and the same code runs on a dev laptop or a bare-metal Cortex-M4.</p>
    <div class="rl-hero__actions">
      <a href="{{ '/getting-started' | relative_url }}" class="rl-btn rl-btn--primary">Get started →</a>
      <a href="{{ '/playground/' | relative_url }}" class="rl-btn rl-btn--secondary">Try the playground ↗</a>
      <a href="https://github.com/EricSpencer00/Resilient" class="rl-btn rl-btn--secondary" target="_blank">GitHub ↗</a>
    </div>
    <div class="rl-hero__badges">
      <span class="rl-badge">MIT License</span>
      <span class="rl-badge">no_std ready</span>
      <span class="rl-badge">JIT · VM · Interp</span>
      <span class="rl-badge">Z3 contract proofs</span>
    </div>
  </div>
  <div class="rl-hero__right">
    <div class="rl-code-frame">
      <div class="rl-code-frame__bar">
        <span class="rl-dot rl-dot--red"></span>
        <span class="rl-dot rl-dot--yellow"></span>
        <span class="rl-dot rl-dot--green"></span>
        <span class="rl-code-frame__label">altitude_controller.rz</span>
      </div>
<pre class="rl-code"><span class="rl-cm">// Fault-tolerant flight controller</span>
<span class="rl-kw">fn</span> <span class="rl-fn">read_pressure</span>(<span class="rl-ty">int</span> sensor_id) -&gt; <span class="rl-ty">float</span>
    <span class="rl-kw">requires</span> sensor_id &gt;= <span class="rl-nu">0</span> &amp;&amp; sensor_id &lt; <span class="rl-nu">4</span>
    <span class="rl-kw">ensures</span>  result &gt;= <span class="rl-nu">0.0</span> &amp;&amp; result &lt;= <span class="rl-nu">120_000.0</span>
{
    <span class="rl-kw">let</span> raw = hal::adc_read(sensor_id);
    <span class="rl-kw">return</span> calibrate(raw, PRESSURE_CAL[sensor_id]);
}

<span class="rl-kw">fn</span> <span class="rl-fn">altitude_controller</span>() {
    <span class="rl-kw">live</span> {
        <span class="rl-cm">// transient faults auto-retry — no crash</span>
        <span class="rl-kw">let</span> p   = read_pressure(PRIMARY_SENSOR);
        <span class="rl-kw">let</span> alt = barometric_altitude(p);

        <span class="rl-kw">assert</span>(alt &lt; MAX_ALTITUDE, <span class="rl-st">"ceiling exceeded"</span>);
        actuator::set_throttle(pid_update(alt));
    }
}</pre>
    </div>
  </div>
</div>

## Three pillars

<div class="rl-cards">
  <div class="rl-card">
    <span class="rl-card__glyph">live { }</span>
    <span class="rl-card__title">Resilience</span>
    <p class="rl-card__body">Failures are expected events, not exceptions. <code>live { }</code> blocks supervise execution — on a recoverable error the runtime restores state and retries automatically, with no crash and no watchdog reset required.</p>
  </div>
  <div class="rl-card">
    <span class="rl-card__glyph">requires / ensures</span>
    <span class="rl-card__title">Verifiability</span>
    <p class="rl-card__body">Function contracts are proven at compile time when the verifier can decide them. When it can't, they become typed runtime asserts. Either way, exportable SMT-LIB2 certificates let downstream consumers re-verify under their own solver.</p>
  </div>
  <div class="rl-card">
    <span class="rl-card__glyph">no_std</span>
    <span class="rl-card__title">Simplicity</span>
    <p class="rl-card__body">No macro system. No inheritance. No implicit conversions. The syntax surface is small by design — fewer places for a bug to hide. The same language targets both your dev laptop and a bare-metal microcontroller.</p>
  </div>
</div>

## Compiler at work

Write a function with contracts — the verifier tells you exactly what it proved at compile time and what becomes a runtime guard.

<div class="rl-terminal">
  <div class="rl-terminal__bar">$ rz --audit altitude_controller.rz</div>
  <pre><span class="rl-ok">✓</span>  <span class="rl-path">read_pressure</span>  <span class="rl-kw">requires</span>  sensor_id ∈ [0, 4)            proved
<span class="rl-ok">✓</span>  <span class="rl-path">read_pressure</span>  <span class="rl-kw">ensures</span>   result ∈ [0.0, 120 000.0 Pa]  proved
<span class="rl-warn">~</span>  <span class="rl-path">altitude_controller</span>  assert(alt &lt; MAX_ALTITUDE)   runtime  (MAX_ALTITUDE is symbolic)

<span class="rl-cert">Certificate →</span> ./certs/altitude_controller.smtlib2</pre>
</div>

Contracts that can't be discharged at compile time become typed runtime asserts — never silently ignored. [Full verification docs →](language-reference#contracts)

## Performance

<div class="rl-stats">
  <div class="rl-stat">
    <div class="rl-stat__value">145×</div>
    <div class="rl-stat__label">JIT vs interpreter</div>
  </div>
  <div class="rl-stat">
    <div class="rl-stat__value">2.8 ms</div>
    <div class="rl-stat__label">fib(25) on M1 Max</div>
  </div>
  <div class="rl-stat">
    <div class="rl-stat__value">1.4×</div>
    <div class="rl-stat__label">of native Rust (JIT)</div>
  </div>
  <div class="rl-stat">
    <div class="rl-stat__value">3</div>
    <div class="rl-stat__label">execution backends</div>
  </div>
</div>

Tree-walking interpreter for fast iteration → bytecode VM (~12×) → Cranelift JIT (~145×, within 1.4× of native Rust). Pick the backend that matches your deploy target. [Benchmark methodology →](performance)

## What's in the box

| Surface | Status | How to invoke |
|---|---|---|
| Tree-walking interpreter | ✅ stable | `rz prog.rz` |
| Bytecode VM | ✅ stable | `rz --vm prog.rz` |
| Cranelift JIT | ✅ stable subset | `rz --jit prog.rz` (build with `--features jit`) |
| Z3 contract proofs | ✅ opt-in | `rz --audit prog.rz` (build with `--features z3`) |
| SMT-LIB2 certificates | ✅ opt-in | `rz --emit-certificate ./certs/ prog.rz` |
| Language Server (LSP) | ✅ opt-in | `rz --lsp` (build with `--features lsp`) |
| `#![no_std]` runtime | ✅ stable | `resilient-runtime/` crate |

## How Resilient compares

If you're evaluating Resilient against an existing safety-critical
toolchain, start with the side-by-side comparisons:

- **[Resilient vs Rust for embedded](compare/rust-vs-resilient)** —
  for teams already using `embedded-rust` and looking for sharper
  compile-time guarantees.
- **[Resilient vs Ada / SPARK](compare/ada-spark-vs-resilient)** —
  for teams in defense, avionics, or rail evaluating a modern
  formally-verified alternative.
- **[Resilient vs MISRA C](compare/misra-c-vs-resilient)** —
  for automotive and industrial teams reaching the limits of MISRA C:2012.

## Standards mapping

Resilient is not a certified tool, but its features map directly to
specific objectives in the major safety standards:

- **[DO-178C (avionics)](standards/do-178c)** — objective-by-objective mapping
- **[ISO 26262 (automotive ASIL D)](standards/iso-26262)** — objective-by-objective mapping
- **[IEC 62304 (medical devices)](standards/iec-62304)** — class C software mapping

## Open source

Resilient is MIT-licensed. Contributions from humans and AI agents are equally welcome — work is tracked in [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues), and `cargo test` is the acceptance gate.

[Contributing guide](contributing){: .btn .btn-outline .mr-2 }
[Community & Open Source](community){: .btn .btn-outline }

---

## Where next?

- **Try in your browser, no install** → [🛝 Playground ↗](https://ericspencer.us/Resilient/playground/)
- **New here?** → [Getting Started](getting-started)
- **Mission-critical example projects** → [Resilient-examples ↗](https://github.com/EricSpencer00/Resilient-examples) (pacemaker, infusion pump, ABS, traffic-light interlock, reactor monitor, CAN parser)
- **Learn the syntax** → [Syntax Reference](syntax)
- **Contracts and formal verification** → [Language Reference](language-reference)
- **Embedded / bare-metal** → [no\_std runtime](no-std)
- **DO-178C / ISO 26262 / IEC 61508** → [Certification and Safety Standards](certification)
- **Editor setup** → [LSP / Editor Integration](lsp)
- **Contributing** → [Contributing guide](contributing) and [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues)
