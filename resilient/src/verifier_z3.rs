// verifier_z3.rs
//
// RES-067: Z3 SMT integration for the contract verifier.
//
// The hand-rolled folder (RES-060..065) handles a narrow but very
// useful subset of contract clauses. This module backstops it: when
// the folder returns Unknown, we hand the clause to Z3 and ask
// whether it's a tautology, a contradiction, or actually undecidable.
//
// The translation supports (LIA path):
//   - integer literals
//   - identifiers (free or bound to a known integer in `bindings`)
//   - +, -, *, /, %  on integers
//   - ==, !=, <, >, <=, >=  comparisons
//   - !, &&, ||  logical connectives
//   - true, false
//
// The translation supports (BV32 path — RES-354):
//   - All of the above, plus
//   - &, |, ^  bitwise AND/OR/XOR
//   - <<, >>   left/right shifts
//   All variables and constants are BV<32>; comparisons use signed BV.
//
// Anything outside the supported subset makes us bail to None — the
// existing runtime check still fires.
//
// RES-354: theory selection
//   - Z3Theory::Auto  — use BV32 if any bitwise op is present, else LIA
//   - Z3Theory::Bv    — always use BV32
//   - Z3Theory::Lia   — always use LIA (error if bitwise ops present)

use crate::{ActorHandler, Node};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use z3::Sort;
use z3::ast::{Array, Ast, BV, Bool, Int};

// ============================================================
// RES-1188: process-local Z3 context reuse
// ============================================================
//
// Every `prove_*` call used to do:
//
//     let cfg = z3::Config::new();
//     let ctx = z3::Context::new(&cfg);
//
// which calls `Z3_mk_context_rc` inside libz3 — that allocates the
// theory plugins, the AST manager, the symbol table, and assorted
// per-context bookkeeping. For a single `cargo test --features z3`
// run the verifier fires hundreds of these (one per bounds check, one
// per loop-invariant induction step + base case, one per alias-
// disjointness query, …), so the constant-factor setup cost adds up
// to a measurable fraction of total wall time.
//
// Z3 contexts are inherently per-thread (their internal allocators
// aren't synchronised), so a `thread_local!` is the right ownership
// model: each test thread lazily initialises its own `Context` on
// first use, every subsequent query in that thread reuses it, and
// the per-context cleanup (`Z3_del_context`) fires once at thread
// exit.
//
// Each query still creates its own `Solver` inside the shared
// context, so query state remains isolated: asserting `formula` on
// a fresh solver doesn't leak into the next solver's assertion set.
// What's shared is the *infrastructure* (theory plugins, sort
// canonicaliser, symbol interner), which is exactly the heavy bit.

thread_local! {
    static Z3_CTX: z3::Context = z3::Context::new(&z3::Config::new());
}

// ============================================================
// RES-354: SMT theory selection
// ============================================================

/// Which Z3 theory to use for encoding integer arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Z3Theory {
    /// Auto-detect: use BV32 if any bitwise operation is present in
    /// the formula, LIA otherwise.
    #[default]
    Auto,
    /// Always encode as 32-bit bit-vectors (QF_BV).
    Bv,
    /// Always encode as linear integer arithmetic (QF_LIA / AUFLIA).
    Lia,
}

/// RES-1675: fold an expression to a constant `i64` when every leaf
/// is a literal and every operator is a checked integer arithmetic
/// operation. Returns `None` on:
///
/// - Free identifiers, function calls, indexing, etc.
/// - Overflow (don't unsoundly wrap; let Z3 see the original tree).
/// - Division or modulo by zero.
///
/// Used by `try_const_eval_bool` to fold the operands of a comparison
/// before checking the literal-vs-literal arm — so `5 + 3 == 8` and
/// `MIN + OFFSET >= 0` (after upstream constant propagation lands
/// integer literals on both sides) both decide without Z3.
fn try_const_int(expr: &Node) -> Option<i64> {
    match expr {
        Node::IntegerLiteral { value, .. } => Some(*value),
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => try_const_int(right).and_then(i64::checked_neg),
        Node::InfixExpression {
            left,
            operator,
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
                // RES-1678: bitwise ops over integer literals. `&`,
                // `|`, `^` are identical in LIA i64 and BV32 (no
                // overflow). Shifts use checked_shl / checked_shr so
                // out-of-range shift amounts (negative, or >= 64)
                // return None and let Z3 see the original expression
                // — preserves soundness even though BV32 wraps and
                // i64 doesn't.
                "&" => Some(l & r),
                "|" => Some(l | r),
                "^" => Some(l ^ r),
                "<<" => {
                    if !(0..64).contains(&r) {
                        None
                    } else {
                        l.checked_shl(r as u32)
                    }
                }
                ">>" => {
                    if !(0..64).contains(&r) {
                        None
                    } else {
                        l.checked_shr(r as u32)
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// RES-1663 / RES-1665 / RES-1667 / RES-1673 / RES-1675: pre-Z3
/// constant folding over the obligation AST. Returns `Some(verdict)`
/// when the expression's value is fully determined by literals alone
/// — bindings, axioms, theory, and timeout cannot change the result.
///
/// Covered shapes:
///
/// 1. `BooleanLiteral(v)` → `Some(v)` (RES-1663).
/// 2. Comparisons over expressions that fold to an `i64` via
///    `try_const_int` — covers raw literals (RES-1665) and arithmetic
///    over literals like `5 + 3 == 8` (RES-1675).
/// 3. `!expr` (RES-1667) — recurse and negate.
/// 4. `lhs && rhs` (RES-1667) — short-circuit on a constant-False
///    side; fold to `true` only when both sides are constant-True.
/// 5. `lhs || rhs` (RES-1667) — short-circuit on a constant-True
///    side; fold to `false` only when both sides are constant-False.
/// 6. Reflexive `Identifier OP Identifier` comparisons (RES-1673).
///
/// Recursion is bounded by AST depth; typical obligations are < 10
/// deep. The fast path runs in constant time relative to the cache
/// hash + persistent-set lookup it replaces.
///
/// These shapes appear in real obligations after constant propagation
/// through inlined helpers, contract specialisation, `@trusted`
/// ensures rewrites, and partially-folded contracts the typechecker
/// has normalized but not fully eliminated.
fn try_const_eval_bool(expr: &Node) -> Option<bool> {
    match expr {
        Node::BooleanLiteral { value, .. } => Some(*value),
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "!" => try_const_eval_bool(right).map(|v| !v),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            // RES-1675: integer-arithmetic constant fold. Try to
            // reduce both sides to a constant `i64`; if either side
            // doesn't fold (free var, overflow, div-by-zero), fall
            // through. Subsumes the original RES-1665 raw-literal
            // arm — a bare `IntegerLiteral` is a one-step fold.
            if let (Some(a), Some(b)) = (try_const_int(left), try_const_int(right)) {
                return match operator.as_str() {
                    "==" => Some(a == b),
                    "!=" => Some(a != b),
                    "<" => Some(a < b),
                    "<=" => Some(a <= b),
                    ">" => Some(a > b),
                    ">=" => Some(a >= b),
                    _ => None,
                };
            }
            // RES-1673: reflexive Identifier comparison. `x == x` is
            // tautologically true in LIA/BV theory (no NaN concerns);
            // `x != x` is tautologically false. Same for `<= >= < >`.
            // Pattern emerges from inlined helpers and contract
            // specialisation where both sides collapse to the same
            // parameter name. Restricted to Identifier-vs-Identifier
            // to avoid the hash-collision soundness risk of
            // generalised structural equality.
            if let (Node::Identifier { name: a, .. }, Node::Identifier { name: b, .. }) =
                (left.as_ref(), right.as_ref())
                && a == b
            {
                return match operator.as_str() {
                    "==" | "<=" | ">=" => Some(true),
                    "!=" | "<" | ">" => Some(false),
                    _ => None,
                };
            }
            // RES-1680 / RES-1684: generalized boolean-comparison
            // fold. When the operator is `==` or `!=`, recursively
            // fold both sides as bool expressions; if both fold,
            // return the comparison verdict directly. Subsumes the
            // pre-RES-1684 BoolLit-vs-BoolLit arm and also catches
            // mixed shapes like `(x == x) == true` (left folds via
            // RES-1673 reflexive, right is a bool literal).
            //
            // Termination: the recursion strictly shrinks the AST
            // subtree so we always halt at the leaf level.
            //
            // Other operators (`&&`, `||`, `<`, ...) fall through to
            // the bool-combinator arm below — `BoolLit(true) && BoolLit(true)`
            // must still fold via the `&&` combinator path.
            if matches!(operator.as_str(), "==" | "!=")
                && let (Some(la), Some(rb)) =
                    (try_const_eval_bool(left), try_const_eval_bool(right))
            {
                return Some(if operator == "==" { la == rb } else { la != rb });
            }
            // Boolean combinators — recurse to fold known sides,
            // short-circuiting before the other side has to fold.
            match operator.as_str() {
                "&&" => {
                    let l = try_const_eval_bool(left);
                    if l == Some(false) {
                        return Some(false);
                    }
                    let r = try_const_eval_bool(right);
                    if r == Some(false) {
                        return Some(false);
                    }
                    if l == Some(true) && r == Some(true) {
                        return Some(true);
                    }
                    None
                }
                "||" => {
                    let l = try_const_eval_bool(left);
                    if l == Some(true) {
                        return Some(true);
                    }
                    let r = try_const_eval_bool(right);
                    if r == Some(true) {
                        return Some(true);
                    }
                    if l == Some(false) && r == Some(false) {
                        return Some(false);
                    }
                    None
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Return `true` if `node` or any sub-expression uses a bitwise
/// operator (`&`, `|`, `^`, `<<`, `>>`).
pub fn has_bitwise_ops(node: &Node) -> bool {
    match node {
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            matches!(operator.as_str(), "&" | "|" | "^" | "<<" | ">>")
                || has_bitwise_ops(left)
                || has_bitwise_ops(right)
        }
        Node::PrefixExpression { right, .. } => has_bitwise_ops(right),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => has_bitwise_ops(function) || arguments.iter().any(has_bitwise_ops),
        _ => false,
    }
}

/// RES-071: a re-verifiable SMT-LIB2 certificate captured when Z3
/// successfully discharges a contract obligation. Feeding the
/// `smt2` string to a stock Z3 (`z3 -smt2 cert.smt2`) must print
/// `unsat`, confirming the proof without trusting our binary.
#[derive(Debug, Clone)]
pub struct ProofCertificate {
    pub smt2: String,
}

/// Return Some(true) if the expression is provably always true under
/// the bindings, Some(false) if provably always false, None if
/// undecidable or out of the supported subset.
///
/// Thin wrapper over `prove_with_certificate` for callers that don't
/// need the SMT-LIB2 dump.
#[allow(dead_code)]
pub fn prove(expr: &Node, bindings: &HashMap<String, i64>) -> Option<bool> {
    prove_with_certificate(expr, bindings).0
}

/// RES-071: like `prove`, but ALSO returns a self-contained
/// SMT-LIB2 certificate when the verdict is `Some(true)`. The
/// certificate, fed to stock Z3, must print `unsat` — that is, the
/// negation of the contract clause is unsatisfiable, which is the
/// definition of a tautology proof. For `Some(false)` and `None`
/// verdicts the certificate is omitted.
pub fn prove_with_certificate(
    expr: &Node,
    bindings: &HashMap<String, i64>,
) -> (Option<bool>, Option<ProofCertificate>) {
    let (verdict, cert, _cx) = prove_with_certificate_and_counterexample(expr, bindings);
    (verdict, cert)
}

/// RES-136: full diagnostic version of `prove_with_certificate`.
/// Returns a third slot — a formatted counterexample — populated
/// when the negated formula is *satisfiable* (i.e. there is an
/// assignment that falsifies the clause). Callers that surface a
/// "could not prove" or "contract cannot hold" diagnostic to the
/// user can append this string to the error message.
///
/// The counterexample is `Some(...)` only when:
///   - The verdict is `Some(false)` (the clause is a contradiction —
///     any assignment falsifies it), OR
///   - The verdict is `None` (undecidable — at least one concrete
///     assignment was found to falsify the clause).
///
/// For `Some(true)` tautology proofs there is no counterexample and
/// the slot is `None`.
///
/// Format matches the ticket (`a = -1, b = 0`): identifier bindings
/// comma-separated, in deterministic BTreeSet order. Variables with
/// no assignment in the model are omitted — Z3 may elide them.
pub fn prove_with_certificate_and_counterexample(
    expr: &Node,
    bindings: &HashMap<String, i64>,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>) {
    let (v, c, cx, _timed_out) = prove_with_timeout(expr, bindings, 0);
    (v, c, cx)
}

/// RES-137: like `prove_with_certificate_and_counterexample` but
/// with a per-query wall-clock timeout in milliseconds. A value of
/// 0 disables the timeout (use the solver's default, which is
/// unlimited).
///
/// The fourth return slot is `true` when Z3 reported `Unknown` —
/// i.e. the tautology check timed out. Callers treat this the same
/// as the existing `None` verdict (not proven → runtime check
/// retained) but get enough signal to emit a hint diagnostic and
/// to bump the `timed-out` audit counter.
pub fn prove_with_timeout(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    timeout_ms: u32,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>, bool) {
    prove_with_axioms_and_timeout(expr, bindings, &[], timeout_ms)
}

/// FFI Phase 1 Task 10: prove `expr` under an additional list of
/// free boolean `axioms` that the solver treats as `true`. Designed
/// for `@trusted` extern fn `ensures` clauses: a caller collects the
/// trusted ensures that reference values in scope, rewrites them so
/// every occurrence of `result` is replaced with the call site's
/// return-value identifier, and hands the list to this function as
/// axioms.
///
/// Axioms that fail to translate (unsupported nodes, floats, etc.)
/// are silently skipped — the same fail-open policy the rest of the
/// translator uses. A silently skipped axiom is safe: dropping
/// information can only weaken the assumption set, never make an
/// unsound verdict sound.
///
/// Return shape matches `prove_with_timeout`. The
/// certificate-generation path does NOT yet embed the axioms in the
/// emitted SMT-LIB2 because the re-verifier would need the same
/// axioms to reproduce the proof; callers that need re-verifiable
/// certificates for trusted-axiom-assisted proofs should persist
/// the axiom list alongside the certificate. Tracked as a follow-up.
#[allow(dead_code)]
pub fn prove_with_axioms_and_timeout(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    axioms: &[Node],
    timeout_ms: u32,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>, bool) {
    // RES-1663 / RES-1665 / RES-1667: pre-Z3 constant fold over the
    // obligation AST. Covers `BooleanLiteral`, literal-vs-literal
    // integer comparisons, negation of any of the above, and
    // `&&` / `||` short-circuits when at least one side is constant.
    // Returns Some(verdict) only when the result is fully determined
    // by literals; otherwise falls through to the cache + Z3 path.
    if let Some(verdict) = try_const_eval_bool(expr) {
        return (Some(verdict), None, None, false);
    }
    // RES-1206: thread-local verdict cache. Builds frequently re-ask
    // the verifier the same `(expr, bindings, axioms, timeout)` —
    // identical bounds checks on different call sites that share an
    // axiom set, the same `requires` clause re-checked at every
    // function callsite, etc. Each cache hit short-circuits the full
    // Z3 round trip (solver construction + formula assertion +
    // solve), which is by far the dominant cost even after RES-1188
    // (thread-local libz3 context) and RES-1194 (tautology fast
    // path) already paid down the setup cost.
    //
    // Key shape: AST debug-print of every input, separated by `|`
    // sentinels. `Node` doesn't derive `Hash`, so we lean on its
    // `Debug` impl — slower than a structural hash but still O(AST
    // size) per key, comfortably cheaper than the dispatch + solve
    // it replaces on a hit, and on a miss the format cost is
    // amortised against the Z3 work that follows. Lives in a
    // `RefCell` inside a `thread_local!` to match the
    // existing `Z3_CTX` ownership model — each test thread (and the
    // CLI's main thread) keeps its own cache without cross-thread
    // synchronisation.
    //
    // `bindings` is sorted into a `BTreeMap` before formatting so
    // identical input maps produce identical keys regardless of
    // `HashMap`'s nondeterministic iteration order — otherwise the
    // cache would miss on semantically equal queries.
    let key = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        // RES-1637: structural span-free hash for the obligation.
        hash_node_spanless(expr, &mut h);
        b'|'.hash(&mut h);
        let bindings_sorted: std::collections::BTreeMap<&String, &i64> = bindings.iter().collect();
        for (k, v) in &bindings_sorted {
            k.hash(&mut h);
            v.hash(&mut h);
        }
        b'|'.hash(&mut h);
        (axioms.len() as u32).hash(&mut h);
        for ax in axioms {
            hash_node_spanless(ax, &mut h);
        }
        b'|'.hash(&mut h);
        timeout_ms.hash(&mut h);
        h.finish()
    };
    // RES-1657: persistent proven-set short-circuit. If this key
    // was proven `Some(true)` in any prior invocation that called
    // `load_persistent_proven`, return immediately. Cert/cx are
    // dropped on the persistent path (rebuildable on demand by a
    // future caller via the live cache, which will repopulate on
    // first reuse).
    if persistent_proven_contains(key) {
        Z3_CACHE_STATS.with(|s| {
            let mut v = s.get();
            v.verdict_hits += 1;
            s.set(v);
        });
        return (Some(true), None, None, false);
    }
    if let Some(cached) = Z3_VERDICT_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        Z3_CACHE_STATS.with(|s| {
            let mut v = s.get();
            v.verdict_hits += 1;
            s.set(v);
        });
        return cached;
    }
    Z3_CACHE_STATS.with(|s| {
        let mut v = s.get();
        v.verdict_misses += 1;
        s.set(v);
    });
    let result = Z3_CTX
        .with(|ctx| prove_with_axioms_and_timeout_in(ctx, expr, bindings, axioms, timeout_ms));
    // RES-1657: persist the key when we proved Some(true). Skip
    // Some(false)/None — those are cheap to re-derive and shouldn't
    // pollute the persistent set.
    if matches!(result.0, Some(true)) {
        persistent_proven_insert(key);
    }
    Z3_VERDICT_CACHE.with(|c| {
        c.borrow_mut().insert(key, result.clone());
    });
    result
}

/// RES-1657: process-wide persistent proven set.
///
/// Stores `u64` keys (computed by the same `hash_node_spanless`
/// pipeline as the thread-local caches) for obligations that have
/// proven `Some(true)` at some point in this process. Sits in front
/// of the thread-local caches so cross-thread reuse short-circuits
/// without re-running Z3.
///
/// Cert + counterexample are not persisted — on a hit we return
/// `(Some(true), None, None, false)` and any caller that needs a
/// fresh cert re-derives it. The vast majority of callers only
/// branch on the verdict.
///
/// `[load|save]_persistent_proven` provide JSON I/O for survival
/// across processes; the caller decides when to load/save (future
/// CLI flag).
// RES-1657 introduced this static with a bare `HashSet::new()`
// initializer, which is not `const` on stable Rust — `cargo test
// --features z3` has been red on `main` ever since. Wrap in
// `LazyLock` so the HashSet is built on first access. RES-1661.
static PERSISTENT_PROVEN: std::sync::LazyLock<std::sync::RwLock<std::collections::HashSet<u64>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(std::collections::HashSet::new()));

// RES-1700: AtomicBool fast-reject. Same pattern as RES-1374 for
// `feature_attrs::find_kind`. Default-mode typechecks (no
// `--persistent-proof-cache` flag) leave the set empty; each
// `persistent_proven_contains` call then drops from a ~100ns RwLock
// read acquire to a ~1ns atomic load. With three caller sites per
// prove (LIA, tautology, BV32) and 100+ obligations per typecheck,
// that's ~30µs saved per typecheck against the no-persistent-cache
// path. Acquire / Release ordering pairs the flag store in
// `persistent_proven_insert` and `load_persistent_proven` with the
// load here.
static PERSISTENT_PROVEN_HAS_ENTRIES: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn persistent_proven_contains(key: u64) -> bool {
    if !PERSISTENT_PROVEN_HAS_ENTRIES.load(std::sync::atomic::Ordering::Acquire) {
        return false;
    }
    PERSISTENT_PROVEN
        .read()
        .map(|s| s.contains(&key))
        .unwrap_or(false)
}

fn persistent_proven_insert(key: u64) {
    if let Ok(mut s) = PERSISTENT_PROVEN.write() {
        s.insert(key);
        PERSISTENT_PROVEN_HAS_ENTRIES.store(true, std::sync::atomic::Ordering::Release);
    }
}

/// RES-1657 / RES-1661: load a persistent proven-set from disk.
///
/// On-disk format is a versioned JSON envelope:
/// `{"compiler_version": "...", "keys": [u64, ...]}`.
///
/// Missing file is not an error (treat as "no cache yet"). A
/// `compiler_version` that does not match `env!("CARGO_PKG_VERSION")`
/// silently returns `Ok(0)` — the AST shape may have shifted between
/// versions, which would invalidate keys produced by `hash_node_spanless`.
/// Malformed JSON (including the pre-RES-1661 flat-array format) also
/// silently returns `Ok(0)` — equivalent to a fresh cache.
pub fn load_persistent_proven(path: &std::path::Path) -> std::io::Result<usize> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Ok(0),
    };
    let stored_version = v
        .get("compiler_version")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if stored_version != env!("CARGO_PKG_VERSION") {
        return Ok(0);
    }
    let keys_arr = match v.get("keys").and_then(|x| x.as_array()) {
        Some(a) => a,
        None => return Ok(0),
    };
    let mut n = 0;
    if let Ok(mut s) = PERSISTENT_PROVEN.write() {
        for k in keys_arr {
            if let Some(k_u64) = k.as_u64() {
                s.insert(k_u64);
                n += 1;
            }
        }
        // RES-1700: flip the flag once instead of per insert.
        if n > 0 {
            PERSISTENT_PROVEN_HAS_ENTRIES.store(true, std::sync::atomic::Ordering::Release);
        }
    }
    Ok(n)
}

/// RES-1657 / RES-1661: save the current persistent proven-set to
/// disk as a versioned JSON envelope. The parent directory is created
/// if missing.
pub fn save_persistent_proven(path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let keys: Vec<u64> = PERSISTENT_PROVEN
        .read()
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    let envelope = serde_json::json!({
        "compiler_version": env!("CARGO_PKG_VERSION"),
        "keys": keys,
    });
    let text = serde_json::to_string(&envelope).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// RES-1641: cache-hit telemetry. Snapshot of the three thread-local
/// verifier caches' hit + miss counts since the last `reset_cache_stats`.
/// Returned by [`cache_stats`] for contributors who want to measure
/// cache effectiveness across the RES-1635 / RES-1637 / RES-1639
/// optimization series.
#[derive(Debug, Clone, Copy, Default)]
pub struct Z3CacheStats {
    pub verdict_hits: u64,
    pub verdict_misses: u64,
    pub tautology_hits: u64,
    pub tautology_misses: u64,
    pub bv_hits: u64,
    pub bv_misses: u64,
}

thread_local! {
    /// RES-1641: hit/miss counters for the three caches below.
    /// Thread-local so concurrent test runs don't cross-contaminate.
    static Z3_CACHE_STATS: std::cell::Cell<Z3CacheStats> = const {
        std::cell::Cell::new(Z3CacheStats {
            verdict_hits: 0,
            verdict_misses: 0,
            tautology_hits: 0,
            tautology_misses: 0,
            bv_hits: 0,
            bv_misses: 0,
        })
    };
}

/// RES-1641: return the current `Z3CacheStats` snapshot for this
/// thread. Counters accumulate from process start (or the last
/// [`reset_cache_stats`] call).
pub fn cache_stats() -> Z3CacheStats {
    Z3_CACHE_STATS.with(|s| s.get())
}

/// RES-1641: zero out all six counters. Useful for measuring a
/// single compilation in isolation. Currently only exercised by
/// the RES-1663 fast-path tests; left `pub` so future audit
/// machinery can call it without re-exposing.
#[allow(dead_code)]
pub fn reset_cache_stats() {
    Z3_CACHE_STATS.with(|s| s.set(Z3CacheStats::default()));
}

thread_local! {
    /// RES-1206: see `prove_with_axioms_and_timeout`. Stays the lifetime
    /// of the thread; reset between top-level compiles isn't needed
    /// because the key fully identifies the input, so stale entries
    /// can only be re-hit by an identical query (which would be
    /// correct either way).
    ///
    /// RES-1635: key is a `u64` hash streamed from the same Debug
    /// format the original String-keyed cache used. Same uniqueness
    /// guarantees, but each lookup skips the per-call `String`
    /// allocation that the format-then-store-then-hash sequence
    /// paid before.
    #[allow(clippy::type_complexity)]
    static Z3_VERDICT_CACHE: std::cell::RefCell<
        std::collections::HashMap<
            u64,
            (Option<bool>, Option<ProofCertificate>, Option<String>, bool),
        >,
    > = std::cell::RefCell::new(std::collections::HashMap::with_capacity(64));

    /// RES-1309 / RES-1635: thread-local cache for the tautology-only
    /// fast path. Disjoint key namespace from `Z3_VERDICT_CACHE` (the
    /// hash input ends with a `|TAUT` discriminator).
    #[allow(clippy::type_complexity)]
    static Z3_TAUTOLOGY_CACHE: std::cell::RefCell<
        std::collections::HashMap<u64, (bool, Option<ProofCertificate>, bool)>,
    > = std::cell::RefCell::new(std::collections::HashMap::with_capacity(64));

    /// RES-1316 / RES-1635: thread-local cache for the BV32 entry
    /// point. Disjoint key namespace (`|BV` suffix).
    #[allow(clippy::type_complexity)]
    static Z3_BV_CACHE: std::cell::RefCell<
        std::collections::HashMap<
            u64,
            (Option<bool>, Option<ProofCertificate>, Option<String>, bool),
        >,
    > = std::cell::RefCell::new(std::collections::HashMap::with_capacity(64));
}

/// RES-1635: a `fmt::Write` adapter that streams formatted bytes
/// directly into a `Hasher` instead of an intermediate `String`.
/// Used by the cache-key construction sites in `prove_with_axioms_
/// and_timeout`, `prove_tautology_with_axioms_and_timeout`, and
/// `prove_bv` to avoid the per-call `format!` allocation. Same
/// bytes hashed as the old `format!("...").into_bytes() → hash`
/// pipeline, so identical inputs still produce identical u64 keys
/// even after rebuilds.
/// RES-1637: span-free recursive hash for `Node`. Covers the
/// variants that dominate `requires`/`ensures` contract clauses
/// (literals, identifiers, infix/prefix expressions, calls, field
/// access, index, array, range). For unsupported variants, falls
/// back to span-inclusive `format!("{:?}", n)` so behaviour is
/// strictly no-worse than the pre-RES-1637 cache.
///
/// The point is to make structurally-identical obligations at
/// different call sites hash to the same key — today every site
/// carries a distinct span and cache-misses. After this, the
/// common arithmetic-comparison shape dedupes across sites.
fn hash_node_spanless<H: std::hash::Hasher>(node: &Node, h: &mut H) {
    use std::hash::Hash;
    // One-byte discriminant per variant so distinct shapes never
    // collide on accident.
    match node {
        Node::IntegerLiteral { value, .. } => {
            b'I'.hash(h);
            value.hash(h);
        }
        Node::FloatLiteral { value, .. } => {
            b'F'.hash(h);
            // f64 doesn't impl Hash directly; hash the bit pattern.
            value.to_bits().hash(h);
        }
        Node::StringLiteral { value, .. } => {
            b'S'.hash(h);
            value.hash(h);
        }
        Node::BytesLiteral { value, .. } => {
            b'Y'.hash(h);
            value.hash(h);
        }
        Node::BooleanLiteral { value, .. } => {
            b'B'.hash(h);
            value.hash(h);
        }
        Node::Identifier { name, .. } => {
            b'i'.hash(h);
            name.hash(h);
        }
        Node::PrefixExpression {
            operator, right, ..
        } => {
            b'P'.hash(h);
            operator.hash(h);
            hash_node_spanless(right, h);
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            b'X'.hash(h);
            operator.hash(h);
            hash_node_spanless(left, h);
            hash_node_spanless(right, h);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            b'C'.hash(h);
            hash_node_spanless(function, h);
            (arguments.len() as u32).hash(h);
            for a in arguments {
                hash_node_spanless(a, h);
            }
        }
        Node::FieldAccess { target, field, .. } => {
            b'.'.hash(h);
            field.hash(h);
            hash_node_spanless(target, h);
        }
        Node::IndexExpression { target, index, .. } => {
            b'['.hash(h);
            hash_node_spanless(target, h);
            hash_node_spanless(index, h);
        }
        Node::ArrayLiteral { items, .. } => {
            b'A'.hash(h);
            (items.len() as u32).hash(h);
            for it in items {
                hash_node_spanless(it, h);
            }
        }
        Node::Range {
            lo, hi, inclusive, ..
        } => {
            b'R'.hash(h);
            inclusive.hash(h);
            hash_node_spanless(lo, h);
            hash_node_spanless(hi, h);
        }
        // RES-1639: additional variants that appear in non-typechecker
        // Z3 callers (`cluster_verifier`, `bounds_check`,
        // `verifier_actors`, `verifier_loop_invariants`). The same
        // shape — one-byte discriminant + non-span fields + recursion.
        Node::Block { stmts, .. } => {
            b'{'.hash(h);
            (stmts.len() as u32).hash(h);
            for s in stmts {
                hash_node_spanless(s, h);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            b'?'.hash(h);
            b'I'.hash(h); // distinct from the fallback's b'?'
            hash_node_spanless(condition, h);
            hash_node_spanless(consequence, h);
            match alternative {
                Some(alt) => {
                    b'E'.hash(h);
                    hash_node_spanless(alt, h);
                }
                None => b'N'.hash(h),
            }
        }
        Node::TryExpression { expr, .. } => {
            b'T'.hash(h);
            hash_node_spanless(expr, h);
        }
        Node::LetStatement { name, value, .. } => {
            b'L'.hash(h);
            name.hash(h);
            hash_node_spanless(value, h);
        }
        Node::Assignment { name, value, .. } => {
            b'='.hash(h);
            name.hash(h);
            hash_node_spanless(value, h);
        }
        Node::ReturnStatement { value, .. } => {
            b'r'.hash(h);
            match value {
                Some(v) => {
                    b'V'.hash(h);
                    hash_node_spanless(v, h);
                }
                None => b'N'.hash(h),
            }
        }
        Node::StructLiteral { name, fields, .. } => {
            b's'.hash(h);
            name.hash(h);
            (fields.len() as u32).hash(h);
            // Field iteration order matters; struct literals don't
            // normalize field order (the parser preserves source
            // order). Same source → same iteration → same hash.
            for (fname, fval) in fields {
                fname.hash(h);
                hash_node_spanless(fval, h);
            }
        }
        // RES-1647: five more statement variants — same
        // discriminator + non-span-fields shape as RES-1639's batch.
        Node::Const { name, value, .. } => {
            b'c'.hash(h);
            name.hash(h);
            hash_node_spanless(value, h);
        }
        Node::StaticLet { name, value, .. } => {
            b'$'.hash(h);
            name.hash(h);
            hash_node_spanless(value, h);
        }
        Node::WhileStatement {
            condition,
            body,
            invariants,
            ..
        } => {
            b'w'.hash(h);
            hash_node_spanless(condition, h);
            hash_node_spanless(body, h);
            (invariants.len() as u32).hash(h);
            for inv in invariants {
                hash_node_spanless(inv, h);
            }
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            invariants,
            ..
        } => {
            b'f'.hash(h);
            name.hash(h);
            hash_node_spanless(iterable, h);
            hash_node_spanless(body, h);
            (invariants.len() as u32).hash(h);
            for inv in invariants {
                hash_node_spanless(inv, h);
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            b';'.hash(h);
            hash_node_spanless(expr, h);
        }
        // RES-1649: four more high-value variants.
        Node::Quantifier {
            kind,
            var,
            range,
            body,
            ..
        } => {
            b'Q'.hash(h);
            // QuantifierKind doesn't derive Hash; match it.
            match kind {
                crate::quantifiers::QuantifierKind::Forall => b'A'.hash(h),
                crate::quantifiers::QuantifierKind::Exists => b'E'.hash(h),
            }
            var.hash(h);
            // QuantRange recurses into Node sub-trees.
            match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    b'R'.hash(h);
                    hash_node_spanless(lo, h);
                    hash_node_spanless(hi, h);
                }
                crate::quantifiers::QuantRange::Iterable(it) => {
                    b'I'.hash(h);
                    hash_node_spanless(it, h);
                }
            }
            hash_node_spanless(body, h);
        }
        Node::InvariantStatement { expr, .. } => {
            b'v'.hash(h);
            hash_node_spanless(expr, h);
        }
        Node::ImplBlock {
            trait_name,
            struct_name,
            methods,
            ..
        } => {
            b'b'.hash(h);
            match trait_name {
                Some(t) => {
                    b'T'.hash(h);
                    t.hash(h);
                }
                None => b'N'.hash(h),
            }
            struct_name.hash(h);
            (methods.len() as u32).hash(h);
            for m in methods {
                hash_node_spanless(m, h);
            }
        }
        Node::ModuleDecl { name, body, .. } => {
            b'm'.hash(h);
            name.hash(h);
            (body.len() as u32).hash(h);
            for n in body {
                hash_node_spanless(n, h);
            }
        }
        // RES-1645: Match — sub-hashes Pattern arms via
        // `hash_pattern_spanless` so pattern-shaped obligations
        // dedupe across sites just like the rest.
        Node::Match {
            scrutinee, arms, ..
        } => {
            b'M'.hash(h);
            hash_node_spanless(scrutinee, h);
            (arms.len() as u32).hash(h);
            for (pat, guard, body) in arms {
                hash_pattern_spanless(pat, h);
                match guard {
                    Some(g) => {
                        b'G'.hash(h);
                        hash_node_spanless(g, h);
                    }
                    None => b'N'.hash(h),
                }
                hash_node_spanless(body, h);
            }
        }
        // Fallback: variant not in the covered subset. Use the
        // existing span-inclusive Debug — strictly no-worse than
        // pre-RES-1637, and the same key still uniquely identifies
        // the obligation (just doesn't dedupe across sites).
        other => {
            b'@'.hash(h);
            format!("{:?}", other).hash(h);
        }
    }
}

/// RES-1645: span-free recursive hash for `Pattern`. Covers all 14
/// variants. The `Pattern::Literal(Node)` arm recurses back into
/// `hash_node_spanless` so a `Literal(Node::IntegerLiteral { value:
/// 42, span: ... })` hashes the same way regardless of the literal's
/// source span.
fn hash_pattern_spanless<H: std::hash::Hasher>(p: &crate::Pattern, h: &mut H) {
    use std::hash::Hash;
    match p {
        crate::Pattern::Literal(n) => {
            b'l'.hash(h);
            hash_node_spanless(n, h);
        }
        crate::Pattern::Identifier(name) => {
            b'd'.hash(h);
            name.hash(h);
        }
        crate::Pattern::Wildcard => {
            b'_'.hash(h);
        }
        crate::Pattern::Or(branches) => {
            b'|'.hash(h);
            (branches.len() as u32).hash(h);
            for b in branches {
                hash_pattern_spanless(b, h);
            }
        }
        crate::Pattern::Range { lo, hi, inclusive } => {
            b'r'.hash(h);
            lo.hash(h);
            hi.hash(h);
            inclusive.hash(h);
        }
        crate::Pattern::Bind(name, inner) => {
            b'@'.hash(h);
            name.hash(h);
            hash_pattern_spanless(inner, h);
        }
        crate::Pattern::Struct {
            struct_name,
            fields,
            has_rest,
        } => {
            b's'.hash(h);
            struct_name.hash(h);
            has_rest.hash(h);
            (fields.len() as u32).hash(h);
            for (fname, fpat) in fields {
                fname.hash(h);
                hash_pattern_spanless(fpat, h);
            }
        }
        crate::Pattern::Some(inner) => {
            b'S'.hash(h);
            hash_pattern_spanless(inner, h);
        }
        crate::Pattern::None => {
            b'O'.hash(h);
        }
        crate::Pattern::Ok(inner) => {
            b'k'.hash(h);
            hash_pattern_spanless(inner, h);
        }
        crate::Pattern::Err(inner) => {
            b'e'.hash(h);
            hash_pattern_spanless(inner, h);
        }
        crate::Pattern::EnumVariant {
            type_name,
            variant_name,
            payload,
        } => {
            b'v'.hash(h);
            match type_name {
                Some(t) => {
                    b'T'.hash(h);
                    t.hash(h);
                }
                None => b'U'.hash(h),
            }
            variant_name.hash(h);
            hash_enum_payload_spanless(payload, h);
        }
        crate::Pattern::TupleStruct { name, fields } => {
            b't'.hash(h);
            name.hash(h);
            (fields.len() as u32).hash(h);
            for f in fields {
                hash_pattern_spanless(f, h);
            }
        }
        crate::Pattern::Tuple(parts) => {
            b'p'.hash(h);
            (parts.len() as u32).hash(h);
            for q in parts {
                hash_pattern_spanless(q, h);
            }
        }
    }
}

/// RES-1645: span-free recursive hash for `EnumPatternPayload`.
/// Three variants — none, named-fields, tuple-positional.
fn hash_enum_payload_spanless<H: std::hash::Hasher>(p: &crate::EnumPatternPayload, h: &mut H) {
    use std::hash::Hash;
    match p {
        crate::EnumPatternPayload::None => {
            b'0'.hash(h);
        }
        crate::EnumPatternPayload::Named(fields) => {
            b'1'.hash(h);
            (fields.len() as u32).hash(h);
            for (fname, fpat) in fields {
                fname.hash(h);
                hash_pattern_spanless(fpat, h);
            }
        }
        crate::EnumPatternPayload::Tuple(parts) => {
            b'2'.hash(h);
            (parts.len() as u32).hash(h);
            for q in parts {
                hash_pattern_spanless(q, h);
            }
        }
    }
}

// RES-1637 removed the `HasherWriter` adapter — `hash_node_spanless`
// feeds bytes directly into the `Hasher` via `Hash::hash` calls, with
// no `format!` step that would have needed a streaming write target.

fn prove_with_axioms_and_timeout_in(
    ctx: &z3::Context,
    expr: &Node,
    bindings: &HashMap<String, i64>,
    axioms: &[Node],
    timeout_ms: u32,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>, bool) {
    // Translate the expression to a Z3 boolean.
    let formula = match translate_bool(ctx, expr, bindings) {
        Some(f) => f,
        None => return (None, None, None, false),
    };

    // RES-137: apply the per-query timeout to both solvers below.
    // Z3's `"timeout"` param is in milliseconds; 0 disables it.
    // RES-1655: build the Params once per prove call rather than
    // per invocation of the closure — the closure runs twice (once
    // for the tautology solver, once for the contradiction solver),
    // and both invocations built an identical Params from scratch.
    let timeout_params = if timeout_ms > 0 {
        let mut p = z3::Params::new(ctx);
        p.set_u32("timeout", timeout_ms);
        Some(p)
    } else {
        None
    };
    let apply_timeout = |solver: &z3::Solver<'_>| {
        if let Some(ref p) = timeout_params {
            solver.set_params(p);
        }
    };

    // RES-131: collect every `len(<ident>)` reference in the
    // formula and inject `len_<ident> >= 0` as an axiom on each
    // solver. Without the axiom the solver treats `len_xs` as an
    // unconstrained Int, which is too loose to prove
    // `len(xs) > 0 → len(xs) >= 1`.
    // RES-1528: borrow each `len(<arg>)` ident name as `&str` from
    // the formula AST instead of cloning into a `BTreeSet<String>`.
    // The set is iterated to (a) build the `len_axioms` Z3 vec and
    // (b) emit `(declare-const len_X Int)` / `(assert (>= len_X 0))`
    // lines into the SMT cert. Both consumers only read the name —
    // the owned `String` keys were pure overhead, paid on *every*
    // Z3 prove call across the entire typecheck. Mirror of RES-1427
    // for the tuple element.
    let mut len_args: BTreeSet<&str> = BTreeSet::new();
    collect_len_args(expr, &mut len_args);
    // RES-1651: lift the zero constant out of the per-arg map closure.
    // Z3 `Int::from_i64(ctx, 0)` allocates a new AST node each call —
    // sharing one across all `len_X >= 0` axioms saves N-1 constructions
    // per Z3 prove on the cache-miss path.
    let zero = Int::from_i64(ctx, 0);
    let len_axioms: Vec<Bool<'_>> = len_args
        .iter()
        .map(|arg| {
            let c = Int::new_const(ctx, format!("len_{}", arg));
            c.ge(&zero)
        })
        .collect();

    // FFI Phase 1 Task 10: translate caller-supplied axioms. Each
    // axiom that successfully translates to a Z3 Bool is asserted
    // on both the tautology-check solver and the contradiction-check
    // solver — just like `len_axioms`. Axioms that translate to None
    // (unsupported nodes) are silently dropped.
    let user_axioms: Vec<Bool<'_>> = axioms
        .iter()
        .filter_map(|ax| translate_bool(ctx, ax, bindings))
        .collect();

    // RES-1696: share one Solver across the tautology + contradiction
    // checks via push/pop scopes. The axioms (`len_axioms` and
    // `user_axioms`) are common to both phases and only need to be
    // asserted once; previously they were re-asserted on a second
    // freshly-allocated Solver, paying both the duplicate Solver setup
    // and the duplicate axiom-assert work.
    let solver = z3::Solver::new(ctx);
    apply_timeout(&solver);
    for axiom in &len_axioms {
        solver.assert(axiom);
    }
    for axiom in &user_axioms {
        solver.assert(axiom);
    }

    // Tautology check: is `NOT formula` unsatisfiable? If yes, formula
    // is always true regardless of any free variables. Pushed onto a
    // scope so we can drop the negated assertion before re-using the
    // solver for the contradiction check.
    let negated = formula.not();
    solver.push();
    solver.assert(&negated);
    let check = solver.check();
    let tautology = matches!(check, z3::SatResult::Unsat);
    // RES-137: Z3 returns Unknown when the timeout fires (or when
    // the theory doesn't decide — QF_NIA, for instance).
    let timed_out = matches!(check, z3::SatResult::Unknown);

    // RES-136: extract a counterexample whenever the negated formula
    // is satisfiable — the model is an assignment that falsifies the
    // clause, which is what a user needs to see to debug a failing
    // contract. We harvest it eagerly so the later contradiction
    // check (after `solver.pop`) doesn't need to re-derive it. Done
    // BEFORE the `pop` because the model goes away when the scope
    // is dropped.
    let counterexample = if matches!(check, z3::SatResult::Sat) {
        extract_counterexample(ctx, &solver, expr, bindings)
    } else {
        None
    };
    solver.pop(1);

    if tautology {
        // Build a self-contained re-verifiable SMT-LIB2 file.
        // Strategy: declare every Int identifier that appears in the
        // expression, then constrain the bound ones to their concrete
        // value, then assert the NEGATED goal so a fresh Z3 returns
        // `unsat` (which is the proof that the original was always
        // true).
        let mut idents: BTreeSet<&str> = BTreeSet::new();
        collect_int_identifiers(expr, &mut idents);

        // RES-408: collect arrays referenced via `a[i]` so the cert
        // declares them with Z3's `(Array Int Int)` sort. Arrays
        // referenced *only* via `len(a)` (and not via index) keep
        // the historical behaviour — `len_a` Int const, no array
        // declaration needed.
        let mut arr_args: BTreeSet<&str> = BTreeSet::new();
        collect_array_args(expr, &mut arr_args);

        // RES-1383: write the SMT-LIB cert via `writeln!` into `smt2`
        // directly — `String` implements `fmt::Write`, so the format
        // machinery copies straight into the buffer with no
        // intermediate `String` allocations from `format!`. The
        // emitted text is byte-identical to the previous shape.
        // RES-1653: pre-size to 512 bytes — typical cert is 300-600
        // bytes, so the growth path hit 4-6 doubling reallocations
        // before. Capacity is per-Some(true)-tautology cost, so the
        // pre-size pays for itself on the first cert built.
        let mut smt2 = String::with_capacity(512);
        smt2.push_str("; RES-071 verification certificate\n");
        smt2.push_str("; expected solver result: unsat (proves the contract is a tautology)\n");
        smt2.push_str("(set-logic AUFLIA)\n");
        for name in &idents {
            writeln!(&mut smt2, "(declare-const {} Int)", name).unwrap();
        }
        // RES-131: declare one Int const per `len(<arg>)` call
        // seen in the formula + emit its `>= 0` axiom so a
        // stock Z3 re-verifying the cert gets the same
        // context the prover used.
        for arg in &len_args {
            writeln!(&mut smt2, "(declare-const len_{} Int)", arg).unwrap();
        }
        // RES-408: declare arrays referenced via `a[i]` with the
        // `(Array Int Int)` sort so the cert is self-contained for
        // stock Z3 re-verification.
        for arg in &arr_args {
            writeln!(&mut smt2, "(declare-const arr_{} (Array Int Int))", arg).unwrap();
        }
        for arg in &len_args {
            writeln!(&mut smt2, "(assert (>= len_{} 0))", arg).unwrap();
        }
        // Bound identifiers: pin them to their concrete value with an
        // equality assertion. Free identifiers are left unconstrained
        // so the proof is universal over them.
        for name in &idents {
            if let Some(v) = bindings.get(*name) {
                writeln!(&mut smt2, "(assert (= {} {}))", name, v).unwrap();
            }
        }
        // The negated goal — Z3 ASTs Display as SMT-LIB2 syntax, so
        // we get a faithful round-trip via `negated.to_string()`.
        writeln!(&mut smt2, "(assert {})", negated).unwrap();
        smt2.push_str("(check-sat)\n");

        return (Some(true), Some(ProofCertificate { smt2 }), None, false);
    }

    // Contradiction check: is `formula` unsatisfiable? If yes, the
    // contract can never hold. Re-uses the same `solver` instance —
    // axioms are still asserted from the pre-push setup above; we
    // only need to push the positive formula into a fresh scope.
    solver.push();
    solver.assert(&formula);
    let contradiction = matches!(solver.check(), z3::SatResult::Unsat);
    solver.pop(1);

    if contradiction {
        return (Some(false), None, counterexample, false);
    }

    (None, None, counterexample, timed_out)
}

/// RES-1194: tautology-only fast path.
///
/// Many callers (`bounds_check`, `verifier_loop_invariants`,
/// `prove_alias_disjoint`) only branch on `Some(true)` — they treat
/// `Some(false)` (provable contradiction) and `None` (uncertain) the
/// same: keep the runtime check. For those callers the
/// `prove_with_axioms_and_timeout` contradiction phase is dead work:
/// one full Z3 `solver.check()` per query whose verdict is
/// discarded.
///
/// This entry point runs **only** the tautology check, then returns
/// `(proven, cert_if_proven, timed_out)`. The certificate is still
/// emitted on tautology so callers that persist it (loop invariants)
/// keep working; the `Option<String>` counterexample slot is dropped
/// because by construction it's only meaningful when the caller
/// would have acted on `Some(false)` / `None`, which they don't.
///
/// Equivalent to dropping `Some(false)` and the counterexample from
/// `prove_with_axioms_and_timeout`'s return shape — modulo the
/// optimisation of not running the contradiction `check()` at all.
#[allow(dead_code)]
pub fn prove_tautology_with_axioms_and_timeout(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    axioms: &[Node],
    timeout_ms: u32,
) -> (bool, Option<ProofCertificate>, bool) {
    // RES-1663 / RES-1665 / RES-1667: pre-Z3 constant fold. See
    // `try_const_eval_bool` for the full pattern list. `(true, None,
    // false)` means tautology; `(false, None, false)` means
    // not-a-tautology — both verdicts come back without dispatching
    // Z3.
    if let Some(verdict) = try_const_eval_bool(expr) {
        return (verdict, None, false);
    }
    // RES-1309: thread-local verdict cache for the tautology-only
    // entry point. RES-1206 already caches results going through
    // `prove_with_axioms_and_timeout`, but this fast path is its own
    // function and was bypassing the cache entirely. Its callers —
    // `bounds_check::check_array_bounds` (every index expression),
    // `verifier_loop_invariants::verify_and_capture` (every
    // `invariant` clause), `prove_alias_disjoint` (every aliasing
    // query) — re-asked Z3 the same question every time the input
    // AST + bindings + axioms repeated.
    //
    // Key shape mirrors RES-1206: AST `Debug` of every input
    // separated by `|`, with bindings sorted into a `BTreeMap` for
    // deterministic ordering, and a `|TAUT` suffix so the cache key
    // namespace stays disjoint from the verdict cache.
    let key = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        hash_node_spanless(expr, &mut h);
        b'|'.hash(&mut h);
        let bindings_sorted: std::collections::BTreeMap<&String, &i64> = bindings.iter().collect();
        for (k, v) in &bindings_sorted {
            k.hash(&mut h);
            v.hash(&mut h);
        }
        b'|'.hash(&mut h);
        (axioms.len() as u32).hash(&mut h);
        for ax in axioms {
            hash_node_spanless(ax, &mut h);
        }
        b'|'.hash(&mut h);
        timeout_ms.hash(&mut h);
        // Discriminator: disjoint namespace from the verdict cache.
        b"TAUT".hash(&mut h);
        h.finish()
    };
    // RES-1657: persistent proven-set short-circuit (tautology path).
    // The tautology entry point returns `(bool, ...)`, so a persistent
    // hit produces `(true, None, false)` directly.
    if persistent_proven_contains(key) {
        Z3_CACHE_STATS.with(|s| {
            let mut v = s.get();
            v.tautology_hits += 1;
            s.set(v);
        });
        return (true, None, false);
    }
    if let Some(cached) = Z3_TAUTOLOGY_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        Z3_CACHE_STATS.with(|s| {
            let mut v = s.get();
            v.tautology_hits += 1;
            s.set(v);
        });
        return cached;
    }
    Z3_CACHE_STATS.with(|s| {
        let mut v = s.get();
        v.tautology_misses += 1;
        s.set(v);
    });
    let result = Z3_CTX.with(|ctx| {
        prove_tautology_with_axioms_and_timeout_in(ctx, expr, bindings, axioms, timeout_ms)
    });
    // RES-1657: persist when the tautology was proven.
    if result.0 {
        persistent_proven_insert(key);
    }
    Z3_TAUTOLOGY_CACHE.with(|c| {
        c.borrow_mut().insert(key, result.clone());
    });
    result
}

fn prove_tautology_with_axioms_and_timeout_in(
    ctx: &z3::Context,
    expr: &Node,
    bindings: &HashMap<String, i64>,
    axioms: &[Node],
    timeout_ms: u32,
) -> (bool, Option<ProofCertificate>, bool) {
    let formula = match translate_bool(ctx, expr, bindings) {
        Some(f) => f,
        None => return (false, None, false),
    };

    // RES-1655: hoist the timeout Params out of the closure so it
    // builds once per prove instead of once per solver.
    let timeout_params = if timeout_ms > 0 {
        let mut p = z3::Params::new(ctx);
        p.set_u32("timeout", timeout_ms);
        Some(p)
    } else {
        None
    };
    let apply_timeout = |solver: &z3::Solver<'_>| {
        if let Some(ref p) = timeout_params {
            solver.set_params(p);
        }
    };

    // Identical len-axiom and user-axiom setup as the full prover;
    // only the second `check()` (contradiction) is omitted.
    // RES-1528: borrow each `len(<arg>)` ident name as `&str` from
    // the formula AST instead of cloning into a `BTreeSet<String>`.
    // The set is iterated to (a) build the `len_axioms` Z3 vec and
    // (b) emit `(declare-const len_X Int)` / `(assert (>= len_X 0))`
    // lines into the SMT cert. Both consumers only read the name —
    // the owned `String` keys were pure overhead, paid on *every*
    // Z3 prove call across the entire typecheck. Mirror of RES-1427
    // for the tuple element.
    let mut len_args: BTreeSet<&str> = BTreeSet::new();
    collect_len_args(expr, &mut len_args);
    // RES-1651: lift the zero constant out of the per-arg map closure
    // (same shape as the verdict path above).
    let zero = Int::from_i64(ctx, 0);
    let len_axioms: Vec<Bool<'_>> = len_args
        .iter()
        .map(|arg| {
            let c = Int::new_const(ctx, format!("len_{}", arg));
            c.ge(&zero)
        })
        .collect();

    let user_axioms: Vec<Bool<'_>> = axioms
        .iter()
        .filter_map(|ax| translate_bool(ctx, ax, bindings))
        .collect();

    let solver = z3::Solver::new(ctx);
    apply_timeout(&solver);
    for axiom in &len_axioms {
        solver.assert(axiom);
    }
    for axiom in &user_axioms {
        solver.assert(axiom);
    }
    let negated = formula.not();
    solver.assert(&negated);
    let check = solver.check();
    let tautology = matches!(check, z3::SatResult::Unsat);
    let timed_out = matches!(check, z3::SatResult::Unknown);

    if !tautology {
        return (false, None, timed_out);
    }

    // Tautology proven — emit the same SMT-LIB2 certificate shape
    // `prove_with_axioms_and_timeout` would have.
    let mut idents: BTreeSet<&str> = BTreeSet::new();
    collect_int_identifiers(expr, &mut idents);
    let mut arr_args: BTreeSet<&str> = BTreeSet::new();
    collect_array_args(expr, &mut arr_args);

    // RES-1383: same `writeln!`-into-buffer fix as the LIA verifier's
    // cert builder above — eliminates the intermediate `format!`
    // String allocations per declaration / axiom / assertion.
    // RES-1653: pre-size to 512 bytes (same shape as the LIA path).
    let mut smt2 = String::with_capacity(512);
    smt2.push_str("; RES-071 verification certificate\n");
    smt2.push_str("; expected solver result: unsat (proves the contract is a tautology)\n");
    smt2.push_str("(set-logic AUFLIA)\n");
    for name in &idents {
        writeln!(&mut smt2, "(declare-const {} Int)", name).unwrap();
    }
    for arg in &len_args {
        writeln!(&mut smt2, "(declare-const len_{} Int)", arg).unwrap();
    }
    for arg in &arr_args {
        writeln!(&mut smt2, "(declare-const arr_{} (Array Int Int))", arg).unwrap();
    }
    for arg in &len_args {
        writeln!(&mut smt2, "(assert (>= len_{} 0))", arg).unwrap();
    }
    for name in &idents {
        if let Some(v) = bindings.get(*name) {
            writeln!(&mut smt2, "(assert (= {} {}))", name, v).unwrap();
        }
    }
    writeln!(&mut smt2, "(assert {})", negated).unwrap();
    smt2.push_str("(check-sat)\n");

    (true, Some(ProofCertificate { smt2 }), false)
}

// ============================================================
// RES-354: BV32 theory prover
// ============================================================

/// Prove `expr` under `bindings` using the BV32 theory. All integer
/// constants are modelled as `BV<32>`; all free identifiers become
/// `BV<32>` constants; arithmetic and bitwise operations use BV ops.
/// Comparisons use signed BV (`bvsgt`, `bvslt`, etc.).
///
/// Returns the same four-slot tuple as `prove_with_axioms_and_timeout`.
/// Certificate generation is not yet supported for the BV path (the
/// SMT-LIB2 certificate infrastructure is LIA-only); `ProofCertificate`
/// is always `None` on this path.
pub fn prove_bv(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    timeout_ms: u32,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>, bool) {
    // RES-1663 / RES-1665 / RES-1667: pre-Z3 constant fold. BV32
    // shares the same trivial-evaluation rules — a literal-bool or
    // literal-comparison is independent of the underlying theory.
    if let Some(verdict) = try_const_eval_bool(expr) {
        return (Some(verdict), None, None, false);
    }
    // RES-1316: thread-local verdict cache for the BV32 entry point.
    // RES-1206 caches the LIA path; RES-1309 the tautology fast path.
    // This is the third uncached Z3 entry — routed to from
    // `prove_auto` (and thus `z3_prove_with_cert_theory`) whenever
    // the contract clause has bitwise operations. Same key shape as
    // the other caches; suffix `|BV` keeps the namespace disjoint
    // from `|TAUT` and the unsuffixed LIA cache.
    let key = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        hash_node_spanless(expr, &mut h);
        b'|'.hash(&mut h);
        let bindings_sorted: std::collections::BTreeMap<&String, &i64> = bindings.iter().collect();
        for (k, v) in &bindings_sorted {
            k.hash(&mut h);
            v.hash(&mut h);
        }
        b'|'.hash(&mut h);
        timeout_ms.hash(&mut h);
        b"BV".hash(&mut h);
        h.finish()
    };
    // RES-1657: persistent proven-set short-circuit (BV path).
    if persistent_proven_contains(key) {
        Z3_CACHE_STATS.with(|s| {
            let mut v = s.get();
            v.bv_hits += 1;
            s.set(v);
        });
        return (Some(true), None, None, false);
    }
    if let Some(cached) = Z3_BV_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        Z3_CACHE_STATS.with(|s| {
            let mut v = s.get();
            v.bv_hits += 1;
            s.set(v);
        });
        return cached;
    }
    Z3_CACHE_STATS.with(|s| {
        let mut v = s.get();
        v.bv_misses += 1;
        s.set(v);
    });
    let result = Z3_CTX.with(|ctx| prove_bv_in(ctx, expr, bindings, timeout_ms));
    // RES-1657: persist when the BV verdict was Some(true).
    if matches!(result.0, Some(true)) {
        persistent_proven_insert(key);
    }
    Z3_BV_CACHE.with(|c| {
        c.borrow_mut().insert(key, result.clone());
    });
    result
}

fn prove_bv_in(
    ctx: &z3::Context,
    expr: &Node,
    bindings: &HashMap<String, i64>,
    timeout_ms: u32,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>, bool) {
    let formula = match translate_bool_bv(ctx, expr, bindings) {
        Some(f) => f,
        None => return (None, None, None, false),
    };

    // RES-1655: hoist the timeout Params out of the closure (BV path).
    let timeout_params = if timeout_ms > 0 {
        let mut p = z3::Params::new(ctx);
        p.set_u32("timeout", timeout_ms);
        Some(p)
    } else {
        None
    };
    let apply_timeout = |solver: &z3::Solver<'_>| {
        if let Some(ref p) = timeout_params {
            solver.set_params(p);
        }
    };

    // RES-1702: share one Solver across the tautology + contradiction
    // checks via push/pop scopes. Same shape as RES-1696 for the LIA
    // path. Saves one `Solver::new()` allocation + one `apply_timeout`
    // setup per BV prove call; Z3's incremental solver can also reuse
    // learned facts across the push/pop boundary.
    let solver = z3::Solver::new(ctx);
    apply_timeout(&solver);

    // Tautology check.
    let negated = formula.not();
    solver.push();
    solver.assert(&negated);
    let check = solver.check();
    let tautology = matches!(check, z3::SatResult::Unsat);
    let timed_out = matches!(check, z3::SatResult::Unknown);

    if tautology {
        // Don't bother popping — function returns and the solver is
        // dropped anyway.
        return (Some(true), None, None, false);
    }

    let counterexample = if matches!(check, z3::SatResult::Sat) {
        // Extract counterexample BEFORE pop — model is scope-bound.
        extract_counterexample_bv(ctx, &solver, expr, bindings)
    } else {
        None
    };
    solver.pop(1);

    // Contradiction check.
    solver.push();
    solver.assert(&formula);
    let contradiction = matches!(solver.check(), z3::SatResult::Unsat);
    solver.pop(1);

    if contradiction {
        return (Some(false), None, counterexample, false);
    }

    (None, None, counterexample, timed_out)
}

/// Auto-detect the theory for `expr` based on `theory` hint and
/// the presence of bitwise operations. Returns the result of
/// whichever theory path is selected.
///
/// - `Z3Theory::Auto`: use BV32 if `has_bitwise_ops(expr)`, else LIA.
/// - `Z3Theory::Bv`: always BV32.
/// - `Z3Theory::Lia`: always LIA; if bitwise ops are present, returns
///   `(None, None, None, false)` — the caller's runtime check fires.
pub fn prove_auto(
    expr: &Node,
    bindings: &HashMap<String, i64>,
    theory: Z3Theory,
    timeout_ms: u32,
) -> (Option<bool>, Option<ProofCertificate>, Option<String>, bool) {
    // RES-1682: constant-fold short-circuit. The downstream entry
    // points (`prove_bv`, `prove_with_axioms_and_timeout`) already
    // fast-path foldable obligations, but `prove_auto` still pays
    // for the `has_bitwise_ops` walk and the function dispatch. The
    // fold is theory-independent — a bool literal or literal
    // comparison evaluates to the same verdict regardless of LIA /
    // BV32 encoding — so we can short-circuit before deciding which
    // theory to use.
    if let Some(verdict) = try_const_eval_bool(expr) {
        return (Some(verdict), None, None, false);
    }
    let use_bv = match theory {
        Z3Theory::Bv => true,
        Z3Theory::Lia => {
            if has_bitwise_ops(expr) {
                // Caller asked for LIA but formula has bitwise ops —
                // cannot encode; bail to None so the runtime check fires.
                return (None, None, None, false);
            }
            false
        }
        Z3Theory::Auto => has_bitwise_ops(expr),
    };
    if use_bv {
        prove_bv(expr, bindings, timeout_ms)
    } else {
        prove_with_axioms_and_timeout(expr, bindings, &[], timeout_ms)
    }
}

/// RES-393 D1: attempt to prove that two function parameters cannot alias
/// given the function's `requires` preconditions.
///
/// Models each parameter as a free Z3 integer (standing for its region ID
/// or pointer value). Returns `Some(true)` if the `requires` clauses imply
/// `param_a != param_b`; `None` if Z3 cannot decide or `requires` is empty.
#[allow(dead_code)]
pub fn prove_alias_disjoint(param_a: &str, param_b: &str, requires: &[Node]) -> Option<bool> {
    if requires.is_empty() {
        return None;
    }
    let neq = Node::InfixExpression {
        left: Box::new(Node::Identifier {
            name: param_a.to_string(),
            span: crate::span::Span::default(),
        }),
        operator: "!=".to_string(),
        right: Box::new(Node::Identifier {
            name: param_b.to_string(),
            span: crate::span::Span::default(),
        }),
        span: crate::span::Span::default(),
    };
    // RES-1194: only `Some(true)` is meaningful here — the caller in
    // `lib.rs` checks `== Some(true)` and treats every other verdict
    // as "couldn't prove disjoint". Use the tautology-only fast path
    // to skip the contradiction-phase `solver.check()`.
    let (proven, _, _) =
        prove_tautology_with_axioms_and_timeout(&neq, &HashMap::new(), requires, 500);
    if proven { Some(true) } else { None }
}

/// Collect every identifier name seen in `node` (for BV counterexample
/// extraction — mirrors `collect_int_identifiers` but used for BV vars).
///
/// RES-1532: borrows from the formula AST instead of cloning into the
/// set — the consumer (`extract_counterexample_bv`) only reads names
/// via `format!` / `bindings.contains_key` / `BV::new_const`, all of
/// which accept `&str`. Same shape as RES-1528 applied to `len_args`
/// / `arr_args`.
fn collect_bv_identifiers<'a>(node: &'a Node, out: &mut BTreeSet<&'a str>) {
    match node {
        Node::Identifier { name, .. } => {
            out.insert(name.as_str());
        }
        Node::PrefixExpression { right, .. } => collect_bv_identifiers(right, out),
        Node::InfixExpression { left, right, .. } => {
            collect_bv_identifiers(left, out);
            collect_bv_identifiers(right, out);
        }
        _ => {}
    }
}

/// Extract a counterexample from a BV solver model: format as
/// `name = value, ...` where each value is the BV constant evaluated
/// as a signed 32-bit integer.
fn extract_counterexample_bv(
    ctx: &z3::Context,
    solver: &z3::Solver<'_>,
    expr: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<String> {
    let model = solver.get_model()?;
    let mut idents: BTreeSet<&str> = BTreeSet::new();
    collect_bv_identifiers(expr, &mut idents);

    let mut parts: Vec<String> = Vec::new();
    for name in &idents {
        if bindings.contains_key(*name) {
            continue;
        }
        let var = BV::new_const(ctx, *name, 32);
        if let Some(v) = model.eval(&var, false) {
            // BV::as_i64() gives the unsigned bit pattern; sign-extend
            // for display by treating values > i32::MAX as negative.
            if let Some(n) = v.as_i64() {
                // Mask to 32 bits then sign-extend.
                let bits = n as u32;
                let signed = bits as i32;
                parts.push(format!("{} = {}", name, signed));
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// Translate an AST expression to a Z3 `Bool` under the BV32 theory.
fn translate_bool_bv<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<Bool<'c>> {
    match node {
        Node::BooleanLiteral { value: b, .. } => Some(Bool::from_bool(ctx, *b)),
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "!" => translate_bool_bv(ctx, right, bindings).map(|b| b.not()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => match operator.as_str() {
            "&&" => {
                let l = translate_bool_bv(ctx, left, bindings)?;
                let r = translate_bool_bv(ctx, right, bindings)?;
                Some(Bool::and(ctx, &[&l, &r]))
            }
            "||" => {
                let l = translate_bool_bv(ctx, left, bindings)?;
                let r = translate_bool_bv(ctx, right, bindings)?;
                Some(Bool::or(ctx, &[&l, &r]))
            }
            "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                let l = translate_bv(ctx, left, bindings)?;
                let r = translate_bv(ctx, right, bindings)?;
                let cmp = match operator.as_str() {
                    "==" => l._eq(&r),
                    "!=" => l._eq(&r).not(),
                    "<" => l.bvslt(&r),
                    ">" => l.bvsgt(&r),
                    "<=" => l.bvsle(&r),
                    ">=" => l.bvsge(&r),
                    _ => unreachable!(),
                };
                Some(cmp)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Translate an AST integer expression to a Z3 BV<32> value.
fn translate_bv<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<BV<'c>> {
    match node {
        Node::IntegerLiteral { value: v, .. } => Some(BV::from_i64(ctx, *v, 32)),
        Node::Identifier { name, .. } => match bindings.get(name) {
            Some(v) => Some(BV::from_i64(ctx, *v, 32)),
            None => Some(BV::new_const(ctx, name.as_str(), 32)),
        },
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => translate_bv(ctx, right, bindings).map(|v| v.bvneg()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let l = translate_bv(ctx, left, bindings)?;
            let r = translate_bv(ctx, right, bindings)?;
            Some(match operator.as_str() {
                "+" => l.bvadd(&r),
                "-" => l.bvsub(&r),
                "*" => l.bvmul(&r),
                "/" => l.bvsdiv(&r),
                "%" => l.bvsrem(&r),
                "&" => l.bvand(&r),
                "|" => l.bvor(&r),
                "^" => l.bvxor(&r),
                "<<" => l.bvshl(&r),
                ">>" => l.bvashr(&r),
                _ => return None,
            })
        }
        _ => None,
    }
}

/// RES-136: harvest identifier assignments from a satisfied Z3
/// solver and format them as `name = value, name = value`. Only
/// integer identifiers the translator could produce are consulted.
///
/// We evaluate each identifier as an `Int` (the translator models
/// every free variable as `Int::new_const(name)`), request
/// model_completion=false so Z3 can legitimately return "not
/// constrained" — variables it didn't assign are silently dropped
/// per the ticket. Constants already pinned via `bindings` are also
/// dropped: echoing back input isn't useful diagnostic output.
fn extract_counterexample(
    ctx: &z3::Context,
    solver: &z3::Solver<'_>,
    expr: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<String> {
    let model = solver.get_model()?;
    let mut idents: BTreeSet<&str> = BTreeSet::new();
    collect_int_identifiers(expr, &mut idents);

    let mut parts: Vec<String> = Vec::new();
    for name in &idents {
        if bindings.contains_key(*name) {
            continue;
        }
        let var = Int::new_const(ctx, *name);
        if let Some(v) = model.eval(&var, false)
            && let Some(n) = v.as_i64()
        {
            parts.push(format!("{} = {}", name, n));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// Walk the AST collecting every identifier that the integer or boolean
/// translator could plausibly emit a `(declare-const NAME Int)` for.
/// Conservative — over-collecting is fine (extra unused declarations
/// don't change satisfiability); under-collecting would make the
/// certificate reference an undefined symbol and stock Z3 would error.
fn collect_int_identifiers<'a>(node: &'a Node, out: &mut BTreeSet<&'a str>) {
    match node {
        Node::Identifier { name, .. } => {
            out.insert(name.as_str());
        }
        Node::PrefixExpression { right, .. } => collect_int_identifiers(right, out),
        Node::InfixExpression { left, right, .. } => {
            collect_int_identifiers(left, out);
            collect_int_identifiers(right, out);
        }
        // RES-408: walk into quantifier bodies so the cert declares
        // free Int identifiers referenced inside `forall i ...: P(i, x)`
        // (where `x` is free). The bound variable `var` is removed
        // afterwards because it's quantified inline by the negated
        // formula's `(forall ((i Int)) ...)` block — declaring it at
        // top level would shadow the bound binding.
        Node::Quantifier {
            var, range, body, ..
        } => {
            if let crate::quantifiers::QuantRange::Range { lo, hi } = range {
                collect_int_identifiers(lo, out);
                collect_int_identifiers(hi, out);
            }
            collect_int_identifiers(body, out);
            out.remove(var.as_str());
        }
        // RES-408: walk into the index of `a[i]` (the array name lives
        // separately in the `arr_<name>` collector — see
        // `collect_array_args`); descending into `target` would
        // mistakenly add the array's name to the Int idents.
        Node::IndexExpression { index, .. } => {
            collect_int_identifiers(index, out);
        }
        // Literals contribute no identifiers; everything else
        // (calls, blocks, etc.) is outside the supported subset and
        // would have caused translate_*() to bail already.
        _ => {}
    }
}

/// RES-408: collect every array identifier referenced via
/// `IndexExpression { target: Identifier(name), .. }` so the
/// certificate generator can emit
/// `(declare-const arr_<name> (Array Int Int))`. Mirrors the shape
/// of `collect_len_args`.
fn collect_array_args<'a>(node: &'a Node, out: &mut BTreeSet<&'a str>) {
    match node {
        Node::IndexExpression { target, index, .. } => {
            if let Node::Identifier { name, .. } = target.as_ref() {
                out.insert(name.as_str());
            }
            collect_array_args(index, out);
        }
        Node::PrefixExpression { right, .. } => collect_array_args(right, out),
        Node::InfixExpression { left, right, .. } => {
            collect_array_args(left, out);
            collect_array_args(right, out);
        }
        Node::Quantifier { range, body, .. } => {
            if let crate::quantifiers::QuantRange::Range { lo, hi } = range {
                collect_array_args(lo, out);
                collect_array_args(hi, out);
            }
            collect_array_args(body, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_array_args(function, out);
            for arg in arguments {
                collect_array_args(arg, out);
            }
        }
        _ => {}
    }
}

/// RES-330: thin pub(crate) wrapper around `translate_int` so the
/// `quantifiers` module (which lives outside this file) can encode
/// the bounds of a `lo..hi` range into the Z3 LIA fragment.
pub(crate) fn translate_int_pub<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<Int<'c>> {
    translate_int(ctx, node, bindings)
}

/// RES-330: thin pub(crate) wrapper around `translate_bool` so the
/// `quantifiers` module can encode quantifier bodies.
pub(crate) fn translate_bool_pub<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<Bool<'c>> {
    translate_bool(ctx, node, bindings)
}

fn translate_bool<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<Bool<'c>> {
    match node {
        Node::BooleanLiteral { value: b, .. } => Some(Bool::from_bool(ctx, *b)),
        // RES-330: dispatch quantifier nodes into the dedicated encoder.
        // Iterable quantifiers return None and the caller falls back to
        // the runtime check.
        Node::Quantifier {
            kind,
            var,
            range,
            body,
            ..
        } => crate::quantifiers::z3_encode(ctx, *kind, var, range, body, bindings),
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "!" => translate_bool(ctx, right, bindings).map(|b| b.not()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => match operator.as_str() {
            "&&" => {
                let l = translate_bool(ctx, left, bindings)?;
                let r = translate_bool(ctx, right, bindings)?;
                Some(Bool::and(ctx, &[&l, &r]))
            }
            "||" => {
                let l = translate_bool(ctx, left, bindings)?;
                let r = translate_bool(ctx, right, bindings)?;
                Some(Bool::or(ctx, &[&l, &r]))
            }
            "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                let l = translate_int(ctx, left, bindings)?;
                let r = translate_int(ctx, right, bindings)?;
                let cmp = match operator.as_str() {
                    "==" => l._eq(&r),
                    "!=" => l._eq(&r).not(),
                    "<" => l.lt(&r),
                    ">" => l.gt(&r),
                    "<=" => l.le(&r),
                    ">=" => l.ge(&r),
                    _ => unreachable!(),
                };
                Some(cmp)
            }
            _ => None,
        },
        _ => None,
    }
}

fn translate_int<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    bindings: &HashMap<String, i64>,
) -> Option<Int<'c>> {
    match node {
        Node::IntegerLiteral { value: v, .. } => Some(Int::from_i64(ctx, *v)),
        Node::Identifier { name, .. } => match bindings.get(name) {
            // If the name is bound to a known constant, model it as a
            // constant. Otherwise model it as a fresh free integer
            // variable so Z3 can reason about it universally.
            Some(v) => Some(Int::from_i64(ctx, *v)),
            None => Some(Int::new_const(ctx, name.as_str())),
        },
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => translate_int(ctx, right, bindings).map(|v| v.unary_minus()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let l = translate_int(ctx, left, bindings)?;
            let r = translate_int(ctx, right, bindings)?;
            Some(match operator.as_str() {
                "+" => Int::add(ctx, &[&l, &r]),
                "-" => Int::sub(ctx, &[&l, &r]),
                "*" => Int::mul(ctx, &[&l, &r]),
                "/" => l.div(&r),
                "%" => l.rem(&r),
                _ => return None,
            })
        }
        // RES-131 (RES-131a): `len(<ident>)` as an uninterpreted
        // Int constant, named `len_<ident>`. Every reference to
        // `len` on the same array identifier maps to the same Int
        // constant (same name → same Z3 const by convention),
        // giving the solver enough structure to prove
        // `len(xs) > 0 → len(xs) >= 1`. The `>= 0` axiom is
        // injected by `collect_len_args` + the `prove_with_timeout`
        // caller, not here; this fn stays side-effect-free on the
        // solver.
        Node::CallExpression {
            function,
            arguments,
            ..
        } if is_len_call(function, arguments) => {
            if let Node::Identifier { name, .. } = &arguments[0] {
                Some(Int::new_const(ctx, format!("len_{}", name)))
            } else {
                // `len(<non-identifier>)` isn't supported —
                // bail to None so the caller's existing
                // fallback logic fires.
                None
            }
        }
        // RES-408: `a[i]` lowers to Z3 array theory. The array is
        // modelled as `(Array Int Int)` named `arr_<name>`; the
        // index translates through the existing Int path and
        // `select` returns an Int. This is what unblocks
        // `forall i in 0..len(a): P(a[i])` proofs — without it the
        // body translates to None and Z3 falls back to runtime.
        Node::IndexExpression { target, index, .. } => {
            if let Node::Identifier { name, .. } = target.as_ref() {
                let idx = translate_int(ctx, index, bindings)?;
                let arr = Array::new_const(
                    ctx,
                    format!("arr_{}", name),
                    &Sort::int(ctx),
                    &Sort::int(ctx),
                );
                arr.select(&idx).as_int()
            } else {
                None
            }
        }
        _ => None,
    }
}

/// RES-131 (RES-131a): syntactic check for `len(<anything>)` —
/// exactly one arg, callee is a bare `Identifier("len")`. Method
/// calls / shadowed `len` don't qualify (we only recognize the
/// top-level builtin).
fn is_len_call(function: &Node, arguments: &[Node]) -> bool {
    if arguments.len() != 1 {
        return false;
    }
    matches!(function, Node::Identifier { name, .. } if name == "len")
}

/// RES-131: collect every array identifier that appears inside
/// a `len(<id>)` call within `node`. Returns the ARG names
/// (not the synthesized `len_<arg>` z3 names) so callers can
/// format the axiom / certificate consistently.
fn collect_len_args<'a>(node: &'a Node, out: &mut BTreeSet<&'a str>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } if is_len_call(function, arguments) => {
            if let Node::Identifier { name, .. } = &arguments[0] {
                out.insert(name.as_str());
            }
        }
        Node::PrefixExpression { right, .. } => {
            collect_len_args(right, out);
        }
        Node::InfixExpression { left, right, .. } => {
            collect_len_args(left, out);
            collect_len_args(right, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_len_args(function, out);
            for arg in arguments {
                collect_len_args(arg, out);
            }
        }
        // RES-408: walk into quantifier bodies so `forall i in 0..len(a)`
        // properly registers `a` for the `len_a >= 0` axiom.
        Node::Quantifier { range, body, .. } => {
            if let crate::quantifiers::QuantRange::Range { lo, hi } = range {
                collect_len_args(lo, out);
                collect_len_args(hi, out);
            }
            collect_len_args(body, out);
        }
        // RES-408: walk into `a[i]` so a `len()` call hidden inside
        // an index expression is still picked up.
        Node::IndexExpression { target, index, .. } => {
            collect_len_args(target, out);
            collect_len_args(index, out);
        }
        _ => {}
    }
}

// ============================================================
// RES-386: actor commutativity check
// ============================================================
//
// The minimum slice models an actor's per-handler state transition
// as a pure function `f: Int -> Int` over a single integer-valued
// `self.state`. For every pair of handlers `(A, B)` we ask the
// solver whether running A-then-B from any symbolic pre-state
// produces the same final state as B-then-A.
//
// This captures the "no lost updates" invariant the ticket body
// motivates — if `Counter::increment` and `Counter::decrement`
// commute, concurrent dispatchers can interleave them without
// locks and still arrive at the same final count.
//
// Verdict shape:
//   - Commute(name)                    — provable, no counterexample.
//   - Diverge { a, b, pre, ab, ba, .. } — Z3 exhibited a model that
//                                        falsifies the commutativity
//                                        formula.
//   - Unknown(name)                    — handler body isn't the
//                                        supported `self.state = <int>;`
//                                        shape (e.g. a branch, a call,
//                                        or an assignment to a
//                                        non-state field) or Z3
//                                        returned Unknown.
//
// Anything beyond the supported body shape is reported as Unknown
// so the driver can surface a clear diagnostic; it is never
// silently treated as proof.

/// RES-386: outcome of a commutativity check for one pair of actor
/// handlers. The driver formats this into a user-facing diagnostic;
/// tests consume the structured form directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommutativityResult {
    /// Z3 proved `A(B(s)) == B(A(s))` for all integer pre-states.
    Commute,
    /// Z3 produced a concrete counterexample.
    Diverge {
        pre_state: String,
        ab_state: String,
        ba_state: String,
    },
    /// Handler body wasn't in the supported shape, or the solver
    /// could not decide (typically a hard non-linear arithmetic
    /// query). The driver emits a `warning`-flavoured diagnostic
    /// rather than a hard error.
    Unknown { reason: String },
}

/// RES-386: per-actor verification outcome, aggregating one
/// `CommutativityResult` per ordered handler pair.
#[derive(Debug, Clone)]
pub struct ActorVerification {
    pub actor_name: String,
    /// Ordered `(handler_a, handler_b, result)` triples. The
    /// verifier only emits each unordered pair once (a < b by
    /// source order) — commutativity is symmetric.
    pub pairs: Vec<(String, String, CommutativityResult)>,
}

/// RES-386: drive the commutativity check for every pair of
/// `receive` handlers in the given actor. See module docs above
/// for the semantic contract.
pub fn check_actor_commutativity(actor_name: &str, handlers: &[ActorHandler]) -> ActorVerification {
    let mut pairs: Vec<(String, String, CommutativityResult)> = Vec::new();
    for i in 0..handlers.len() {
        for j in (i + 1)..handlers.len() {
            let a = &handlers[i];
            let b = &handlers[j];
            let result = check_pair_commute(a, b);
            pairs.push((a.name.clone(), b.name.clone(), result));
        }
    }
    ActorVerification {
        actor_name: actor_name.to_string(),
        pairs,
    }
}

/// Check that running `a` then `b` produces the same final state
/// as running `b` then `a`, starting from an arbitrary symbolic
/// integer pre-state.
fn check_pair_commute(a: &ActorHandler, b: &ActorHandler) -> CommutativityResult {
    // Extract each handler's symbolic RHS expression (the expression
    // that computes the new `self.state` from the old one). Anything
    // outside the supported `self.state = <int_expr>;` shape fails
    // fast with a descriptive `Unknown`.
    let a_rhs = match extract_state_rhs(&a.body) {
        Ok(n) => n,
        Err(why) => {
            return CommutativityResult::Unknown {
                reason: format!("handler `{}`: {}", a.name, why),
            };
        }
    };
    let b_rhs = match extract_state_rhs(&b.body) {
        Ok(n) => n,
        Err(why) => {
            return CommutativityResult::Unknown {
                reason: format!("handler `{}`: {}", b.name, why),
            };
        }
    };

    Z3_CTX.with(|ctx| check_pair_commute_in(ctx, &a_rhs, &b_rhs, &a.name, &b.name))
}

fn check_pair_commute_in(
    ctx: &z3::Context,
    a_rhs: &Node,
    b_rhs: &Node,
    a_name: &str,
    b_name: &str,
) -> CommutativityResult {
    // Name conventions for the counterexample formatter:
    //   state_0         — the symbolic pre-state.
    //   state_after_<h> — abbreviations recovered from the model.
    let pre = Int::new_const(ctx, "state_0");

    // Build the A-then-B chain.
    let Some(ab_inter) = translate_state_rhs(ctx, a_rhs, &pre) else {
        return CommutativityResult::Unknown {
            reason: format!(
                "handler `{}`: RHS contains unsupported operations (verifier only models +, -, *, /, %, and integer literals)",
                a_name,
            ),
        };
    };
    let Some(ab_final) = translate_state_rhs(ctx, b_rhs, &ab_inter) else {
        return CommutativityResult::Unknown {
            reason: format!(
                "handler `{}`: RHS contains unsupported operations (verifier only models +, -, *, /, %, and integer literals)",
                b_name,
            ),
        };
    };

    // Build the B-then-A chain.
    let Some(ba_inter) = translate_state_rhs(ctx, b_rhs, &pre) else {
        return CommutativityResult::Unknown {
            reason: format!(
                "handler `{}`: RHS contains unsupported operations (verifier only models +, -, *, /, %, and integer literals)",
                b_name,
            ),
        };
    };
    let Some(ba_final) = translate_state_rhs(ctx, a_rhs, &ba_inter) else {
        return CommutativityResult::Unknown {
            reason: format!(
                "handler `{}`: RHS contains unsupported operations (verifier only models +, -, *, /, %, and integer literals)",
                a_name,
            ),
        };
    };

    // Tautology question: is (ab_final != ba_final) UNSAT?
    // If UNSAT → commute. If SAT → counterexample. If Unknown → Unknown.
    let goal = ab_final._eq(&ba_final);
    let negated = goal.not();
    let solver = z3::Solver::new(ctx);
    solver.assert(&negated);
    match solver.check() {
        z3::SatResult::Unsat => CommutativityResult::Commute,
        z3::SatResult::Sat => match solver.get_model() {
            Some(model) => {
                let pre_val = model
                    .eval(&pre, true)
                    .and_then(|v| v.as_i64())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "<?>".to_string());
                let ab_val = model
                    .eval(&ab_final, true)
                    .and_then(|v| v.as_i64())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "<?>".to_string());
                let ba_val = model
                    .eval(&ba_final, true)
                    .and_then(|v| v.as_i64())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "<?>".to_string());
                CommutativityResult::Diverge {
                    pre_state: pre_val,
                    ab_state: ab_val,
                    ba_state: ba_val,
                }
            }
            None => CommutativityResult::Unknown {
                reason: "Z3 reported Sat but provided no model".to_string(),
            },
        },
        z3::SatResult::Unknown => CommutativityResult::Unknown {
            reason:
                "Z3 returned Unknown — the commutativity formula is outside the decided fragment"
                    .to_string(),
        },
    }
}

/// Peel a handler body down to its single `self.state = <rhs>;`
/// assignment. Returns the RHS expression on success. Any other
/// body shape is rejected with a human-readable reason — the
/// minimum slice deliberately narrows the accepted form rather
/// than silently proving trivial-seeming commutativity on
/// unrepresented control flow.
fn extract_state_rhs(body: &Node) -> Result<Node, String> {
    let stmts: &[Node] = match body {
        Node::Block { stmts, .. } => stmts,
        _ => {
            return Err(
                "body must be a block containing exactly `self.state = <int_expr>;`".to_string(),
            );
        }
    };
    if stmts.len() != 1 {
        return Err(format!(
            "body must contain exactly one statement (`self.state = ...`), found {}",
            stmts.len()
        ));
    }
    let expr_stmt = match &stmts[0] {
        Node::ExpressionStatement { expr, .. } => expr.as_ref(),
        other => other,
    };
    match expr_stmt {
        Node::FieldAssignment {
            target,
            field,
            value,
            ..
        } => {
            let Node::Identifier { name, .. } = target.as_ref() else {
                return Err("assignment target must be `self.state` (minimum slice)".to_string());
            };
            if name != "self" || field != "state" {
                return Err(format!(
                    "assignment target must be `self.state`, got `{}.{}`",
                    name, field
                ));
            }
            Ok((**value).clone())
        }
        _ => Err("body statement must be `self.state = <int_expr>;`".to_string()),
    }
}

/// Translate a handler's RHS expression into a Z3 `Int`, with any
/// `self.state` field access bound to `pre_state`. Supports the
/// same integer subset as `translate_int`.
fn translate_state_rhs<'c>(
    ctx: &'c z3::Context,
    node: &Node,
    pre_state: &Int<'c>,
) -> Option<Int<'c>> {
    match node {
        Node::IntegerLiteral { value, .. } => Some(Int::from_i64(ctx, *value)),
        Node::FieldAccess { target, field, .. } => {
            if let Node::Identifier { name, .. } = target.as_ref()
                && name == "self"
                && field == "state"
            {
                Some(pre_state.clone())
            } else {
                None
            }
        }
        // A bare `self` with no field access is nonsensical here.
        Node::Identifier { .. } => None,
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => translate_state_rhs(ctx, right, pre_state).map(|v| v.unary_minus()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let l = translate_state_rhs(ctx, left, pre_state)?;
            let r = translate_state_rhs(ctx, right, pre_state)?;
            Some(match operator.as_str() {
                "+" => Int::add(ctx, &[&l, &r]),
                "-" => Int::sub(ctx, &[&l, &r]),
                "*" => Int::mul(ctx, &[&l, &r]),
                "/" => l.div(&r),
                "%" => l.rem(&r),
                _ => return None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RES-1188: prove the same expression repeatedly in a single
    /// thread — the thread-local `Z3_CTX` is reused across all
    /// calls, and the result must remain consistent. A regression
    /// where solver state leaks between calls would show up as
    /// `Some(true)` flipping to `None` (or worse, `Some(false)`)
    /// on the second invocation.
    #[test]
    fn repeated_prove_in_one_thread_is_consistent() {
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral {
                value: 7,
                span: crate::span::Span::default(),
            }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 3,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        let first = prove(&expr, &no_b);
        let second = prove(&expr, &no_b);
        let third = prove(&expr, &no_b);
        assert_eq!(first, Some(true));
        assert_eq!(second, Some(true));
        assert_eq!(third, Some(true));
    }

    /// RES-1194: the tautology-only entry point agrees with the
    /// full prover on tautological inputs and reports the same
    /// `proven=true` verdict alongside an SMT-LIB2 certificate.
    #[test]
    fn tautology_only_path_agrees_with_full_prover_on_tautology() {
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral {
                value: 5,
                span: crate::span::Span::default(),
            }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 3,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        let (full_verdict, full_cert, _, _) = prove_with_axioms_and_timeout(&expr, &no_b, &[], 0);
        let (taut_proven, taut_cert, taut_timed_out) =
            prove_tautology_with_axioms_and_timeout(&expr, &no_b, &[], 0);
        assert_eq!(full_verdict, Some(true));
        assert!(taut_proven);
        assert!(!taut_timed_out);
        // Both paths emit a certificate on tautology success — they
        // build it from the same shape (sorted idents, len-arg
        // axioms, negated goal), so the bytes must match.
        assert_eq!(
            full_cert.as_ref().map(|c| &c.smt2),
            taut_cert.as_ref().map(|c| &c.smt2)
        );
    }

    /// RES-1194: when the goal is a contradiction (provably always
    /// false), the full prover returns `Some(false)` but the
    /// tautology-only path returns `proven=false` (with no cert).
    /// Callers that route through the new fast path treat this
    /// identically to "uncertain" — keep the runtime check.
    #[test]
    fn tautology_only_path_returns_false_on_contradiction() {
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        let (proven, cert, timed_out) =
            prove_tautology_with_axioms_and_timeout(&expr, &no_b, &[], 0);
        assert!(!proven);
        assert!(cert.is_none());
        assert!(!timed_out);
    }

    /// RES-1188: a tautology call followed by a contradiction call on
    /// the same shared context must each return their own verdict.
    /// The previous (per-call context) implementation had no shared
    /// state to leak; this guards the new shared-context layout
    /// against accidentally carrying assertion state across calls.
    #[test]
    fn alternating_tautology_and_contradiction_in_one_thread() {
        let no_b = HashMap::new();
        let taut = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral {
                value: 5,
                span: crate::span::Span::default(),
            }),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        let contra = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&taut, &no_b), Some(true));
        assert_eq!(prove(&contra, &no_b), Some(false));
        assert_eq!(prove(&taut, &no_b), Some(true));
        assert_eq!(prove(&contra, &no_b), Some(false));
    }

    #[test]
    fn z3_proves_tautology_no_bindings() {
        let no_b = HashMap::new();
        // `5 != 0` — provably true, no free variables.
        let expr = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral {
                value: 5,
                span: crate::span::Span::default(),
            }),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn z3_proves_contradiction_no_bindings() {
        let no_b = HashMap::new();
        // `0 != 0` — provably false.
        let expr = Node::InfixExpression {
            left: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            operator: "!=".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), Some(false));
    }

    #[test]
    fn z3_proves_universal_tautology_with_free_var() {
        // `x + 0 == x` is true for all x — the kind of thing the
        // hand-rolled folder CAN'T prove because x is free.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier {
                    name: "x".to_string(),
                    span: crate::span::Span::default(),
                }),
                operator: "+".to_string(),
                right: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: crate::span::Span::default(),
                }),
                span: crate::span::Span::default(),
            }),
            operator: "==".to_string(),
            right: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn z3_proves_implication_via_inequality() {
        // `x > 0 → x != 0`. We assert `x > 0 && !(x != 0)` — should
        // be unsat, meaning the implication holds. To frame it for
        // our prover: ask whether `(x > 0) || !(x > 0) || (x != 0)`
        // is a tautology. That's trivially true. A more interesting
        // case: prove `x * 2 > 0` from `x > 0`. We can't model the
        // implication directly with our prove() interface — instead
        // we build the combined formula as the input.
        // Simpler interesting case: `x > 0 || x <= 0` is a tautology.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier {
                    name: "x".to_string(),
                    span: crate::span::Span::default(),
                }),
                operator: ">".to_string(),
                right: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: crate::span::Span::default(),
                }),
                span: crate::span::Span::default(),
            }),
            operator: "||".to_string(),
            right: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier {
                    name: "x".to_string(),
                    span: crate::span::Span::default(),
                }),
                operator: "<=".to_string(),
                right: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: crate::span::Span::default(),
                }),
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn certificate_for_tautology_contains_negated_goal_and_check_sat() {
        // RES-071: a successfully proven tautology yields a self-
        // contained .smt2 file declaring every free identifier and
        // asserting the negation of the goal.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::InfixExpression {
                left: Box::new(Node::Identifier {
                    name: "x".to_string(),
                    span: crate::span::Span::default(),
                }),
                operator: "+".to_string(),
                right: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: crate::span::Span::default(),
                }),
                span: crate::span::Span::default(),
            }),
            operator: "==".to_string(),
            right: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        let (verdict, cert) = prove_with_certificate(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        let cert = cert.expect("tautology must yield a certificate");
        assert!(
            cert.smt2.contains("(declare-const x Int)"),
            "missing decl in:\n{}",
            cert.smt2
        );
        assert!(
            cert.smt2.contains("(check-sat)"),
            "missing check-sat in:\n{}",
            cert.smt2
        );
        assert!(
            cert.smt2.contains("(set-logic"),
            "missing set-logic in:\n{}",
            cert.smt2
        );
        assert!(
            cert.smt2.contains("(assert "),
            "missing negated assertion in:\n{}",
            cert.smt2
        );
    }

    #[test]
    fn certificate_pins_bound_identifiers_to_their_concrete_value() {
        // RES-071: when a parameter has a known constant binding, the
        // certificate must include an `(assert (= NAME VALUE))` so the
        // re-verification reflects the same call site.
        let mut bindings = HashMap::new();
        bindings.insert("n".to_string(), 5);
        let expr = Node::InfixExpression {
            left: Box::new(Node::Identifier {
                name: "n".to_string(),
                span: crate::span::Span::default(),
            }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        let (verdict, cert) = prove_with_certificate(&expr, &bindings);
        assert_eq!(verdict, Some(true));
        let cert = cert.expect("bound tautology must yield a certificate");
        assert!(cert.smt2.contains("(declare-const n Int)"));
        assert!(
            cert.smt2.contains("(assert (= n 5))"),
            "missing binding pin:\n{}",
            cert.smt2
        );
    }

    #[test]
    fn certificate_is_omitted_for_undecidable() {
        // RES-071: don't emit a certificate when there's no proof.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: crate::span::Span::default(),
            }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        let (_, cert) = prove_with_certificate(&expr, &no_b);
        assert!(cert.is_none(), "no proof => no cert");
    }

    #[test]
    fn z3_undecidable_returns_none_when_satisfiable() {
        // `x > 0` — neither tautology nor contradiction; Z3 returns
        // sat for both forms, so prove() returns None.
        let no_b = HashMap::new();
        let expr = Node::InfixExpression {
            left: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: crate::span::Span::default(),
            }),
            operator: ">".to_string(),
            right: Box::new(Node::IntegerLiteral {
                value: 0,
                span: crate::span::Span::default(),
            }),
            span: crate::span::Span::default(),
        };
        assert_eq!(prove(&expr, &no_b), None);
    }

    // --- RES-136: counterexample extraction from Z3 models ---

    /// Build `Node::Identifier { name }` with a default span.
    fn ident(name: &str) -> Node {
        Node::Identifier {
            name: name.to_string(),
            span: crate::span::Span::default(),
        }
    }

    /// Build `Node::IntegerLiteral { value }` with a default span.
    fn int(value: i64) -> Node {
        Node::IntegerLiteral {
            value,
            span: crate::span::Span::default(),
        }
    }

    /// Build `left OP right` with a default span.
    fn infix(left: Node, op: &str, right: Node) -> Node {
        Node::InfixExpression {
            left: Box::new(left),
            operator: op.to_string(),
            right: Box::new(right),
            span: crate::span::Span::default(),
        }
    }

    #[test]
    fn verifier_emits_counterexample_for_contradiction() {
        // `x > 5 && x < 0` — a strict contradiction. Verdict is
        // Some(false); counterexample is Z3's arbitrary witness to
        // the negation (anything not satisfying both conjuncts).
        let no_b = HashMap::new();
        let expr = infix(
            infix(ident("x"), ">", int(5)),
            "&&",
            infix(ident("x"), "<", int(0)),
        );
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(false));
        let cx = cx.expect("contradiction must surface a counterexample");
        assert!(
            cx.contains("x ="),
            "counterexample should name `x`; got: {:?}",
            cx
        );
    }

    #[test]
    fn verifier_emits_counterexample_for_undecidable() {
        // `x > 0` — undecidable (neither always true nor always
        // false). Verdict is None; counterexample is a concrete
        // assignment where the clause fails — any x <= 0.
        let no_b = HashMap::new();
        let expr = infix(ident("x"), ">", int(0));
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, None);
        let cx = cx.expect("undecidable clause must surface a counterexample");
        assert!(
            cx.contains("x ="),
            "counterexample should name `x`; got: {:?}",
            cx
        );
    }

    #[test]
    fn verifier_omits_counterexample_for_tautology() {
        // `x + 0 == x` — tautology. No counterexample expected.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "+", int(0)), "==", ident("x"));
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        assert!(
            cx.is_none(),
            "tautology should have no counterexample, got: {:?}",
            cx
        );
    }

    #[test]
    fn counterexample_omits_bound_identifiers() {
        // `n > 10` with `n` bound to 5: verdict is Some(false) and
        // the counterexample should NOT re-echo the pinned binding
        // (it's uninformative to print what the user already told us).
        let mut bindings = HashMap::new();
        bindings.insert("n".to_string(), 5);
        let expr = infix(ident("n"), ">", int(10));
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &bindings);
        assert_eq!(verdict, Some(false));
        // No free variables → no counterexample content.
        assert!(
            cx.as_deref().map(|s| !s.contains("n =")).unwrap_or(true),
            "bound identifier should not appear in counterexample: {:?}",
            cx,
        );
    }

    #[test]
    fn counterexample_names_multiple_free_identifiers() {
        // `a > 0 && b < 0` — undecidable.
        // Negation: `a <= 0 || b >= 0`. Z3 may only need to assign
        // one of the variables to satisfy a disjunction, so we
        // accept a counterexample that mentions at least one.
        let no_b = HashMap::new();
        let expr = infix(
            infix(ident("a"), ">", int(0)),
            "&&",
            infix(ident("b"), "<", int(0)),
        );
        let (verdict, _cert, cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, None);
        let cx = cx.expect("undecidable clause must surface a counterexample");
        assert!(
            cx.contains("a =") || cx.contains("b ="),
            "counterexample should name at least one free var; got: {:?}",
            cx,
        );
    }

    // --- RES-137: per-query timeout ---

    #[test]
    fn timeout_returns_timed_out_flag_on_hard_nia() {
        // Construct a non-linear integer arithmetic obligation that
        // Z3 can't decide in the default QF_NIA fragment without
        // significant work. `x * x = 2 * y * y + 3` (a variant of
        // Pell-style / norm-form equations) has integer solutions
        // that Z3's decision procedures won't exhaust quickly.
        //
        // With a 1ms timeout, Z3 should return Unknown and the
        // fourth return slot should be `true`. With no timeout,
        // Z3 might eventually settle (on this machine) — so we
        // only assert the timed-out path, not the unlimited path.
        let no_b = HashMap::new();
        // `x * x != 2 * y * y + 3` as an asserted-tautology query.
        // The negated-formula check forces Z3 to reason about the
        // full integer plane. A 1-ms budget is plenty to fire the
        // timeout before the solver finds its answer.
        let x = ident("x");
        let y = ident("y");
        let expr = infix(
            infix(x.clone(), "*", x.clone()),
            "!=",
            infix(
                infix(int(2), "*", infix(y.clone(), "*", y.clone())),
                "+",
                int(3),
            ),
        );
        let (_verdict, _cert, _cx, timed_out) = prove_with_timeout(&expr, &no_b, 1);
        assert!(
            timed_out,
            "expected the 1ms budget to trigger Z3's Unknown return"
        );
    }

    #[test]
    fn timeout_zero_disables_timeout() {
        // `x + 0 == x` is a straightforward tautology Z3 closes in
        // microseconds; the 0 (unlimited) timeout argument must
        // preserve the existing success path.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "+", int(0)), "==", ident("x"));
        let (verdict, _cert, _cx, timed_out) = prove_with_timeout(&expr, &no_b, 0);
        assert_eq!(verdict, Some(true));
        assert!(!timed_out, "unlimited timeout should not report timed_out");
    }

    // ---------- RES-131 (RES-131a): len(<ident>) SMT encoding ----------

    fn len_call(arg_name: &str) -> Node {
        Node::CallExpression {
            function: Box::new(Node::Identifier {
                name: "len".to_string(),
                span: crate::span::Span::default(),
            }),
            arguments: vec![Node::Identifier {
                name: arg_name.to_string(),
                span: crate::span::Span::default(),
            }],
            span: crate::span::Span::default(),
        }
    }

    fn int_lit(v: i64) -> Node {
        Node::IntegerLiteral {
            value: v,
            span: crate::span::Span::default(),
        }
    }

    #[test]
    fn len_of_ident_is_nonnegative_by_axiom() {
        // `len(xs) >= 0` — the injected axiom says this is
        // always true, so the solver proves it.
        let no_b = HashMap::new();
        let expr = infix(len_call("xs"), ">=", int_lit(0));
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn len_of_ident_gt_zero_is_not_universal() {
        // `len(xs) > 0` without a precondition is NOT a
        // tautology — the axiom only says `>= 0`, so `xs`
        // empty is still a valid Z3 model.
        let no_b = HashMap::new();
        let expr = infix(len_call("xs"), ">", int_lit(0));
        assert_eq!(prove(&expr, &no_b), None);
    }

    #[test]
    fn compound_formula_using_len_proves() {
        // `len(xs) >= 0 && 0 <= 0` — tautology reachable only
        // because both sides are discharged, and the `len`
        // side uses the axiom.
        let no_b = HashMap::new();
        let lhs = infix(len_call("xs"), ">=", int_lit(0));
        let rhs = infix(int_lit(0), "<=", int_lit(0));
        let expr = infix(lhs, "&&", rhs);
        assert_eq!(prove(&expr, &no_b), Some(true));
    }

    #[test]
    fn certificate_declares_len_const_and_axiom() {
        // Tautology round-trip: the SMT-LIB2 cert includes
        // a `(declare-const len_xs Int)` line + its `>= 0`
        // assertion so a stock Z3 can re-verify.
        let no_b = HashMap::new();
        let expr = infix(len_call("xs"), ">=", int_lit(0));
        let (_, cert, _cx) = prove_with_certificate_and_counterexample(&expr, &no_b);
        let smt2 = cert.expect("should produce a certificate").smt2;
        assert!(
            smt2.contains("(declare-const len_xs Int)"),
            "missing len_xs declaration in cert:\n{}",
            smt2
        );
        assert!(
            smt2.contains("(assert (>= len_xs 0))"),
            "missing len_xs >= 0 axiom in cert:\n{}",
            smt2
        );
    }

    #[test]
    fn multiple_len_calls_on_different_arrays_get_distinct_consts() {
        // `len(a) >= 0 && len(b) >= 0` — two distinct
        // Int consts + two axioms.
        let no_b = HashMap::new();
        let lhs = infix(len_call("a"), ">=", int_lit(0));
        let rhs = infix(len_call("b"), ">=", int_lit(0));
        let expr = infix(lhs, "&&", rhs);
        let (verdict, cert, _) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        let smt2 = cert.unwrap().smt2;
        assert!(smt2.contains("(declare-const len_a Int)"));
        assert!(smt2.contains("(declare-const len_b Int)"));
        assert!(smt2.contains("(assert (>= len_a 0))"));
        assert!(smt2.contains("(assert (>= len_b 0))"));
    }

    #[test]
    fn len_of_same_array_reuses_same_const() {
        // `len(xs) == len(xs)` — trivially true because the
        // same Z3 const is used on both sides. No two
        // different consts created.
        let no_b = HashMap::new();
        let expr = infix(len_call("xs"), "==", len_call("xs"));
        let (verdict, cert, _) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(true));
        let smt2 = cert.unwrap().smt2;
        // Exactly one `(declare-const len_xs Int)` line.
        assert_eq!(
            smt2.matches("(declare-const len_xs Int)").count(),
            1,
            "expected one declaration, got cert:\n{}",
            smt2
        );
    }

    #[test]
    fn len_with_non_identifier_arg_bails() {
        // `len(1)` — the arg isn't an identifier; translator
        // returns None and the existing fallback logic keeps
        // the runtime check. `prove` returns None.
        let no_b = HashMap::new();
        let call = Node::CallExpression {
            function: Box::new(Node::Identifier {
                name: "len".to_string(),
                span: crate::span::Span::default(),
            }),
            arguments: vec![int_lit(1)],
            span: crate::span::Span::default(),
        };
        let expr = infix(call, ">=", int_lit(0));
        assert_eq!(prove(&expr, &no_b), None);
    }

    #[test]
    fn collect_len_args_finds_all_references() {
        let expr = infix(infix(len_call("xs"), "+", len_call("ys")), ">", int_lit(0));
        let mut out: BTreeSet<&str> = BTreeSet::new();
        collect_len_args(&expr, &mut out);
        assert_eq!(out, ["xs", "ys"].into_iter().collect());
    }

    #[test]
    fn collect_len_args_ignores_non_len_calls() {
        // `foo(xs) + 1 > 0` — `foo` is not `len`, so the
        // collector returns an empty set.
        let foo_call = Node::CallExpression {
            function: Box::new(Node::Identifier {
                name: "foo".to_string(),
                span: crate::span::Span::default(),
            }),
            arguments: vec![Node::Identifier {
                name: "xs".to_string(),
                span: crate::span::Span::default(),
            }],
            span: crate::span::Span::default(),
        };
        let expr = infix(infix(foo_call, "+", int_lit(1)), ">", int_lit(0));
        let mut out: BTreeSet<&str> = BTreeSet::new();
        collect_len_args(&expr, &mut out);
        assert!(out.is_empty());
    }

    // ---------- FFI Phase 1 Task 10: trusted-ensures as axioms ----------

    #[test]
    fn axiom_promotes_undecidable_to_tautology() {
        // Without axioms, `r >= 0` with a free `r` is undecidable —
        // `r` could be negative. Feeding `r >= 0` as an axiom (as a
        // trusted extern's `ensures result >= 0` would do after
        // rewriting `result` → `r`) lets the solver close the proof.
        let no_b = HashMap::new();
        let goal = infix(ident("r"), ">=", int(0));
        let axiom = infix(ident("r"), ">=", int(0));
        let (verdict, _cert, _cx, _t) = prove_with_axioms_and_timeout(&goal, &no_b, &[axiom], 0);
        assert_eq!(verdict, Some(true));
    }

    #[test]
    fn empty_axioms_behaves_like_plain_prove() {
        // Passing an empty axiom slice must preserve the existing
        // `prove_with_timeout` behaviour — a regression check that
        // the new plumbing doesn't perturb the default path.
        let no_b = HashMap::new();
        let expr = infix(ident("x"), ">", int(0));
        let (v1, _, _, _) = prove_with_timeout(&expr, &no_b, 0);
        let (v2, _, _, _) = prove_with_axioms_and_timeout(&expr, &no_b, &[], 0);
        assert_eq!(v1, v2);
        assert_eq!(v1, None); // `x > 0` is undecidable without context
    }

    #[test]
    fn untranslatable_axiom_is_silently_skipped() {
        // A float literal inside an axiom can't be translated (the
        // verifier is integer-only). The axiom must be dropped
        // rather than panic; the goal proof proceeds as if no axiom
        // were supplied — here `x + 0 == x` is a plain tautology.
        let no_b = HashMap::new();
        let goal = infix(infix(ident("x"), "+", int(0)), "==", ident("x"));
        // Use a string literal as a deliberately untranslatable axiom.
        let bogus_axiom = Node::StringLiteral {
            value: "nope".to_string(),
            span: crate::span::Span::default(),
        };
        let (verdict, _cert, _cx, _t) =
            prove_with_axioms_and_timeout(&goal, &no_b, &[bogus_axiom], 0);
        assert_eq!(verdict, Some(true));
    }

    #[test]
    fn axiom_chain_enables_two_step_reasoning() {
        // Given two axioms `a > 0` and `b > a`, prove `b > 0`.
        // Neither axiom alone proves the goal, and the goal is
        // undecidable without them.
        let no_b = HashMap::new();
        let goal = infix(ident("b"), ">", int(0));
        let ax1 = infix(ident("a"), ">", int(0));
        let ax2 = infix(ident("b"), ">", ident("a"));
        let (verdict, _cert, _cx, _t) = prove_with_axioms_and_timeout(&goal, &no_b, &[ax1, ax2], 0);
        assert_eq!(verdict, Some(true));
    }

    // -------------------------------------------------------
    // RES-354: BV32 theory tests
    // -------------------------------------------------------

    #[test]
    fn bv_bitwise_and_mask_lower_bound() {
        // `(x & 0xF) >= 0` — BV signed: masking with 0xF (= 15)
        // can never set the sign bit on 32-bit values.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "&", int(15)), ">=", int(0));
        let (verdict, _cert, _cx, _t) = prove_bv(&expr, &no_b, 0);
        assert_eq!(verdict, Some(true), "bvand with 0xF should be >= 0");
    }

    #[test]
    fn bv_bitwise_and_mask_upper_bound() {
        // `(x & 0xF) <= 15` — the low nibble is at most 15.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "&", int(15)), "<=", int(15));
        let (verdict, _cert, _cx, _t) = prove_bv(&expr, &no_b, 0);
        assert_eq!(verdict, Some(true), "bvand with 0xF should be <= 15");
    }

    #[test]
    fn bv_xor_self_is_zero() {
        // `x ^ x == 0` — XOR with self is always zero.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "^", ident("x")), "==", int(0));
        let (verdict, _cert, _cx, _t) = prove_bv(&expr, &no_b, 0);
        assert_eq!(verdict, Some(true), "x ^ x should be 0");
    }

    #[test]
    fn bv_or_self_is_self() {
        // `x | x == x` — OR with self is identity.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "|", ident("x")), "==", ident("x"));
        let (verdict, _cert, _cx, _t) = prove_bv(&expr, &no_b, 0);
        assert_eq!(verdict, Some(true), "x | x should equal x");
    }

    #[test]
    fn bv_constant_shift_right() {
        // `(16 >> 4) == 1` — constant folding in BV32: 16 >> 4 = 1.
        let no_b = HashMap::new();
        let expr = infix(infix(int(16), ">>", int(4)), "==", int(1));
        let (verdict, _cert, _cx, _t) = prove_bv(&expr, &no_b, 0);
        assert_eq!(verdict, Some(true), "16 >> 4 should equal 1 in BV32");
    }

    #[test]
    fn bv_constant_shift_left() {
        // `(1 << 3) == 8` — constant left shift.
        let no_b = HashMap::new();
        let expr = infix(infix(int(1), "<<", int(3)), "==", int(8));
        let (verdict, _cert, _cx, _t) = prove_bv(&expr, &no_b, 0);
        assert_eq!(verdict, Some(true), "1 << 3 should equal 8 in BV32");
    }

    #[test]
    fn has_bitwise_ops_detects_and() {
        // `x & y > 0` — has_bitwise_ops must return true.
        let expr = infix(infix(ident("x"), "&", ident("y")), ">", int(0));
        assert!(has_bitwise_ops(&expr), "should detect & operator");
    }

    #[test]
    fn has_bitwise_ops_detects_shift() {
        // `x >> 2` — has_bitwise_ops must return true.
        let expr = infix(ident("x"), ">>", int(2));
        assert!(has_bitwise_ops(&expr), "should detect >> operator");
    }

    #[test]
    fn has_bitwise_ops_returns_false_for_pure_lia() {
        // `x + y > 0` — pure integer arithmetic, no bitwise ops.
        let expr = infix(infix(ident("x"), "+", ident("y")), ">", int(0));
        assert!(!has_bitwise_ops(&expr), "pure LIA should not trigger BV");
    }

    #[test]
    fn prove_auto_detects_and_uses_bv_for_bitwise_expr() {
        // `(x & 0xF) <= 15` — Auto should pick BV32 and prove it.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "&", int(15)), "<=", int(15));
        let (verdict, _cert, _cx, _t) = prove_auto(&expr, &no_b, Z3Theory::Auto, 0);
        assert_eq!(verdict, Some(true), "Auto should prove BV mask <= 15");
    }

    #[test]
    fn prove_auto_uses_lia_for_pure_arithmetic() {
        // `x + 0 == x` — Auto should use LIA and prove it.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "+", int(0)), "==", ident("x"));
        let (verdict, _cert, _cx, _t) = prove_auto(&expr, &no_b, Z3Theory::Auto, 0);
        assert_eq!(verdict, Some(true), "Auto should use LIA for x + 0 == x");
    }

    #[test]
    fn prove_auto_lia_forced_bails_on_bitwise_ops() {
        // `(x & 0xF) <= 15` with theory=Lia — should return None
        // because LIA cannot encode bitwise ops.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "&", int(15)), "<=", int(15));
        let (verdict, _cert, _cx, _t) = prove_auto(&expr, &no_b, Z3Theory::Lia, 0);
        assert_eq!(
            verdict, None,
            "Lia theory must bail (None) when bitwise ops are present"
        );
    }

    #[test]
    fn prove_auto_bv_forced_proves_pure_arithmetic() {
        // `x + 0 == x` with theory=Bv — BV32 should still prove
        // basic arithmetic identities.
        let no_b = HashMap::new();
        let expr = infix(infix(ident("x"), "+", int(0)), "==", ident("x"));
        let (verdict, _cert, _cx, _t) = prove_auto(&expr, &no_b, Z3Theory::Bv, 0);
        assert_eq!(verdict, Some(true), "Bv theory should prove x + 0 == x");
    }

    // -----------------------------------------------------------
    // RES-408: array-theory quantifier tests
    // -----------------------------------------------------------

    /// Build `target[index]` (`Node::IndexExpression`) with default span.
    fn index_expr(target: Node, index: Node) -> Node {
        Node::IndexExpression {
            target: Box::new(target),
            index: Box::new(index),
            span: crate::span::Span::default(),
        }
    }

    /// Build `forall id in lo..hi: body` (`Node::Quantifier`) with default span.
    fn forall_range(id: &str, lo: Node, hi: Node, body: Node) -> Node {
        Node::Quantifier {
            kind: crate::quantifiers::QuantifierKind::Forall,
            var: id.to_string(),
            range: crate::quantifiers::QuantRange::Range {
                lo: Box::new(lo),
                hi: Box::new(hi),
            },
            body: Box::new(body),
            span: crate::span::Span::default(),
        }
    }

    /// Build `exists id in lo..hi: body` (`Node::Quantifier`) with default span.
    fn exists_range(id: &str, lo: Node, hi: Node, body: Node) -> Node {
        Node::Quantifier {
            kind: crate::quantifiers::QuantifierKind::Exists,
            var: id.to_string(),
            range: crate::quantifiers::QuantRange::Range {
                lo: Box::new(lo),
                hi: Box::new(hi),
            },
            body: Box::new(body),
            span: crate::span::Span::default(),
        }
    }

    #[test]
    fn z3_proves_forall_array_reflexive_body() {
        // `forall i in 0..len(a): a[i] == a[i]` — body is a literal
        // tautology under any value of `(select arr_a i)`. Z3 must
        // prove it via array theory; if the IndexExpression arm
        // bailed to None the whole quantifier would translate to None
        // and the verdict would be None.
        let no_b = HashMap::new();
        let body = infix(
            index_expr(ident("a"), ident("i")),
            "==",
            index_expr(ident("a"), ident("i")),
        );
        let q = forall_range("i", int(0), len_call("a"), body);
        let (verdict, cert, _) = prove_with_certificate_and_counterexample(&q, &no_b);
        assert_eq!(
            verdict,
            Some(true),
            "forall i in 0..len(a): a[i] == a[i] must be a tautology"
        );
        let smt2 = cert.expect("expected cert for tautology").smt2;
        assert!(
            smt2.contains("(declare-const arr_a (Array Int Int))"),
            "cert must declare arr_a as Array Int Int:\n{}",
            smt2
        );
        assert!(
            smt2.contains("(declare-const len_a Int)"),
            "cert must declare len_a Int (range upper bound is len(a)):\n{}",
            smt2
        );
        assert!(
            smt2.contains("(assert (>= len_a 0))"),
            "cert must include len_a >= 0 axiom:\n{}",
            smt2
        );
    }

    #[test]
    fn z3_exists_array_irreflexive_witness_is_contradiction() {
        // `exists i in 0..len(a): a[i] != a[i]` is a contradiction:
        // for any value of `(select arr_a i)`, `v != v` is false.
        // Z3 must produce verdict = Some(false). Exercises the
        // existential encoding through array theory.
        let no_b = HashMap::new();
        let body = infix(
            index_expr(ident("a"), ident("i")),
            "!=",
            index_expr(ident("a"), ident("i")),
        );
        let q = exists_range("i", int(0), len_call("a"), body);
        let (verdict, _cert, _) = prove_with_certificate_and_counterexample(&q, &no_b);
        assert_eq!(
            verdict,
            Some(false),
            "exists i: a[i] != a[i] is a contradiction"
        );
    }

    #[test]
    fn z3_index_expression_outside_quantifier() {
        // `a[0] == a[0]` — index access outside a quantifier still
        // lowers to (select arr_a 0); reflexivity is trivially true.
        let no_b = HashMap::new();
        let expr = infix(
            index_expr(ident("a"), int(0)),
            "==",
            index_expr(ident("a"), int(0)),
        );
        let (verdict, _cert, _) = prove_with_certificate_and_counterexample(&expr, &no_b);
        assert_eq!(verdict, Some(true));
    }

    #[test]
    fn collect_array_args_finds_index_target() {
        // `a[i] + b[j]` — both `a` and `b` should be picked up.
        let expr = infix(
            index_expr(ident("a"), ident("i")),
            "+",
            index_expr(ident("b"), ident("j")),
        );
        let mut out: BTreeSet<&str> = BTreeSet::new();
        collect_array_args(&expr, &mut out);
        assert_eq!(out, ["a", "b"].into_iter().collect());
    }

    #[test]
    fn collect_array_args_walks_into_quantifier_body() {
        // `forall i in 0..len(xs): xs[i] >= 0` — `xs` is referenced
        // inside the body; collector must recurse.
        let body = infix(index_expr(ident("xs"), ident("i")), ">=", int(0));
        let q = forall_range("i", int(0), len_call("xs"), body);
        let mut out: BTreeSet<&str> = BTreeSet::new();
        collect_array_args(&q, &mut out);
        assert_eq!(out, ["xs"].into_iter().collect());
    }

    #[test]
    fn collect_int_identifiers_excludes_quantifier_bound_var() {
        // `forall i in 0..len(a): i + x >= 0` — the bound `i` must
        // NOT be in the collected idents (otherwise the cert would
        // declare it and shadow the forall binding). `x` and the
        // implicit `len` arg `a` should still be picked up via their
        // dedicated collectors.
        let body = infix(infix(ident("i"), "+", ident("x")), ">=", int(0));
        let q = forall_range("i", int(0), len_call("a"), body);
        let mut idents: BTreeSet<&str> = BTreeSet::new();
        collect_int_identifiers(&q, &mut idents);
        assert!(!idents.contains("i"), "bound var leaked into idents");
        assert!(idents.contains("x"), "free var x missing from idents");
    }

    #[test]
    fn z3_index_with_non_identifier_target_bails() {
        // `[1, 2, 3][0]` — target is a literal-array, not an
        // Identifier. translate_int returns None; prove returns
        // None (runtime fallback fires).
        let no_b = HashMap::new();
        let arr_lit = Node::ArrayLiteral {
            items: vec![int(1), int(2), int(3)],
            span: crate::span::Span::default(),
        };
        let expr = infix(index_expr(arr_lit, int(0)), "==", int(1));
        assert_eq!(
            prove(&expr, &no_b),
            None,
            "non-identifier index target must bail to None"
        );
    }

    fn unique_cache_path(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "res-1661-{}-{}-{}.json",
            tag,
            std::process::id(),
            nanos
        ))
    }

    /// RES-1661: a versioned envelope written by `save` must round-trip
    /// through `load` with the current `CARGO_PKG_VERSION`. Loading the
    /// same path twice is idempotent — keys collapse into the global
    /// proven-set.
    #[test]
    fn persistent_cache_envelope_roundtrip() {
        let path = unique_cache_path("roundtrip");
        save_persistent_proven(&path).expect("save");
        let text = std::fs::read_to_string(&path).expect("read");
        let v: serde_json::Value = serde_json::from_str(&text).expect("envelope is JSON");
        assert_eq!(
            v.get("compiler_version").and_then(|x| x.as_str()),
            Some(env!("CARGO_PKG_VERSION")),
            "envelope must stamp current compiler version"
        );
        assert!(
            v.get("keys").and_then(|x| x.as_array()).is_some(),
            "envelope must contain a `keys` array"
        );
        // Load round-trips without error and returns the same key count
        // the envelope claims (the global proven-set may have grown,
        // but load returns only how many *this file* contributed).
        let claimed_len = v.get("keys").and_then(|x| x.as_array()).unwrap().len();
        let loaded = load_persistent_proven(&path).expect("load");
        assert_eq!(loaded, claimed_len, "load count matches envelope keys");
        let _ = std::fs::remove_file(&path);
    }

    /// RES-1661: a stale envelope (different `compiler_version`) must
    /// be ignored — loading returns Ok(0) without erroring. This is
    /// the cross-version invalidation contract.
    #[test]
    fn persistent_cache_version_mismatch_returns_zero() {
        let path = unique_cache_path("version-mismatch");
        let stale = serde_json::json!({
            "compiler_version": "0.0.0-stale-from-test",
            "keys": [1u64, 2u64, 3u64],
        });
        std::fs::write(&path, serde_json::to_string(&stale).unwrap()).expect("write stale");
        let loaded = load_persistent_proven(&path).expect("load mismatched envelope");
        assert_eq!(loaded, 0, "stale version must be silently dropped");
        let _ = std::fs::remove_file(&path);
    }

    /// RES-1661: the pre-RES-1661 on-disk format was a flat `[u64,...]`
    /// array with no envelope. Such files must silently fail to parse
    /// (Ok(0)) so an upgrade clears the cache instead of erroring.
    #[test]
    fn persistent_cache_flat_array_is_ignored() {
        let path = unique_cache_path("flat-array");
        std::fs::write(&path, "[1,2,3,4]").expect("write flat array");
        let loaded = load_persistent_proven(&path).expect("flat array must not error");
        assert_eq!(loaded, 0, "pre-RES-1661 format is treated as empty cache");
        let _ = std::fs::remove_file(&path);
    }

    /// RES-1661: a missing cache file is the steady-state of a fresh
    /// checkout. It must not error.
    #[test]
    fn persistent_cache_missing_file_is_ok() {
        let path = unique_cache_path("missing");
        // Ensure absent.
        let _ = std::fs::remove_file(&path);
        let loaded = load_persistent_proven(&path).expect("missing file is Ok");
        assert_eq!(loaded, 0);
    }

    /// RES-1661: corrupt (non-JSON) cache files must not propagate an
    /// `InvalidData` error to the CLI — the cache is purely advisory.
    #[test]
    fn persistent_cache_garbage_input_is_ignored() {
        let path = unique_cache_path("garbage");
        std::fs::write(&path, "not even valid JSON {{").expect("write garbage");
        let loaded = load_persistent_proven(&path).expect("garbage must be swallowed");
        assert_eq!(loaded, 0);
        let _ = std::fs::remove_file(&path);
    }

    fn bool_lit(value: bool) -> Node {
        Node::BooleanLiteral {
            value,
            span: crate::span::Span::default(),
        }
    }

    /// RES-1663: `BooleanLiteral(true)` proves trivially without any
    /// Z3 round trip — `cache_stats` shows zero verdict misses for the
    /// query.
    #[test]
    fn trivial_true_literal_short_circuits_z3() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        assert_eq!(prove(&bool_lit(true), &no_b), Some(true));
        let after = cache_stats();
        assert_eq!(
            after.verdict_misses, before.verdict_misses,
            "BooleanLiteral(true) must not consult Z3"
        );
        assert_eq!(
            after.verdict_hits, before.verdict_hits,
            "and must not consult the verdict cache either"
        );
    }

    /// RES-1663: `BooleanLiteral(false)` is correctly verified as not
    /// a tautology without any Z3 round trip.
    #[test]
    fn trivial_false_literal_short_circuits_z3() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        assert_eq!(prove(&bool_lit(false), &no_b), Some(false));
        let after = cache_stats();
        assert_eq!(
            after.verdict_misses, before.verdict_misses,
            "BooleanLiteral(false) must not consult Z3"
        );
    }

    /// RES-1663: the tautology-only entry point honors the fast path.
    #[test]
    fn trivial_literal_short_circuits_tautology_path() {
        let no_b = HashMap::new();
        let no_ax: Vec<Node> = Vec::new();
        reset_cache_stats();
        let before = cache_stats();
        let (proven, _cert, timed_out) =
            prove_tautology_with_axioms_and_timeout(&bool_lit(true), &no_b, &no_ax, 0);
        assert!(proven);
        assert!(!timed_out);
        let (proven_false, _, _) =
            prove_tautology_with_axioms_and_timeout(&bool_lit(false), &no_b, &no_ax, 0);
        assert!(!proven_false);
        let after = cache_stats();
        assert_eq!(
            after.tautology_misses, before.tautology_misses,
            "tautology fast path must not dispatch Z3 for BooleanLiteral"
        );
    }

    /// RES-1663: the BV32 entry point honors the fast path. Without
    /// it the bare-literal obligation would round-trip through a BV32
    /// solver instance.
    #[test]
    fn trivial_literal_short_circuits_bv_path() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        let (verdict, _, _, _timed_out) = prove_bv(&bool_lit(true), &no_b, 0);
        assert_eq!(verdict, Some(true));
        let after = cache_stats();
        assert_eq!(
            after.bv_misses, before.bv_misses,
            "BV fast path must not dispatch Z3 for BooleanLiteral"
        );
    }

    /// RES-1665: every comparison operator over two `IntegerLiteral`
    /// operands folds to the expected verdict.
    #[test]
    fn const_int_compare_covers_every_operator() {
        assert_eq!(
            try_const_eval_bool(&infix(int(5), "==", int(5))),
            Some(true)
        );
        assert_eq!(
            try_const_eval_bool(&infix(int(5), "==", int(6))),
            Some(false)
        );
        assert_eq!(
            try_const_eval_bool(&infix(int(5), "!=", int(6))),
            Some(true)
        );
        assert_eq!(
            try_const_eval_bool(&infix(int(5), "!=", int(5))),
            Some(false)
        );
        assert_eq!(try_const_eval_bool(&infix(int(3), "<", int(5))), Some(true));
        assert_eq!(
            try_const_eval_bool(&infix(int(5), "<", int(3))),
            Some(false)
        );
        assert_eq!(
            try_const_eval_bool(&infix(int(5), "<=", int(5))),
            Some(true)
        );
        assert_eq!(
            try_const_eval_bool(&infix(int(6), "<=", int(5))),
            Some(false)
        );
        assert_eq!(try_const_eval_bool(&infix(int(5), ">", int(3))), Some(true));
        assert_eq!(
            try_const_eval_bool(&infix(int(3), ">", int(5))),
            Some(false)
        );
        assert_eq!(
            try_const_eval_bool(&infix(int(5), ">=", int(5))),
            Some(true)
        );
        assert_eq!(
            try_const_eval_bool(&infix(int(4), ">=", int(5))),
            Some(false)
        );
    }

    /// RES-1665: non-comparison operators (`+`, `&&`, ...) and
    /// non-literal operands return None — the caller falls through
    /// to the regular Z3 dispatch.
    #[test]
    fn const_int_compare_falls_through_on_non_comparison() {
        // `5 + 3` is not a comparison.
        assert_eq!(try_const_eval_bool(&infix(int(5), "+", int(3))), None);
        // `x > 3` has a free variable.
        assert_eq!(try_const_eval_bool(&infix(ident("x"), ">", int(3))), None);
        // `5 > x` has a free variable on the right.
        assert_eq!(try_const_eval_bool(&infix(int(5), ">", ident("x"))), None);
        // A bare literal (no comparison) returns None.
        assert_eq!(try_const_eval_bool(&int(5)), None);
    }

    /// RES-1665: `prove(...)` short-circuits when the obligation is a
    /// literal-vs-literal comparison — no verdict miss is recorded.
    #[test]
    fn const_int_compare_short_circuits_z3_lia_path() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        assert_eq!(prove(&infix(int(7), ">", int(3)), &no_b), Some(true));
        assert_eq!(prove(&infix(int(7), "<", int(3)), &no_b), Some(false));
        let after = cache_stats();
        assert_eq!(
            after.verdict_misses, before.verdict_misses,
            "literal-comparison obligations must not consult Z3"
        );
    }

    /// RES-1665: same fast path on the tautology entry point. Both
    /// the "is tautology" and "not a tautology" answers come back
    /// without dispatching Z3.
    #[test]
    fn const_int_compare_short_circuits_tautology_path() {
        let no_b = HashMap::new();
        let no_ax: Vec<Node> = Vec::new();
        reset_cache_stats();
        let before = cache_stats();
        let (proven_t, _, _) =
            prove_tautology_with_axioms_and_timeout(&infix(int(5), "==", int(5)), &no_b, &no_ax, 0);
        assert!(proven_t, "5 == 5 must prove as tautology");
        let (proven_f, _, _) =
            prove_tautology_with_axioms_and_timeout(&infix(int(5), "==", int(6)), &no_b, &no_ax, 0);
        assert!(!proven_f, "5 == 6 must not prove as tautology");
        let after = cache_stats();
        assert_eq!(
            after.tautology_misses, before.tautology_misses,
            "tautology fast path must not dispatch Z3 for literal comparisons"
        );
    }

    /// RES-1665: same fast path on the BV32 entry point.
    #[test]
    fn const_int_compare_short_circuits_bv_path() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        let (v, _, _, _) = prove_bv(&infix(int(10), ">=", int(10)), &no_b, 0);
        assert_eq!(v, Some(true));
        let after = cache_stats();
        assert_eq!(
            after.bv_misses, before.bv_misses,
            "BV fast path must not dispatch Z3 for literal comparisons"
        );
    }

    fn not_node(inner: Node) -> Node {
        Node::PrefixExpression {
            operator: "!".to_string(),
            right: Box::new(inner),
            span: crate::span::Span::default(),
        }
    }

    /// RES-1667: negation folds when the inner expression folds.
    #[test]
    fn const_eval_negation_of_bool_literal() {
        assert_eq!(try_const_eval_bool(&not_node(bool_lit(true))), Some(false));
        assert_eq!(try_const_eval_bool(&not_node(bool_lit(false))), Some(true));
    }

    /// RES-1667: negation recurses through an integer comparison.
    #[test]
    fn const_eval_negation_of_int_compare() {
        // !(5 > 3) → !true → false
        assert_eq!(
            try_const_eval_bool(&not_node(infix(int(5), ">", int(3)))),
            Some(false)
        );
        // !(3 > 5) → !false → true
        assert_eq!(
            try_const_eval_bool(&not_node(infix(int(3), ">", int(5)))),
            Some(true)
        );
    }

    /// RES-1667: double negation collapses correctly.
    #[test]
    fn const_eval_double_negation() {
        // !!true → true
        assert_eq!(
            try_const_eval_bool(&not_node(not_node(bool_lit(true)))),
            Some(true)
        );
        // !!false → false
        assert_eq!(
            try_const_eval_bool(&not_node(not_node(bool_lit(false)))),
            Some(false)
        );
    }

    /// RES-1667: negation of a non-foldable expression returns None.
    #[test]
    fn const_eval_negation_falls_through() {
        assert_eq!(try_const_eval_bool(&not_node(ident("x"))), None);
        // !(x > 3) — inner has a free variable.
        assert_eq!(
            try_const_eval_bool(&not_node(infix(ident("x"), ">", int(3)))),
            None
        );
    }

    /// RES-1667: `&&` short-circuits on a constant-False side, even
    /// when the other side has a free variable (Z3 can't change a
    /// `false` conjunct).
    #[test]
    fn const_eval_and_short_circuits_on_false() {
        // false && x → false
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(false), "&&", ident("x"))),
            Some(false)
        );
        // x && false → false
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "&&", bool_lit(false))),
            Some(false)
        );
        // (5 < 3) && x → false  (left folds to false)
        assert_eq!(
            try_const_eval_bool(&infix(infix(int(5), "<", int(3)), "&&", ident("x"))),
            Some(false)
        );
    }

    /// RES-1667: `&&` folds to true only when both sides are
    /// constant-True; otherwise None and the caller falls through.
    #[test]
    fn const_eval_and_both_true_or_undetermined() {
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(true), "&&", bool_lit(true))),
            Some(true)
        );
        // true && x — can't determine; falls through.
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(true), "&&", ident("x"))),
            None
        );
        // x && true — same.
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "&&", bool_lit(true))),
            None
        );
        // x && y — neither side folds.
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "&&", ident("y"))),
            None
        );
    }

    /// RES-1667: `||` short-circuits on a constant-True side.
    #[test]
    fn const_eval_or_short_circuits_on_true() {
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(true), "||", ident("x"))),
            Some(true)
        );
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "||", bool_lit(true))),
            Some(true)
        );
        // (5 > 3) || x → true
        assert_eq!(
            try_const_eval_bool(&infix(infix(int(5), ">", int(3)), "||", ident("x"))),
            Some(true)
        );
    }

    /// RES-1667: `||` folds to false only when both sides fold to
    /// false; otherwise None.
    #[test]
    fn const_eval_or_both_false_or_undetermined() {
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(false), "||", bool_lit(false))),
            Some(false)
        );
        // false || x — falls through.
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(false), "||", ident("x"))),
            None
        );
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "||", ident("y"))),
            None
        );
    }

    /// RES-1667: the unified helper drives the LIA prove entry point
    /// for the new patterns — `false && x` must come back as
    /// Some(false) with no verdict miss recorded.
    #[test]
    fn const_eval_short_circuits_z3_on_compound_obligations() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        assert_eq!(
            prove(&infix(bool_lit(false), "&&", ident("x")), &no_b),
            Some(false)
        );
        assert_eq!(
            prove(&infix(bool_lit(true), "||", ident("x")), &no_b),
            Some(true)
        );
        assert_eq!(
            prove(&not_node(infix(int(5), ">", int(3))), &no_b),
            Some(false)
        );
        let after = cache_stats();
        assert_eq!(
            after.verdict_misses, before.verdict_misses,
            "compound constant-folded obligations must not consult Z3"
        );
    }

    /// RES-1673: `x == x` folds to true via the reflexive Identifier
    /// fast path — no Z3 round trip.
    #[test]
    fn const_eval_reflexive_identifier_equality_folds() {
        // `x == x` → true
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "==", ident("x"))),
            Some(true)
        );
        // `x != x` → false
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "!=", ident("x"))),
            Some(false)
        );
        // `x <= x` → true (reflexive)
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "<=", ident("x"))),
            Some(true)
        );
        // `x >= x` → true (reflexive)
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), ">=", ident("x"))),
            Some(true)
        );
        // `x < x` → false (irreflexive strict order)
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "<", ident("x"))),
            Some(false)
        );
        // `x > x` → false
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), ">", ident("x"))),
            Some(false)
        );
    }

    /// RES-1673: `x == y` (different names) does NOT fold — the
    /// caller falls through to Z3.
    #[test]
    fn const_eval_reflexive_falls_through_on_different_names() {
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "==", ident("y"))),
            None
        );
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "<", ident("y"))),
            None
        );
    }

    /// RES-1673: reflexive folding only applies to comparison
    /// operators. Arithmetic operators like `+`, `-` aren't
    /// boolean-typed and never fold here.
    #[test]
    fn const_eval_reflexive_only_for_comparisons() {
        // `x + x` is not a boolean expression — return None.
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "+", ident("x"))),
            None
        );
        assert_eq!(
            try_const_eval_bool(&infix(ident("x"), "-", ident("x"))),
            None
        );
    }

    /// RES-1673: reflexive folding deliberately does NOT match
    /// arbitrary structurally-equal expressions (a more general
    /// helper would need spanless structural equality, not hash-based
    /// equality which has collision risk). `(a + 1) == (a + 1)` is
    /// allowed to fall through to Z3.
    #[test]
    fn const_eval_reflexive_skips_compound_expressions() {
        // Both sides are structurally equal compound expressions,
        // but the conservative helper only matches bare Identifiers.
        let e1 = infix(ident("a"), "+", int(1));
        let e2 = infix(ident("a"), "+", int(1));
        assert_eq!(try_const_eval_bool(&infix(e1, "==", e2)), None);
    }

    /// RES-1673: reflexive Identifier comparison short-circuits the
    /// LIA prove entry point — no Z3 round trip needed.
    #[test]
    fn const_eval_reflexive_short_circuits_z3() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        assert_eq!(
            prove(&infix(ident("x"), "==", ident("x")), &no_b),
            Some(true)
        );
        assert_eq!(
            prove(&infix(ident("x"), "<", ident("x")), &no_b),
            Some(false)
        );
        let after = cache_stats();
        assert_eq!(
            after.verdict_misses, before.verdict_misses,
            "reflexive Identifier comparison must not consult Z3"
        );
    }

    /// RES-1675: `try_const_int` folds raw integer literals.
    #[test]
    fn const_int_folds_literal() {
        assert_eq!(try_const_int(&int(42)), Some(42));
        assert_eq!(try_const_int(&int(-7)), Some(-7));
    }

    /// RES-1675: `try_const_int` folds unary minus.
    #[test]
    fn const_int_folds_unary_minus() {
        let neg5 = Node::PrefixExpression {
            operator: "-".to_string(),
            right: Box::new(int(5)),
            span: crate::span::Span::default(),
        };
        assert_eq!(try_const_int(&neg5), Some(-5));
    }

    /// RES-1675: `try_const_int` folds basic arithmetic.
    #[test]
    fn const_int_folds_arithmetic() {
        assert_eq!(try_const_int(&infix(int(5), "+", int(3))), Some(8));
        assert_eq!(try_const_int(&infix(int(5), "-", int(3))), Some(2));
        assert_eq!(try_const_int(&infix(int(5), "*", int(3))), Some(15));
        assert_eq!(try_const_int(&infix(int(7), "/", int(2))), Some(3));
        assert_eq!(try_const_int(&infix(int(7), "%", int(3))), Some(1));
    }

    /// RES-1675: `try_const_int` returns None on overflow rather than
    /// wrapping silently. Z3 sees the original expression and decides.
    #[test]
    fn const_int_returns_none_on_overflow() {
        // i64::MAX + 1 overflows.
        assert_eq!(try_const_int(&infix(int(i64::MAX), "+", int(1))), None);
        // i64::MIN * -1 overflows (one more positive value than negative).
        assert_eq!(try_const_int(&infix(int(i64::MIN), "*", int(-1))), None);
        // i64::MIN / -1 overflows.
        assert_eq!(try_const_int(&infix(int(i64::MIN), "/", int(-1))), None);
    }

    /// RES-1675: division and modulo by zero return None.
    #[test]
    fn const_int_returns_none_on_div_by_zero() {
        assert_eq!(try_const_int(&infix(int(5), "/", int(0))), None);
        assert_eq!(try_const_int(&infix(int(5), "%", int(0))), None);
    }

    /// RES-1675: `try_const_int` falls through on free variables and
    /// non-arithmetic operators. (RES-1678 added bitwise ops; this
    /// test uses comparison operators which don't fold to i64.)
    #[test]
    fn const_int_falls_through_on_non_arith() {
        assert_eq!(try_const_int(&ident("x")), None);
        assert_eq!(try_const_int(&infix(int(5), "+", ident("x"))), None);
        // Comparison operators don't fold to i64.
        assert_eq!(try_const_int(&infix(int(5), "==", int(2))), None);
        assert_eq!(try_const_int(&infix(int(5), "<", int(2))), None);
    }

    /// RES-1675: nested arithmetic folds end-to-end.
    #[test]
    fn const_int_folds_nested_expressions() {
        // (2 + 3) * 4 → 20
        let inner = infix(int(2), "+", int(3));
        let outer = infix(inner, "*", int(4));
        assert_eq!(try_const_int(&outer), Some(20));
    }

    /// RES-1675: `try_const_eval_bool` folds comparisons whose
    /// operands fold via `try_const_int`. `5 + 3 == 8` decides
    /// without dispatching Z3.
    #[test]
    fn const_eval_arith_comparison_folds() {
        let lhs = infix(int(5), "+", int(3));
        assert_eq!(try_const_eval_bool(&infix(lhs, "==", int(8))), Some(true));
        let lhs2 = infix(int(7), "*", int(2));
        assert_eq!(try_const_eval_bool(&infix(lhs2, ">", int(13))), Some(true));
        // (2 + 3) >= 5 → true.
        let folded = infix(int(2), "+", int(3));
        assert_eq!(
            try_const_eval_bool(&infix(folded, ">=", int(5))),
            Some(true)
        );
    }

    /// RES-1675: comparisons where one side has a free var fall
    /// through to Z3 (as before).
    #[test]
    fn const_eval_arith_with_free_var_falls_through() {
        let lhs = infix(int(5), "+", ident("x"));
        assert_eq!(try_const_eval_bool(&infix(lhs, "==", int(8))), None);
    }

    /// RES-1675: the arithmetic fold short-circuits the LIA prove
    /// entry point — no verdict miss is recorded.
    #[test]
    fn const_eval_arith_short_circuits_z3() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        let lhs = infix(int(10), "+", int(15));
        assert_eq!(prove(&infix(lhs, "==", int(25)), &no_b), Some(true));
        let after = cache_stats();
        assert_eq!(
            after.verdict_misses, before.verdict_misses,
            "arithmetic-folded comparisons must not consult Z3"
        );
    }

    /// RES-1678: `try_const_int` folds bitwise AND, OR, XOR.
    #[test]
    fn const_int_folds_bitwise_and_or_xor() {
        // 0xF0 & 0x0F = 0
        assert_eq!(try_const_int(&infix(int(0xF0), "&", int(0x0F))), Some(0));
        // 0xF0 | 0x0F = 0xFF
        assert_eq!(try_const_int(&infix(int(0xF0), "|", int(0x0F))), Some(0xFF));
        // 0xFF ^ 0x0F = 0xF0
        assert_eq!(try_const_int(&infix(int(0xFF), "^", int(0x0F))), Some(0xF0));
    }

    /// RES-1678: `try_const_int` folds left and right shifts.
    #[test]
    fn const_int_folds_shifts() {
        // 1 << 4 = 16
        assert_eq!(try_const_int(&infix(int(1), "<<", int(4))), Some(16));
        // 16 >> 2 = 4
        assert_eq!(try_const_int(&infix(int(16), ">>", int(2))), Some(4));
        // 7 << 0 = 7 (identity)
        assert_eq!(try_const_int(&infix(int(7), "<<", int(0))), Some(7));
    }

    /// RES-1678: out-of-range shifts return None — the result would
    /// differ between LIA i64 (panic / poison) and BV32 (defined as
    /// implementation-specific or shift mod 32). Letting Z3 see the
    /// expression preserves soundness.
    #[test]
    fn const_int_returns_none_on_out_of_range_shift() {
        assert_eq!(try_const_int(&infix(int(1), "<<", int(64))), None);
        assert_eq!(try_const_int(&infix(int(1), "<<", int(100))), None);
        assert_eq!(try_const_int(&infix(int(1), "<<", int(-1))), None);
        assert_eq!(try_const_int(&infix(int(1), ">>", int(64))), None);
        assert_eq!(try_const_int(&infix(int(1), ">>", int(-1))), None);
    }

    /// RES-1678: a bitwise-folded comparison short-circuits the LIA
    /// prove path — no Z3 round trip needed.
    #[test]
    fn const_eval_bitwise_comparison_short_circuits_z3() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        // (1 << 4) == 16 → true
        let shift = infix(int(1), "<<", int(4));
        assert_eq!(prove(&infix(shift, "==", int(16)), &no_b), Some(true));
        // (0xF0 & 0x0F) == 0 → true
        let masked = infix(int(0xF0), "&", int(0x0F));
        assert_eq!(prove(&infix(masked, "==", int(0)), &no_b), Some(true));
        let after = cache_stats();
        assert_eq!(
            after.verdict_misses, before.verdict_misses,
            "bitwise-folded comparisons must not consult Z3"
        );
    }

    /// RES-1680: BooleanLiteral-vs-BooleanLiteral equality folds for
    /// every combination of `true` / `false`.
    #[test]
    fn const_eval_bool_literal_equality_folds() {
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(true), "==", bool_lit(true))),
            Some(true)
        );
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(false), "==", bool_lit(false))),
            Some(true)
        );
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(true), "==", bool_lit(false))),
            Some(false)
        );
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(false), "==", bool_lit(true))),
            Some(false)
        );
    }

    /// RES-1680: BooleanLiteral-vs-BooleanLiteral disequality is the
    /// dual of equality.
    #[test]
    fn const_eval_bool_literal_inequality_folds() {
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(true), "!=", bool_lit(true))),
            Some(false)
        );
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(true), "!=", bool_lit(false))),
            Some(true)
        );
    }

    /// RES-1680: non-`==`/`!=` comparisons on bool literals fall
    /// through — they don't typically appear in obligations and the
    /// semantics (does `false < true` make sense?) are intentionally
    /// not pinned here.
    #[test]
    fn const_eval_bool_literal_inequality_operators_fall_through() {
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(false), "<", bool_lit(true))),
            None
        );
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(true), ">=", bool_lit(false))),
            None
        );
    }

    /// RES-1680: bool-literal equality short-circuits the LIA prove
    /// entry point — no Z3 round trip.
    #[test]
    fn const_eval_bool_literal_equality_short_circuits_z3() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        assert_eq!(
            prove(&infix(bool_lit(true), "==", bool_lit(true)), &no_b),
            Some(true)
        );
        assert_eq!(
            prove(&infix(bool_lit(true), "!=", bool_lit(false)), &no_b),
            Some(true)
        );
        let after = cache_stats();
        assert_eq!(
            after.verdict_misses, before.verdict_misses,
            "bool-literal equality must not consult Z3"
        );
    }

    /// RES-1682: `prove_auto` itself short-circuits for foldable
    /// obligations — neither the `has_bitwise_ops` walk nor the
    /// dispatch to `prove_bv` / `prove_with_axioms_and_timeout` runs.
    /// Verifiable by asserting zero misses on ALL three downstream
    /// caches (verdict, tautology, bv).
    #[test]
    fn prove_auto_short_circuits_on_constant_fold() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();

        // Bool literal — covered by RES-1663 fold.
        let (v, _, _, _) = prove_auto(&bool_lit(true), &no_b, Z3Theory::Auto, 0);
        assert_eq!(v, Some(true));

        // Integer-literal comparison — covered by RES-1665 / RES-1675.
        let (v, _, _, _) = prove_auto(&infix(int(5), ">", int(3)), &no_b, Z3Theory::Auto, 0);
        assert_eq!(v, Some(true));

        // Arithmetic fold via RES-1675.
        let (v, _, _, _) = prove_auto(
            &infix(infix(int(2), "+", int(3)), "==", int(5)),
            &no_b,
            Z3Theory::Auto,
            0,
        );
        assert_eq!(v, Some(true));

        let after = cache_stats();
        assert_eq!(
            after.verdict_misses, before.verdict_misses,
            "prove_auto fold must not dispatch to LIA path"
        );
        assert_eq!(
            after.bv_misses, before.bv_misses,
            "prove_auto fold must not dispatch to BV path"
        );
    }

    /// RES-1682: forcing `Z3Theory::Bv` on a foldable obligation
    /// still short-circuits — the theory hint is irrelevant when
    /// the verdict is already determined.
    #[test]
    fn prove_auto_short_circuits_with_bv_theory_hint() {
        let no_b = HashMap::new();
        reset_cache_stats();
        let before = cache_stats();
        let (v, _, _, _) = prove_auto(&bool_lit(false), &no_b, Z3Theory::Bv, 0);
        assert_eq!(v, Some(false));
        let after = cache_stats();
        assert_eq!(
            after.bv_misses, before.bv_misses,
            "Z3Theory::Bv must not force BV dispatch for foldable inputs"
        );
    }

    /// RES-1684: generalized fold catches `(x == x) == true` — left
    /// folds via RES-1673 reflexive Identifier, right is a bool
    /// literal, the outer `==` compares them.
    #[test]
    fn const_eval_reflexive_compared_to_bool_literal_folds() {
        let reflexive_eq = infix(ident("x"), "==", ident("x")); // → true
        assert_eq!(
            try_const_eval_bool(&infix(reflexive_eq.clone(), "==", bool_lit(true))),
            Some(true)
        );
        assert_eq!(
            try_const_eval_bool(&infix(reflexive_eq.clone(), "==", bool_lit(false))),
            Some(false)
        );
        assert_eq!(
            try_const_eval_bool(&infix(reflexive_eq, "!=", bool_lit(false))),
            Some(true)
        );
    }

    /// RES-1684: generalized fold catches `(5 > 3) != false` — left
    /// folds via int-literal comparison.
    #[test]
    fn const_eval_int_compare_compared_to_bool_literal_folds() {
        let int_gt = infix(int(5), ">", int(3)); // → true
        assert_eq!(
            try_const_eval_bool(&infix(int_gt.clone(), "==", bool_lit(true))),
            Some(true)
        );
        assert_eq!(
            try_const_eval_bool(&infix(int_gt, "!=", bool_lit(true))),
            Some(false)
        );
    }

    /// RES-1684: both sides may be compound bool expressions —
    /// `(5 > 3) == (10 == 10)` folds to true. Validates that the
    /// recursion bottoms out at the leaves on both sides.
    #[test]
    fn const_eval_compound_bool_equality_folds() {
        let lhs = infix(int(5), ">", int(3)); // → true
        let rhs = infix(int(10), "==", int(10)); // → true
        assert_eq!(
            try_const_eval_bool(&infix(lhs.clone(), "==", rhs.clone())),
            Some(true)
        );
        let rhs_false = infix(int(10), "<", int(5)); // → false
        assert_eq!(
            try_const_eval_bool(&infix(lhs, "==", rhs_false)),
            Some(false)
        );
    }

    /// RES-1684: when either side does NOT fold (free variable), the
    /// generalized fold returns None and the caller falls through to
    /// Z3.
    #[test]
    fn const_eval_generalized_falls_through_on_non_foldable_side() {
        assert_eq!(
            try_const_eval_bool(&infix(bool_lit(true), "==", ident("flag"))),
            None
        );
        assert_eq!(
            try_const_eval_bool(&infix(ident("a"), "==", ident("b"))),
            None
        );
    }
}
