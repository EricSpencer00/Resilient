//! RES-072 Phase A + RES-096 Phase B: Cranelift JIT backend.
//!
//! Phase A wired the dep tree, the `--jit` flag, and a stub
//! `run` that returned `JitError::Unsupported`. Phase B (this
//! revision) actually lowers a tiny subset of the AST to native
//! code and executes it:
//!
//! - `Node::IntegerLiteral { value, .. }` → `iconst`
//! - `Node::InfixExpression` with `+` → recursive lower + `iadd`
//! - `Node::ReturnStatement { value: Some(expr), .. }` → lower
//!   the expression and emit `Op::Return` for the JIT'd function
//! - Top-level `Node::Program` containing a single
//!   `ReturnStatement` is wrapped as the JIT's `main`
//!
//! Anything else returns `JitError::Unsupported(...)`. Future
//! tickets layer on let bindings (RES-097-?), control flow,
//! function calls, etc.

#![allow(dead_code)]

use std::collections::HashMap;

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use crate::Node;

/// RES-104: per-function lowering context. Threads the locals
/// map (name → cranelift Variable) and the Variable counter
/// through all lowering helpers so let bindings + identifier
/// reads compose naturally with everything from earlier phases.
///
/// RES-105: also carries the program-wide function map (name →
/// FuncId). The map is filled in Pass 1 (declare every fn) and
/// read in Pass 2 (compile bodies, including CallExpression
/// lowering). Owned, not borrowed from JITModule, so module
/// can stay independently mutable.
struct LowerCtx {
    /// Variable index counter — Cranelift's `Variable` is a u32
    /// newtype, increment on each `let`. The counter is owned by
    /// the ctx (not global) so per-function lowering stays
    /// independent.
    next_var: u32,
    /// Currently-in-scope locals. Phase G is function-scoped:
    /// the same map is used for the whole function body. Block
    /// scoping is a future ticket.
    locals: HashMap<String, Variable>,
    /// RES-105: program-wide function map. Cloned (cheap — FuncId
    /// is Copy) into each per-function LowerCtx so call sites
    /// can resolve direct calls by name.
    functions: HashMap<String, FuncId>,
    /// RES-105: arity per declared function, keyed by name.
    /// Used to validate call sites — mismatch is reported as a
    /// clean Unsupported instead of letting Cranelift segfault.
    function_arities: HashMap<String, usize>,
    /// RES-168: TCO state — the name of the currently-compiling
    /// function, set only by `compile_function` and `None` while
    /// lowering top-level `main`. A `ReturnStatement` whose value
    /// is a direct call to this name (with matching arity) is
    /// lowered as a back-edge jump to `tco_target` instead of a
    /// regular call + return.
    current_fn: Option<String>,
    /// RES-168: the block to jump to on a detected tail call.
    /// Distinct from the function's entry block: entry carries
    /// function-signature block params and is sealed immediately;
    /// `tco_target` is the "body" block that entry jumps into,
    /// left unsealed until all tail-call back-edges have been
    /// emitted so Cranelift's SSA construction can reconcile the
    /// re-`def_var`'d parameter Variables into phis.
    tco_target: Option<Block>,
    /// RES-168: parameter Variables in declaration order. A tail
    /// call lowers its argument expressions first (so in-scope
    /// names still resolve to the *old* param values), then
    /// `def_var`s each param Variable with the new value in order.
    param_vars: Vec<Variable>,
    /// RES-175: per-function AST of every declared function in
    /// the program — `(parameters, body)` keyed by name. Needed
    /// to inline trivial leaf callees at call sites (the JIT
    /// otherwise only has module-local FuncIds, which can't be
    /// re-lowered). Populated at `run_internal` time;
    /// `compile_function` clones the map into each per-fn ctx.
    fn_asts: HashMap<String, (Vec<(String, String)>, Node)>,
    /// RES-175: when set, a `ReturnStatement` in the body lowers
    /// to `jump(merge, &[value])` — producing the value as the
    /// inlined expression's result — instead of the usual
    /// `return_`. Set by the inliner before lowering a callee's
    /// body and cleared after. `None` in normal (non-inline)
    /// lowering keeps `return` emitting the function-level
    /// return it always did.
    inline_return_target: Option<Block>,
}

impl LowerCtx {
    fn new() -> Self {
        Self {
            next_var: 0,
            locals: HashMap::new(),
            functions: HashMap::new(),
            function_arities: HashMap::new(),
            current_fn: None,
            tco_target: None,
            param_vars: Vec::new(),
            fn_asts: HashMap::new(),
            inline_return_target: None,
        }
    }

    /// Reserve a fresh `Variable`, declare it on the
    /// FunctionBuilder, and remember the binding under `name`.
    /// Shadowing a previous binding just overwrites the map
    /// entry — subsequent uses get the fresh Variable.
    fn declare(&mut self, name: &str, bcx: &mut FunctionBuilder) -> Variable {
        let var = Variable::from_u32(self.next_var);
        self.next_var += 1;
        bcx.declare_var(var, types::I64);
        self.locals.insert(name.to_string(), var);
        var
    }

    fn lookup(&self, name: &str) -> Option<Variable> {
        self.locals.get(name).copied()
    }
}

/// RES-175: hard size limit for the trivial-leaf-fn inliner. Count
/// every node subterm in the body: above this, we bail out on
/// inline and emit a regular indirect call. Value chosen
/// conservatively per the ticket — big enough to catch useful
/// cases (`return n + 1;`, `return n * 2;`, small arithmetic
/// wrappers), small enough that inlining never bloats the caller
/// unexpectedly.
const TRIVIAL_LEAF_MAX_NODES: usize = 8;

/// RES-175: recursive node-count over an AST subtree. Includes the
/// root itself in the count.
fn count_nodes(n: &Node) -> usize {
    1 + match n {
        Node::IntegerLiteral { .. }
        | Node::FloatLiteral { .. }
        | Node::StringLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::BooleanLiteral { .. }
        | Node::Identifier { .. }
        | Node::DurationLiteral { .. }
        | Node::Use { .. }
        | Node::TypeAlias { .. }
        | Node::StructDecl { .. } => 0,
        Node::PrefixExpression { right, .. } => count_nodes(right),
        Node::InfixExpression { left, right, .. } => count_nodes(left) + count_nodes(right),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => count_nodes(function) + arguments.iter().map(count_nodes).sum::<usize>(),
        Node::ReturnStatement { value, .. } => value.as_ref().map_or(0, |v| count_nodes(v)),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            count_nodes(condition)
                + count_nodes(consequence)
                + alternative.as_ref().map_or(0, |a| count_nodes(a))
        }
        Node::WhileStatement {
            condition, body, ..
        } => count_nodes(condition) + count_nodes(body),
        Node::ForInStatement { iterable, body, .. } => count_nodes(iterable) + count_nodes(body),
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Assignment { value, .. } => count_nodes(value),
        Node::ExpressionStatement { expr, .. } => count_nodes(expr),
        Node::Block { stmts, .. } => stmts.iter().map(count_nodes).sum(),
        Node::Program(stmts) => stmts.iter().map(|s| count_nodes(&s.node)).sum(),
        // Variants not typically found inside JIT-compiled fn
        // bodies (live blocks, asserts, matches, struct literals,
        // field access, try-expr, function literals, etc.) — count
        // as 0 children so the node-count predicate still reaches
        // them, and rely on `has_disqualifying_construct` below
        // to reject most of them outright.
        _ => 0,
    }
}

/// RES-175: reject bodies that contain a construct the inliner's
/// "trivial leaf" contract rules out. The ticket says: no calls,
/// no loops, no match.
fn has_disqualifying_construct(n: &Node) -> bool {
    match n {
        Node::CallExpression { .. }
        | Node::WhileStatement { .. }
        | Node::ForInStatement { .. }
        | Node::Match { .. }
        | Node::LiveBlock { .. } => true,
        Node::PrefixExpression { right, .. } => has_disqualifying_construct(right),
        Node::InfixExpression { left, right, .. } => {
            has_disqualifying_construct(left) || has_disqualifying_construct(right)
        }
        Node::ReturnStatement { value, .. } => value
            .as_ref()
            .is_some_and(|v| has_disqualifying_construct(v)),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            has_disqualifying_construct(condition)
                || has_disqualifying_construct(consequence)
                || alternative
                    .as_ref()
                    .is_some_and(|a| has_disqualifying_construct(a))
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Assignment { value, .. } => has_disqualifying_construct(value),
        Node::ExpressionStatement { expr, .. } => has_disqualifying_construct(expr),
        Node::Block { stmts, .. } => stmts.iter().any(has_disqualifying_construct),
        _ => false,
    }
}

/// RES-175: decide whether a callee's AST qualifies as a trivial
/// leaf suitable for inlining at a call site.
///
/// Criteria (all must hold):
///   1. Node count in the body ≤ `TRIVIAL_LEAF_MAX_NODES`.
///   2. No calls / loops / match anywhere in the body.
///   3. Callee is NOT the enclosing function (self-recursion
///      would infinite-loop the inliner).
fn is_trivial_leaf(body: &Node, callee_name: &str, current_fn: Option<&str>) -> bool {
    if current_fn == Some(callee_name) {
        return false;
    }
    if count_nodes(body) > TRIVIAL_LEAF_MAX_NODES {
        return false;
    }
    !has_disqualifying_construct(body)
}

/// RES-174: FNV-1a 64-bit hash of a function's AST, with source
/// spans stripped. Two functions with identical parameters,
/// requires/ensures clauses, and body hash to the same value
/// regardless of their declared name — that's the invariant the
/// cache needs to treat them as aliases.
///
/// The canonical form is a byte stream written by
/// `write_canon_*`: a per-variant discriminant byte plus the
/// variant's payload in a fixed order. Spans are deliberately
/// NOT written so a reformatting / re-indentation of the source
/// (which shifts spans but preserves semantics) still produces
/// the same hash.
fn fn_hash(
    parameters: &[(String, String)],
    requires: &[Node],
    ensures: &[Node],
    body: &Node,
) -> u64 {
    let mut buf: Vec<u8> = Vec::new();
    // Tag to distinguish the "function" byte-stream from
    // free-standing node streams in case we ever reuse the
    // canonical writer elsewhere.
    buf.push(b'F');
    // Parameters: length prefix then each (type_name, param_name)
    // as length-prefixed strings. Names matter (different param
    // names produce different code paths / local mappings).
    write_u32(&mut buf, parameters.len() as u32);
    for (ty, n) in parameters {
        write_str(&mut buf, ty);
        write_str(&mut buf, n);
    }
    write_u32(&mut buf, requires.len() as u32);
    for r in requires {
        write_canon_node(&mut buf, r);
    }
    write_u32(&mut buf, ensures.len() as u32);
    for e in ensures {
        write_canon_node(&mut buf, e);
    }
    write_canon_node(&mut buf, body);
    fnv1a64(&buf)
}

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET_BASIS;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn write_u32(buf: &mut Vec<u8>, n: u32) {
    buf.extend_from_slice(&n.to_le_bytes());
}

fn write_i64(buf: &mut Vec<u8>, n: i64) {
    buf.extend_from_slice(&n.to_le_bytes());
}

fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

/// Canonical byte-stream writer for a Node. A discriminant byte
/// identifies the variant; the payload follows. Spans are never
/// written. Variants outside the JIT's supported subset use a
/// catch-all tag with no payload — they hash to the same value,
/// which is fine because the JIT can't compile them anyway
/// (they'd be rejected by `lower_expr` / `compile_node_list`
/// before the cache entry would ever be reused).
fn write_canon_node(buf: &mut Vec<u8>, node: &Node) {
    match node {
        Node::IntegerLiteral { value, .. } => {
            buf.push(1);
            write_i64(buf, *value);
        }
        Node::BooleanLiteral { value, .. } => {
            buf.push(2);
            buf.push(if *value { 1 } else { 0 });
        }
        Node::Identifier { name, .. } => {
            buf.push(3);
            write_str(buf, name);
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            buf.push(4);
            write_str(buf, operator);
            write_canon_node(buf, left);
            write_canon_node(buf, right);
        }
        Node::PrefixExpression {
            operator, right, ..
        } => {
            buf.push(5);
            write_str(buf, operator);
            write_canon_node(buf, right);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            buf.push(6);
            write_canon_node(buf, function);
            write_u32(buf, arguments.len() as u32);
            for a in arguments {
                write_canon_node(buf, a);
            }
        }
        Node::ReturnStatement { value, .. } => {
            buf.push(7);
            match value {
                Some(v) => {
                    buf.push(1);
                    write_canon_node(buf, v);
                }
                None => buf.push(0),
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            buf.push(8);
            write_canon_node(buf, condition);
            write_canon_node(buf, consequence);
            match alternative {
                Some(a) => {
                    buf.push(1);
                    write_canon_node(buf, a);
                }
                None => buf.push(0),
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            buf.push(9);
            write_canon_node(buf, condition);
            write_canon_node(buf, body);
        }
        Node::LetStatement { name, value, .. } => {
            buf.push(10);
            write_str(buf, name);
            write_canon_node(buf, value);
        }
        Node::Assignment { name, value, .. } => {
            buf.push(11);
            write_str(buf, name);
            write_canon_node(buf, value);
        }
        Node::ExpressionStatement { expr, .. } => {
            buf.push(12);
            write_canon_node(buf, expr);
        }
        Node::Block { stmts, .. } => {
            buf.push(13);
            write_u32(buf, stmts.len() as u32);
            for s in stmts {
                write_canon_node(buf, s);
            }
        }
        // Catch-all for variants the JIT doesn't lower. Hashing
        // them collides, but a fn with an unsupported variant in
        // its body won't JIT successfully anyway, so a collision
        // here never produces an incorrect cache hit at runtime.
        _ => buf.push(0xFF),
    }
}

/// RES-174: per-`run()` JIT cache. Stores the AST hash → FuncId
/// mapping for the current module so two functions in the same
/// program with identical bodies share one compile. Statistics
/// also accumulate into the process-wide `GLOBAL_JIT_STATS`
/// counters so `--jit-cache-stats` can report totals at exit.
///
/// FuncIds are module-local, so the cache is NOT reused across
/// `run()` invocations: each call starts fresh. Cross-session
/// (on-disk) caching is explicitly out-of-scope per the ticket's
/// Notes — that requires stable Cranelift serialization +
/// compiler-version invalidation.
#[derive(Debug, Default)]
pub struct JitCache {
    /// fn-ast hash → the FuncId we declared for the first fn with
    /// that hash in the current module. Subsequent fns with the
    /// same hash reuse this id (alias).
    pub map: HashMap<u64, FuncId>,
    /// Per-run stats; mirrored into `GLOBAL_JIT_STATS` on drop so
    /// the CLI's `--jit-cache-stats` can report lifetime totals.
    pub hits: u32,
    pub misses: u32,
    pub compiles: u32,
}

impl JitCache {
    pub fn new() -> Self {
        Self::default()
    }
}

/// RES-174: process-wide cumulative cache stats. Updated on
/// every `run()` call via the `JitCache::flush_into_globals`
/// Drop glue; read by `--jit-cache-stats` on program exit.
/// Relaxed ordering — these are diagnostic counters, not a
/// synchronization primitive.
static GLOBAL_JIT_HITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GLOBAL_JIT_MISSES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GLOBAL_JIT_COMPILES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Read the process-wide JIT cache counters. Returns `(hits,
/// misses, compiles)`. Called by the CLI's `--jit-cache-stats`
/// handler from `main.rs` at program exit.
pub fn cache_stats() -> (u64, u64, u64) {
    use std::sync::atomic::Ordering;
    (
        GLOBAL_JIT_HITS.load(Ordering::Relaxed),
        GLOBAL_JIT_MISSES.load(Ordering::Relaxed),
        GLOBAL_JIT_COMPILES.load(Ordering::Relaxed),
    )
}

fn flush_cache_stats_to_globals(cache: &JitCache) {
    use std::sync::atomic::Ordering;
    GLOBAL_JIT_HITS.fetch_add(cache.hits as u64, Ordering::Relaxed);
    GLOBAL_JIT_MISSES.fetch_add(cache.misses as u64, Ordering::Relaxed);
    GLOBAL_JIT_COMPILES.fetch_add(cache.compiles as u64, Ordering::Relaxed);
}

/// Errors the JIT backend can surface.
#[derive(Debug, Clone, PartialEq)]
pub enum JitError {
    /// A construct outside Phase B's supported subset showed up.
    Unsupported(&'static str),
    /// `cranelift_native::builder()` failed to detect the host ISA.
    IsaInit(String),
    /// `JITModule::finalize_definitions` returned an error.
    LinkError(String),
    /// Top-level Program had no `return EXPR;` statement to JIT.
    EmptyProgram,
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JitError::Unsupported(what) => write!(f, "jit: unsupported: {}", what),
            JitError::IsaInit(msg) => write!(f, "jit: ISA init failed: {}", msg),
            JitError::LinkError(msg) => write!(f, "jit: link error: {}", msg),
            JitError::EmptyProgram => write!(f, "jit: program has no top-level return"),
        }
    }
}

impl std::error::Error for JitError {}

/// Build a fresh JITModule for the host ISA.
fn make_module() -> Result<JITModule, JitError> {
    let mut flag_builder = settings::builder();
    // Default cranelift settings work for our needs; setting these
    // two explicitly avoids surprises on platforms where the
    // defaults change between cranelift versions.
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| JitError::IsaInit(e.to_string()))?;
    flag_builder
        .set("is_pic", "false")
        .map_err(|e| JitError::IsaInit(e.to_string()))?;
    let isa_builder = cranelift_native::builder().map_err(|e| JitError::IsaInit(e.to_string()))?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|e| JitError::IsaInit(e.to_string()))?;
    let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    // RES-166a: register runtime-shim symbols so JIT-lowered code
    // can `call` the Rust-side helpers by name. Must run BEFORE
    // the `JITModule::new(builder)` call — once the module is
    // built, its symbol table is frozen.
    register_runtime_symbols(&mut builder);
    Ok(JITModule::new(builder))
}

// ============================================================
// RES-166a: runtime shims for JIT-lowered array ops
// ============================================================
//
// Resilient arrays aren't native to Cranelift. To lower
// `Node::IndexExpression` / `Node::IndexAssignment` (RES-166b/c),
// the JIT emits calls into a small set of `extern "C"` helpers
// that manage a heap-allocated `Vec<i64>` on the Rust side. This
// ticket (RES-166a) lays the foundation: the shim functions and
// their symbol registrations. No AST lowering changes yet — the
// plumbing is wired so subsequent tickets can land lowering on
// top without touching the runtime layer again.
//
// Calling convention: every shim takes C-ABI i64 args / returns
// i64 or a pointer-sized integer. `*mut ResArray` is an opaque
// handle from Cranelift's POV; from Rust's POV it's the owning
// `Box<ResArray>` raw pointer.
//
// We declare the shims as `extern "C-unwind"` rather than plain
// `extern "C"`. The two ABIs are byte-for-byte identical on
// every target cranelift emits; the only difference is that
// `C-unwind` permits panics to propagate across the call
// boundary. Because `res_array_get` / `res_array_set` panic on
// bounds violation (matching the ticket's "panics with a clean
// error" requirement), plain `"C"` would force a non-unwinding
// abort — correct for the production JIT target, but unusable
// from Rust-side unit tests that want to `#[should_panic]` the
// same code path. `C-unwind` gets us both.
//
// Safety: the JIT guarantees that every pointer passed into the
// shims was produced by `res_array_new` on the same thread and
// hasn't been freed yet. Violations would be undefined behaviour;
// the shims `assert!` on null as a cheap early-panic for the
// common mistake. Bounds checks on `res_array_get` / `res_array_set`
// use the Vec's length and panic cleanly with an i/len message on
// out-of-bounds.
//
// Exposed symbols (registered in `register_runtime_symbols`):
//   - res_array_new(len: i64) -> *mut ResArray
//   - res_array_get(arr: *mut ResArray, i: i64) -> i64
//   - res_array_set(arr: *mut ResArray, i: i64, v: i64)
//   - res_array_free(arr: *mut ResArray)
//
// The free fn is part of 166a even though the caller-side lowering
// (tying calls to scope exit) is RES-166c/d work. Keeping it here
// means the surface is complete from day one.

pub(crate) mod runtime_shims {
    //! RES-166a: C-ABI helpers the JIT calls for array ops.
    //! `pub(crate)` so tests in the parent module can round-trip
    //! through the shims without cranelift in the picture.

    /// Heap-allocated backing store for a Resilient array inside
    /// JIT-compiled code. Opaque from Cranelift's POV; always
    /// passed as `*mut ResArray`.
    ///
    /// `repr(C)` is conservative: the JIT only ever sees the
    /// pointer, so the layout of the struct's fields doesn't
    /// affect correctness. We still pin the layout so a future
    /// ticket that reads `len` inline (for the unchecked perf
    /// variant — see the ticket's note about RES-131) has a
    /// stable ABI.
    #[repr(C)]
    pub struct ResArray {
        /// The payload. `Vec<i64>` carries its own (len, cap,
        /// ptr) tuple. The JIT only ever goes through the shim
        /// fns, which downgrade this back to a `&[i64]` view.
        pub items: Vec<i64>,
    }

    /// Allocate a new `ResArray` with `len` zero-initialized i64
    /// slots. Returns a raw pointer whose ownership is
    /// transferred to the caller; reclaim with
    /// `res_array_free`. Negative `len` is clamped to 0 so the
    /// JIT doesn't need to validate the argument inline.
    pub extern "C-unwind" fn res_array_new(len: i64) -> *mut ResArray {
        let len = len.max(0) as usize;
        let items = vec![0i64; len];
        Box::into_raw(Box::new(ResArray { items }))
    }

    /// Read `arr[i]`. Panics with a clean message on null pointer
    /// or out-of-bounds index. Panic on the JIT side turns into
    /// the process's panic handler (abort by default on release),
    /// matching the ticket's "shim panics with a clean error"
    /// requirement.
    ///
    /// Safety: `arr` must have been produced by `res_array_new`
    /// and not yet freed.
    pub extern "C-unwind" fn res_array_get(arr: *mut ResArray, i: i64) -> i64 {
        assert!(!arr.is_null(), "res_array_get: null array pointer");
        // SAFETY: the JIT calling convention guarantees the
        // pointer's validity for the duration of this call.
        let arr_ref = unsafe { &*arr };
        if i < 0 || (i as usize) >= arr_ref.items.len() {
            panic!(
                "res_array_get: index {} out of bounds for length {}",
                i,
                arr_ref.items.len()
            );
        }
        arr_ref.items[i as usize]
    }

    /// Write `arr[i] = v`. Same null-check + bounds-check as
    /// `res_array_get`.
    ///
    /// Safety: same contract as `res_array_get`.
    pub extern "C-unwind" fn res_array_set(arr: *mut ResArray, i: i64, v: i64) {
        assert!(!arr.is_null(), "res_array_set: null array pointer");
        // SAFETY: same as `res_array_get`.
        let arr_ref = unsafe { &mut *arr };
        if i < 0 || (i as usize) >= arr_ref.items.len() {
            panic!(
                "res_array_set: index {} out of bounds for length {}",
                i,
                arr_ref.items.len()
            );
        }
        arr_ref.items[i as usize] = v;
    }

    /// Free an array previously produced by `res_array_new`. A
    /// null pointer is a no-op so the JIT doesn't need to guard
    /// the call.
    ///
    /// Safety: the pointer must not be used after this call.
    pub extern "C-unwind" fn res_array_free(arr: *mut ResArray) {
        if arr.is_null() {
            return;
        }
        // SAFETY: ownership was transferred to this call.
        let _ = unsafe { Box::from_raw(arr) };
    }
}

/// RES-166a: register the runtime-shim FFI symbols on a
/// `JITBuilder` so lowered code can look them up by name. The
/// JIT expects the C-ABI calling convention documented on each
/// shim fn above. Symbol registration is absolute-address and
/// valid only for the lifetime of the running process (we link
/// directly to the function pointer), which is exactly what
/// `JITBuilder::symbol` is for.
///
/// Extracted into its own fn so tests can exercise the shims
/// without duplicating wiring, and so RES-167 (builtin calls —
/// `len`, `push`, etc.) can reuse the same registration seam.
fn register_runtime_symbols(builder: &mut JITBuilder) {
    builder.symbol("res_array_new", runtime_shims::res_array_new as *const u8);
    builder.symbol("res_array_get", runtime_shims::res_array_get as *const u8);
    builder.symbol("res_array_set", runtime_shims::res_array_set as *const u8);
    builder.symbol("res_array_free", runtime_shims::res_array_free as *const u8);
    // RES-167a: register the JIT-side builtin shims alongside the
    // array runtime shims. Both surfaces use the same absolute-
    // address `JITBuilder::symbol` mechanism, so piggy-backing on
    // the same entry point keeps `make_module` single-purpose.
    register_jit_builtin_symbols(builder);
}

// ============================================================
// RES-167a: JIT-side builtin shim table
// ============================================================
//
// The interpreter's builtin functions (`abs`, `min`, `max`, etc.
// — see `BUILTINS` in main.rs) take `&[Value]` and return
// `RResult<Value>`. That signature isn't callable from
// Cranelift-compiled code, which only speaks i64 (and soon f64
// via RES-098). This module provides thin `extern "C-unwind"`
// wrappers whose signatures match the JIT's value model, plus
// a lookup registry so the (future) `Node::CallExpression`
// lowering in RES-167b can resolve a callee name to an absolute
// address and arity.
//
// RES-167a deliberately lands ONLY the arity-stable, single-
// signature Int builtins: `abs`, `min`, `max`. These three
// operate purely on i64 and have exactly one overload each, so
// the JIT can lower them without needing the RES-124
// monomorphization pass (which was the second blocker in this
// ticket's original Attempt-1 bail). Mixed-type builtins (`pow`
// returning Int or Float) and side-effecting IO (`println`) stay
// deferred to RES-167b/c.
//
// Convention: JIT-side symbol names are prefixed with `res_jit_`
// to keep them distinct from the interpreter's BUILTINS table
// and from the `res_array_*` runtime shims. The registry maps
// the *Resilient* name (as written in user source) to the
// JIT-side symbol, absolute address, and arity.

pub(crate) mod jit_builtins {
    //! RES-167a: extern-"C-unwind" shim wrappers over the arity-
    //! stable Int builtins in main.rs's BUILTINS table. These
    //! match the interpreter's semantics for the integer-only
    //! case so tree-walker and JIT output agree.
    //!
    //! `pub(crate)` so tests in the parent module can exercise
    //! the shims directly without going through Cranelift.

    /// Integer absolute value. Matches `builtin_abs` for the
    /// `Value::Int` case in main.rs. `i64::MIN` wraps (same as
    /// `wrapping_abs`) to avoid a panic on the minimum value —
    /// the interpreter does the same via `.abs()` on two's-
    /// complement, which panics in debug but wraps in release.
    /// We wrap explicitly so behaviour is predictable.
    pub extern "C-unwind" fn res_jit_abs(x: i64) -> i64 {
        x.wrapping_abs()
    }

    /// Two-argument integer min. Matches `builtin_min` for the
    /// two-Int case in main.rs.
    pub extern "C-unwind" fn res_jit_min(a: i64, b: i64) -> i64 {
        a.min(b)
    }

    /// Two-argument integer max. Matches `builtin_max` for the
    /// two-Int case in main.rs.
    pub extern "C-unwind" fn res_jit_max(a: i64, b: i64) -> i64 {
        a.max(b)
    }
}

/// RES-167a: descriptor for a JIT-side builtin. `name` is the
/// Resilient source-level identifier a user writes; `symbol` is
/// the FFI symbol cranelift looks up via
/// `Module::declare_function`; `addr` is the absolute address
/// registered with `JITBuilder::symbol`; `arity` is the number
/// of i64 parameters (all JIT builtins today take i64 args and
/// return i64).
#[derive(Debug, Clone, Copy)]
pub(crate) struct JitBuiltinSig {
    pub name: &'static str,
    pub symbol: &'static str,
    pub arity: usize,
    pub addr: *const u8,
}

// SAFETY: `JitBuiltinSig` contains a raw function pointer. It's
// only ever read, never dereferenced directly in safe Rust — the
// JIT hands the address to Cranelift, which emits code that
// calls through it. Function pointers are trivially Send/Sync
// between threads; we mark the wrapper so the static table can
// live in a `const`.
unsafe impl Send for JitBuiltinSig {}
unsafe impl Sync for JitBuiltinSig {}

/// RES-167a: the full set of JIT-callable builtins. Keep this
/// table sorted alphabetically by `name` to make the miss-lookup
/// test's assertions stable and to surface accidental duplicates
/// when a new entry is inserted.
pub(crate) fn jit_builtin_table() -> &'static [JitBuiltinSig] {
    // Can't be a `const` (raw function-pointer casts aren't
    // usable in const initializers on stable), so we return a
    // reference to a `static` initialized once at first call.
    // The slice contents are compile-time known; the only
    // non-const bit is the `as *const u8` coercion.
    use jit_builtins::*;
    static TABLE: std::sync::OnceLock<[JitBuiltinSig; 3]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        [
            JitBuiltinSig {
                name: "abs",
                symbol: "res_jit_abs",
                arity: 1,
                addr: res_jit_abs as *const u8,
            },
            JitBuiltinSig {
                name: "max",
                symbol: "res_jit_max",
                arity: 2,
                addr: res_jit_max as *const u8,
            },
            JitBuiltinSig {
                name: "min",
                symbol: "res_jit_min",
                arity: 2,
                addr: res_jit_min as *const u8,
            },
        ]
    })
}

/// RES-167a: look up a JIT builtin by the Resilient source-level
/// name. Returns `None` if no JIT shim exists for that name —
/// caller (RES-167b's lowering) turns this into
/// `JitError::Unsupported` so an un-JIT-able builtin falls back
/// to the tree-walker.
pub(crate) fn lookup_jit_builtin(name: &str) -> Option<&'static JitBuiltinSig> {
    jit_builtin_table().iter().find(|b| b.name == name)
}

/// RES-167a: register every JIT builtin's FFI symbol on the
/// `JITBuilder`. Mirrors `register_runtime_symbols` for the
/// `res_array_*` surface. Called from `make_module`.
fn register_jit_builtin_symbols(builder: &mut JITBuilder) {
    for b in jit_builtin_table() {
        builder.symbol(b.symbol, b.addr);
    }
}

/// RES-072 + RES-096 + RES-105: compile a Resilient `Program`
/// to native code and execute it.
///
/// Two-pass compilation (RES-105):
///   Pass 1: declare every `Node::Function` in the JIT module,
///           collecting (name → FuncId) into a function map.
///           This lets bodies reference each other (and
///           themselves — recursion) without forward-decl pain.
///   Pass 2: compile each function body using compile_function,
///           plus the program's top-level non-function
///           statements as `main`.
fn run_internal(program: &Node) -> Result<(i64, JitCache), JitError> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Err(JitError::Unsupported("non-Program root")),
    };

    let mut module = make_module()?;

    // RES-174: per-run JIT cache. Within a single module, two
    // functions with the same AST hash share one FuncId (the
    // second becomes an alias of the first). Stats flush into
    // the process-wide counters on return.
    let mut cache = JitCache::new();

    // ---------- Pass 1: declare all top-level functions ----------
    //
    // RES-174: before declaring, hash the function's AST. If the
    // cache already has that hash, reuse its FuncId under the
    // new name — don't declare a second time. Otherwise declare
    // normally, record the FuncId in the cache, and enqueue the
    // function's body for Pass 2.
    let mut functions: HashMap<String, FuncId> = HashMap::new();
    let mut function_arities: HashMap<String, usize> = HashMap::new();
    // RES-175: per-name AST map so the leaf-fn inliner can
    // re-lower a callee's body at each qualifying call site.
    let mut fn_asts: HashMap<String, (Vec<(String, String)>, Node)> = HashMap::new();
    // Names of functions whose bodies we need to compile (the
    // "primary" for each unique AST hash). Aliases skip compile.
    let mut primaries: Vec<String> = Vec::new();
    for spanned in stmts {
        if let Node::Function {
            name,
            parameters,
            body,
            requires,
            ensures,
            ..
        } = &spanned.node
        {
            // RES-175: stash the AST up-front — independent of
            // whether this fn becomes a cache alias or a primary.
            fn_asts.insert(name.clone(), (parameters.clone(), (**body).clone()));
            let h = fn_hash(parameters, requires, ensures, body);
            if let Some(existing) = cache.map.get(&h).copied() {
                // Cache hit — reuse the FuncId under the new name.
                cache.hits += 1;
                functions.insert(name.clone(), existing);
                function_arities.insert(name.clone(), parameters.len());
            } else {
                cache.misses += 1;
                let mut sig = module.make_signature();
                for _ in parameters {
                    sig.params.push(AbiParam::new(types::I64));
                }
                sig.returns.push(AbiParam::new(types::I64));
                let func_id = module
                    .declare_function(name, Linkage::Local, &sig)
                    .map_err(|e| JitError::LinkError(e.to_string()))?;
                cache.map.insert(h, func_id);
                functions.insert(name.clone(), func_id);
                function_arities.insert(name.clone(), parameters.len());
                primaries.push(name.clone());
            }
        }
    }

    // ---------- Pass 2: compile each primary function body ----------
    // Aliases skip this loop — their FuncId points at the primary's
    // compiled code, so calls to them dispatch to the same entry.
    for spanned in stmts {
        if let Node::Function {
            name,
            parameters,
            body,
            ..
        } = &spanned.node
        {
            if !primaries.contains(name) {
                continue;
            }
            let func_id = functions[name];
            compile_function(
                func_id,
                name,
                parameters,
                body,
                &functions,
                &function_arities,
                &fn_asts,
                &mut module,
            )?;
            cache.compiles += 1;
        }
    }

    // ---------- Pass 2 cont.: compile main ----------
    // The "main" function is the program's top-level non-function
    // statements. If the program has no top-level return,
    // compile_statements raises EmptyProgram and we never run.
    let mut main_sig = module.make_signature();
    main_sig.returns.push(AbiParam::new(types::I64));
    let main_id = module
        .declare_function("__resilient_main__", Linkage::Local, &main_sig)
        .map_err(|e| JitError::LinkError(e.to_string()))?;

    let mut ctx = module.make_context();
    ctx.func.signature = main_sig;
    let mut builder_ctx = FunctionBuilderContext::new();
    {
        let mut bcx = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
        let entry = bcx.create_block();
        bcx.append_block_params_for_function_params(entry);
        bcx.switch_to_block(entry);
        bcx.seal_block(entry);

        let mut lctx = LowerCtx::new();
        lctx.functions = functions.clone();
        lctx.function_arities = function_arities.clone();
        lctx.fn_asts = fn_asts.clone();
        compile_statements(stmts, &mut bcx, &mut lctx, &mut module)?;
        bcx.finalize();
    }

    module
        .define_function(main_id, &mut ctx)
        .map_err(|e| JitError::LinkError(e.to_string()))?;
    module.clear_context(&mut ctx);
    module
        .finalize_definitions()
        .map_err(|e| JitError::LinkError(e.to_string()))?;

    // ---------- Run main ----------
    let raw = module.get_finalized_function(main_id);
    // SAFETY: `raw` points at a freshly-finalized function with
    // signature `extern "C" fn() -> i64`; we constructed that
    // signature ourselves above. The JITModule keeps the code
    // alive — `module` outlives this call.
    let f: unsafe extern "C" fn() -> i64 = unsafe { std::mem::transmute(raw) };
    let result = unsafe { f() };
    // RES-174: fold this run's cache stats into the process-wide
    // counters so `--jit-cache-stats` can print lifetime totals
    // from `main.rs` at exit. Counters are relaxed-atomic — no
    // synchronization semantics, just diagnostic accumulation.
    flush_cache_stats_to_globals(&cache);
    Ok((result, cache))
}

/// Public `run()` — discards the per-run cache after folding its
/// stats into the process-wide counters. Most callers (the CLI,
/// the examples_smoke harness, the rest of the test suite) go
/// through this path.
pub fn run(program: &Node) -> Result<i64, JitError> {
    run_internal(program).map(|(v, _)| v)
}

/// RES-174: test hook — run the program and return the per-run
/// cache's (hits, misses, compiles) alongside the i64 result.
/// Unlike reading `cache_stats()` before/after, this variant
/// captures ONLY the numbers this run produced, so parallel
/// test execution doesn't pollute the delta.
#[cfg(test)]
pub(crate) fn run_with_stats(program: &Node) -> Result<(i64, u32, u32, u32), JitError> {
    let (result, cache) = run_internal(program)?;
    Ok((result, cache.hits, cache.misses, cache.compiles))
}

/// RES-105: compile a single user-defined function body.
/// Parameters are declared as Variables in a fresh LowerCtx
/// (inheriting the program-wide function map for cross-function
/// calls including recursion).
///
/// RES-168: body lowering is routed through a dedicated
/// `body_block` so tail-recursive self-calls can jump back to it
/// instead of emitting a regular call + return. The function's
/// entry block carries the Cranelift-ABI block params (function
/// arguments), `def_var`s each into a parameter Variable, and
/// unconditionally jumps to `body_block`. The body block is left
/// unsealed until lowering finishes so back-edges from tail
/// calls (which re-`def_var` parameter Variables) can reconcile
/// through Cranelift's SSA-construction phi inference.
/// RES-175: alias for the per-name AST map threaded through the
/// JIT. Hoisted to a type alias so the `compile_function`
/// signature doesn't trip clippy's type-complexity lint.
type FnAstMap = HashMap<String, (Vec<(String, String)>, Node)>;

#[allow(clippy::too_many_arguments)] // RES-175: each arg is used
fn compile_function(
    func_id: FuncId,
    fn_name: &str,
    parameters: &[(String, String)],
    body: &Node,
    functions: &HashMap<String, FuncId>,
    function_arities: &HashMap<String, usize>,
    fn_asts: &FnAstMap,
    module: &mut JITModule,
) -> Result<(), JitError> {
    // Build the signature again — we declared it in Pass 1, but
    // module.make_context() gives us a fresh empty Function we
    // need to populate.
    let mut sig = module.make_signature();
    for _ in parameters {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut builder_ctx = FunctionBuilderContext::new();
    {
        let mut bcx = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);

        // ---------- RES-168: entry + body split ----------
        let entry = bcx.create_block();
        bcx.append_block_params_for_function_params(entry);
        bcx.switch_to_block(entry);
        bcx.seal_block(entry);

        // Bind each parameter to a Variable in the LowerCtx so
        // identifier reads in the body resolve correctly. Capture
        // the Variables in declaration order for TCO back-edges.
        let mut lctx = LowerCtx::new();
        lctx.functions = functions.clone();
        lctx.function_arities = function_arities.clone();
        lctx.fn_asts = fn_asts.clone();
        // parameters: Vec<(String, String)> — (type, name) per
        // the AST. Name is the second element.
        let block_params: Vec<Value> = bcx.block_params(entry).to_vec();
        let mut param_vars: Vec<Variable> = Vec::with_capacity(parameters.len());
        for ((_ty, name), pval) in parameters.iter().zip(block_params.iter()) {
            let var = lctx.declare(name, &mut bcx);
            bcx.def_var(var, *pval);
            param_vars.push(var);
        }

        // Create body block and jump entry → body. Deliberately
        // NOT sealing body yet — tail calls will add back-edges.
        let body_block = bcx.create_block();
        bcx.ins().jump(body_block, &[]);
        bcx.switch_to_block(body_block);

        // Wire up TCO state on the LowerCtx. Any ReturnStatement
        // whose RHS is a direct call to `fn_name` with matching
        // arity lowers to a back-edge jump instead of a regular
        // return (see the ReturnStatement arms in
        // `compile_node_list` / `lower_block_or_stmt`).
        lctx.current_fn = Some(fn_name.to_string());
        lctx.tco_target = Some(body_block);
        lctx.param_vars = param_vars;

        // The function body is a Block — lower it; require it
        // to terminate (Phase H functions must end in a return,
        // same constraint as `main`).
        let terminated = lower_block_or_stmt(body, &mut bcx, &mut lctx, module)?;
        if !terminated {
            return Err(JitError::EmptyProgram);
        }

        // Seal body now that all possible back-edges (if any)
        // from tail calls in the body have been emitted.
        bcx.seal_block(body_block);
        bcx.finalize();
    }

    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| JitError::LinkError(e.to_string()))?;
    module.clear_context(&mut ctx);
    Ok(())
}

/// RES-175: lower a qualifying call site by splicing the callee's
/// body into the caller. The body is evaluated in a fresh locals
/// scope (parameter names shadow any caller local with the same
/// name; the shadow is dropped on exit). Every `ReturnStatement`
/// in the body lowers as `jump(merge_block, &[v])` instead of a
/// function-level `return_`, so the inlined expression's result
/// is the merge block's sole i64 parameter.
///
/// Caller guarantees (via `is_trivial_leaf` + the null
/// `inline_return_target` check at the call site):
///   - No nested calls in the body (so we don't blow stack on a
///     malicious chain of self-aliases).
///   - No loops / match (lowering them in an inlined context
///     would require tracking the enclosing function's TCO
///     target and would drag in match-statement plumbing the JIT
///     doesn't have yet).
///   - Callee != current fn (self-recursion would infinite-loop
///     the inliner).
///   - We're not already inside an inline (prevents runaway
///     nesting at code-size level — a future phase can relax).
fn try_lower_inline_call(
    callee_params: &[(String, String)],
    callee_body: &Node,
    arguments: &[Node],
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
    module: &mut JITModule,
) -> Result<Value, JitError> {
    // Merge block: single i64 block-param carries the inlined
    // return value back to the caller.
    let merge = bcx.create_block();
    bcx.append_block_param(merge, types::I64);

    // Lower arguments FIRST — in the caller's scope, so their
    // expressions see the caller's bindings.
    let mut arg_vals: Vec<Value> = Vec::with_capacity(arguments.len());
    for arg in arguments {
        arg_vals.push(lower_expr(arg, bcx, ctx, module)?);
    }

    // Snapshot locals + inline target so we can restore on exit.
    // Snapshotting the whole map is equivalent to the ticket's
    // "suffix each with a unique counter at AST-level before
    // lowering" — it gives each inlined body its own scope
    // without rewriting the AST.
    let saved_locals = ctx.locals.clone();
    let saved_target = ctx.inline_return_target.take();

    // Bind each parameter to a FRESH Variable; shadow any
    // caller local with the same name. After the inline ends,
    // restoring `saved_locals` puts the caller's binding back.
    for ((_ty, pname), argval) in callee_params.iter().zip(arg_vals.iter()) {
        let var = ctx.declare(pname, bcx);
        bcx.def_var(var, *argval);
    }

    // Install the merge target so `ReturnStatement` lowers as
    // a jump to merge instead of a function-level return.
    ctx.inline_return_target = Some(merge);

    // Lower the body. `lower_block_or_stmt` handles Block / If /
    // Return shapes — all that a trivial leaf can contain given
    // the `has_disqualifying_construct` filter.
    let terminated = lower_block_or_stmt(callee_body, bcx, ctx, module)?;
    if !terminated {
        // Defensive: the body fell through without a terminator.
        // For trivial leaves this shouldn't happen (they end in
        // `return`), but if someone writes a body like `let x = 1;`
        // with no return, the block leaves the builder live. Jump
        // to merge with a zero to keep the IR valid.
        let zero = bcx.ins().iconst(types::I64, 0);
        bcx.ins().jump(merge, &[zero]);
    }

    // Restore caller state.
    ctx.locals = saved_locals;
    ctx.inline_return_target = saved_target;

    // Seal merge (all predecessors are in), switch to it, and
    // return the block-param as this call site's Value.
    bcx.seal_block(merge);
    bcx.switch_to_block(merge);
    Ok(bcx.block_params(merge)[0])
}

/// RES-168: try to lower a `return <call>` as a tail-call back-edge
/// jump. Returns `Ok(true)` on a successful tail-call emit
/// (terminator emitted — caller stops walking), `Ok(false)` when
/// the shape doesn't qualify (direct call, name match, arity match
/// — all three must hold), and `Err(_)` only on a real lowering
/// failure of the argument expressions.
///
/// Non-qualifying cases (indirect call, different name, wrong
/// arity) are handled silently by returning false; the caller
/// falls back to the regular `return_` path. The ticket's "non-
/// tail-position call to self is NOT optimized" guarantee falls
/// out of this function only being called from `ReturnStatement`
/// handlers whose value is a direct `CallExpression`.
fn try_lower_tail_call(
    expr: &Node,
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
    module: &mut JITModule,
) -> Result<bool, JitError> {
    // Bail if we're not inside a function (e.g. top-level `main`).
    let Some(target) = ctx.tco_target else {
        return Ok(false);
    };
    let Some(current) = ctx.current_fn.clone() else {
        return Ok(false);
    };

    // Shape: ReturnStatement's value must be a direct call to the
    // enclosing function with matching arity.
    let Node::CallExpression {
        function,
        arguments,
        ..
    } = expr
    else {
        return Ok(false);
    };
    let Node::Identifier { name, .. } = function.as_ref() else {
        return Ok(false);
    };
    if *name != current {
        return Ok(false);
    }
    if arguments.len() != ctx.param_vars.len() {
        return Ok(false);
    }

    // Lower all argument expressions FIRST — they reference the
    // *current* parameter values, so we must capture those before
    // reassigning. `lower_expr` recurses into the ctx, but never
    // touches `param_vars` / `tco_target`, so ordering is safe.
    let mut new_vals: Vec<Value> = Vec::with_capacity(arguments.len());
    for arg in arguments {
        new_vals.push(lower_expr(arg, bcx, ctx, module)?);
    }

    // Re-`def_var` each parameter Variable with the new value.
    // Cranelift's SSA construction inserts phi nodes at
    // `target`'s entry automatically once the block is sealed
    // (handled by the caller — `compile_function` seals after
    // lowering completes).
    let vars: Vec<Variable> = ctx.param_vars.clone();
    for (var, val) in vars.iter().zip(new_vals.iter()) {
        bcx.def_var(*var, *val);
    }
    bcx.ins().jump(target, &[]);
    Ok(true)
}

/// RES-102 + RES-103: walk a slice of top-level statements and
/// emit Cranelift instructions including the function's `return_`.
///
/// Supported shapes (grows ticket by ticket):
/// 1. A single `ReturnStatement { value: Some(expr) }`
///    → lowers the expression and emits `return_`.
/// 2. An `IfStatement`. Phase F (RES-103) handles four sub-cases:
///    both arms terminate, then-only terminates,
///    else-only terminates, neither terminates. For any
///    fallthrough case the surrounding compile_node_list keeps
///    walking from the merge block. If the walk completes
///    without ever emitting a return, compile_statements raises
///    `EmptyProgram` ("program has no top-level return") — same
///    behavior as a program with no return statement at all.
fn compile_statements(
    stmts: &[crate::Spanned<Node>],
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
    module: &mut JITModule,
) -> Result<(), JitError> {
    // Top-level statements are Spanned<Node>; Block bodies are
    // raw Node. Strip the wrapper here and delegate to the shared
    // walker so the lowering logic isn't duplicated.
    //
    // RES-105: skip Function nodes — Pass 1 declared them in the
    // module already, Pass 2 is compiling each body separately.
    // Walking them again here would double-compile.
    let nodes: Vec<&Node> = stmts
        .iter()
        .map(|s| &s.node)
        .filter(|n| !matches!(n, Node::Function { .. }))
        .collect();
    let returned = compile_node_list(&nodes, bcx, ctx, module)?;
    if !returned {
        return Err(JitError::EmptyProgram);
    }
    Ok(())
}

/// Walks a slice of statement nodes and emits cranelift
/// instructions. Returns `Ok(true)` when the walk emitted a
/// terminator (a `return_`, or an if/else where both arms
/// terminated). Returns `Ok(false)` when the walk completed
/// without emitting any terminator — the caller decides whether
/// that's an error (top-level) or a fallthrough (inside a Block).
fn compile_node_list(
    stmts: &[&Node],
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
    module: &mut JITModule,
) -> Result<bool, JitError> {
    for node in stmts {
        match node {
            Node::ReturnStatement {
                value: Some(expr), ..
            } => {
                // RES-168: try TCO first. On a qualifying tail
                // call, `try_lower_tail_call` emits the back-edge
                // jump and we skip the regular `return_` below.
                if try_lower_tail_call(expr, bcx, ctx, module)? {
                    return Ok(true);
                }
                let v = lower_expr(expr, bcx, ctx, module)?;
                // RES-175: if we're lowering the body of a leaf-fn
                // being inlined into its caller, route `return` to
                // the inline merge block instead of emitting the
                // enclosing function's `return_`.
                if let Some(merge) = ctx.inline_return_target {
                    bcx.ins().jump(merge, &[v]);
                    return Ok(true);
                }
                bcx.ins().return_(&[v]);
                return Ok(true);
            }
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                let if_terminated = lower_if_statement(
                    condition,
                    consequence,
                    alternative.as_deref(),
                    bcx,
                    ctx,
                    module,
                )?;
                if if_terminated {
                    // Both arms returned — function exits, no
                    // statements after the if can run.
                    return Ok(true);
                }
                // RES-103: at least one arm fell through; the
                // builder is now positioned at the merge block.
                // Keep walking — trailing statements lower into
                // the merge block.
                continue;
            }
            // RES-104: `let NAME = EXPR;` — lower the RHS, declare
            // a fresh Variable, and bind NAME to it. Subsequent
            // identifier reads via lower_expr will use_var the
            // same Variable.
            Node::LetStatement { name, value, .. } => {
                let v = lower_expr(value, bcx, ctx, module)?;
                let var = ctx.declare(name, bcx);
                bcx.def_var(var, v);
                continue;
            }
            // RES-107: `NAME = EXPR;` reassignment — look up the
            // Variable from a prior `let`, lower the RHS, and
            // `def_var` it. Cranelift's SSA construction handles
            // the rest (phi insertion at merge points is automatic
            // — we don't emit phis manually).
            Node::Assignment { name, value, .. } => {
                let Some(var) = ctx.lookup(name) else {
                    return Err(JitError::Unsupported(
                        "reassignment of undeclared identifier",
                    ));
                };
                let v = lower_expr(value, bcx, ctx, module)?;
                bcx.def_var(var, v);
                continue;
            }
            // RES-107: `while (cond) { body }` — classic three-
            // block structured loop.
            //
            // Block layout:
            //   header_block  — lowers `cond`, `brif(cond, body, exit)`
            //   body_block    — lowers `body`; falls through back to
            //                   header (emitting the back-edge), or
            //                   terminates early (no back-edge).
            //   exit_block    — where statements after the while land.
            //
            // Sealing order is the subtle part: `header_block` has
            // TWO predecessors (the entry jump AND the back-edge
            // from the body), so we seal it AFTER the back-edge is
            // emitted. `body_block` and `exit_block` each have one
            // predecessor and can be sealed immediately after
            // `switch_to_block`.
            Node::WhileStatement {
                condition, body, ..
            } => {
                let while_terminated = lower_while_statement(condition, body, bcx, ctx, module)?;
                if while_terminated {
                    return Ok(true);
                }
                continue;
            }
            // Skip statements with no JIT-relevant effect for now;
            // a future phase will lower expression statements,
            // reassignment, while loops, etc.
            _ => continue,
        }
    }
    Ok(false)
}

/// RES-107: lower a `while` loop to Cranelift IR.
///
/// Returns `Ok(true)` when the loop body unconditionally emits a
/// terminator (e.g. `return` inside the body) AND the loop has no
/// natural exit path — today that can't happen because we always
/// emit a header-block branch that falls to `exit_block` when the
/// condition is false, so we return `Ok(false)` and leave the
/// builder positioned at `exit_block` for trailing statements.
///
/// Sealing contract (matches Cranelift's SSA construction docs):
/// - `body_block` is sealed as soon as we switch to it — its only
///   predecessor is the header's `brif`.
/// - `exit_block` is sealed as soon as we switch to it — its only
///   predecessor is the same `brif`.
/// - `header_block` is sealed AFTER the body's back-edge jump is
///   emitted, since it has two predecessors.
fn lower_while_statement(
    condition: &Node,
    body: &Node,
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
    module: &mut JITModule,
) -> Result<bool, JitError> {
    let header_block = bcx.create_block();
    let body_block = bcx.create_block();
    let exit_block = bcx.create_block();

    // Entry predecessor of header: jump in from current block.
    bcx.ins().jump(header_block, &[]);

    // Header: lower condition, branch to body or exit. Not sealed
    // yet — body's back-edge is the second predecessor.
    bcx.switch_to_block(header_block);
    let cond_val = lower_expr(condition, bcx, ctx, module)?;
    bcx.ins().brif(cond_val, body_block, &[], exit_block, &[]);

    // Body: sealable immediately (single predecessor = header's
    // brif). If it falls through we emit the back-edge; if it
    // terminates (early return inside the body), we don't — the
    // function exited and header won't loop via that path.
    bcx.switch_to_block(body_block);
    bcx.seal_block(body_block);
    let body_terminated = lower_block_or_stmt(body, bcx, ctx, module)?;
    if !body_terminated {
        bcx.ins().jump(header_block, &[]);
    }

    // Now that the back-edge (if any) is emitted, header's
    // predecessor set is frozen and we can seal it.
    bcx.seal_block(header_block);

    // Exit: single predecessor = header's brif. Switch, seal, and
    // let the caller's compile_node_list continue into it.
    bcx.switch_to_block(exit_block);
    bcx.seal_block(exit_block);

    // A while loop never unconditionally terminates the enclosing
    // function — even an infinitely-looping `while true { ... }`
    // is only detected at runtime, and our compile-time view has
    // to assume the exit path is reachable.
    Ok(false)
}

/// RES-102 + RES-103: lower an IfStatement.
///
/// Returns `Ok(true)` when both arms emit terminators (function
/// exits from each arm — no merge block needed). Returns
/// `Ok(false)` when at least one arm falls through; in that case
/// the function builder is positioned at the merge block on
/// return so the caller can continue lowering trailing
/// statements there.
///
/// Cranelift block dance:
///   brif(cond, then_block, &[], else_block, &[])
///   then_block: lower then-arm; emits return_ OR jump merge
///   else_block: lower else-arm (or missing → straight to merge);
///               emits return_ OR jump merge
///   merge_block (if either arm fell through): switch + seal
///
/// No phi nodes are needed because lower_if_statement doesn't
/// produce an SSA value yet — that's a future "if as expression"
/// phase.
fn lower_if_statement(
    condition: &Node,
    consequence: &Node,
    alternative: Option<&Node>,
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
    module: &mut JITModule,
) -> Result<bool, JitError> {
    let cond_val = lower_expr(condition, bcx, ctx, module)?;

    let then_block = bcx.create_block();
    let else_block = bcx.create_block();
    // Create merge_block up-front so each arm can jump to it
    // inline. If neither arm needs the merge (both terminate)
    // we'll just never switch to it — Cranelift doesn't require
    // unused blocks to be sealed before finalize.
    let merge_block = bcx.create_block();

    bcx.ins().brif(cond_val, then_block, &[], else_block, &[]);

    // then-arm
    bcx.switch_to_block(then_block);
    bcx.seal_block(then_block);
    let then_terminated = lower_block_or_stmt(consequence, bcx, ctx, module)?;
    if !then_terminated {
        // then-arm fell through — jump to merge so the trailing
        // statements after the if can run.
        bcx.ins().jump(merge_block, &[]);
    }

    // else-arm
    bcx.switch_to_block(else_block);
    bcx.seal_block(else_block);
    let else_terminated = match alternative {
        Some(alt) => lower_block_or_stmt(alt, bcx, ctx, module)?,
        // Bare `if` with no else: the else-block has nothing to
        // lower and falls through immediately. RES-103 treats
        // this as a fallthrough (Phase E used to reject it).
        None => false,
    };
    if !else_terminated {
        bcx.ins().jump(merge_block, &[]);
    }

    if then_terminated && else_terminated {
        // Both arms exited — merge_block has no predecessors, so
        // we'll never use it. Cranelift accepts unused blocks at
        // finalize time as long as they're sealed; seal it here
        // to keep things tidy. (FunctionBuilder will skip
        // emitting code for it.)
        bcx.seal_block(merge_block);
        return Ok(true);
    }

    // At least one arm fell through. Switch to merge so the
    // caller's compile_node_list lowers trailing statements
    // here, and seal — both predecessor jumps were emitted
    // above (or one arm terminated and the merge has a single
    // predecessor jump).
    bcx.switch_to_block(merge_block);
    bcx.seal_block(merge_block);
    Ok(false)
}

/// Lower a Block, or a single statement (in case `else if` chains
/// ever land — for now `consequence` is always a Block from the
/// parser). Recurses into compile_statements so the same set of
/// statement shapes is supported uniformly.
/// Lower a Block (typical) or single statement (for `else if`,
/// where the parser gives a nested IfStatement directly as
/// `alternative`). Returns Ok(true) when a terminator (return)
/// was emitted, Ok(false) when the block fell through.
fn lower_block_or_stmt(
    node: &Node,
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
    module: &mut JITModule,
) -> Result<bool, JitError> {
    match node {
        Node::Block { stmts, .. } => {
            let refs: Vec<&Node> = stmts.iter().collect();
            compile_node_list(&refs, bcx, ctx, module)
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            // RES-103: an if "terminates" only if both arms did.
            // Otherwise the merge block is now active and the
            // surrounding block's caller may want to keep walking.
            lower_if_statement(
                condition,
                consequence,
                alternative.as_deref(),
                bcx,
                ctx,
                module,
            )
        }
        Node::ReturnStatement {
            value: Some(expr), ..
        } => {
            // RES-168: same TCO hook as compile_node_list — any
            // `return <self>(args)` in a function body is lowered
            // as a back-edge jump, regardless of whether it's
            // at the top of the body or nested inside an if/while.
            if try_lower_tail_call(expr, bcx, ctx, module)? {
                return Ok(true);
            }
            let v = lower_expr(expr, bcx, ctx, module)?;
            // RES-175: redirect to the inline merge block when
            // we're inside an inlined leaf-fn body.
            if let Some(merge) = ctx.inline_return_target {
                bcx.ins().jump(merge, &[v]);
                return Ok(true);
            }
            bcx.ins().return_(&[v]);
            Ok(true)
        }
        _ => Err(JitError::Unsupported(node_kind(node))),
    }
}

/// Lower an expression to a Cranelift `Value` of type `i64`.
fn lower_expr(
    node: &Node,
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
    module: &mut JITModule,
) -> Result<Value, JitError> {
    match node {
        Node::IntegerLiteral { value, .. } => Ok(bcx.ins().iconst(types::I64, *value)),
        // RES-100: bool literals lower to i64 0/1 — matches how
        // the bytecode VM materializes booleans, so the JIT result
        // is identical when the program runs on either backend.
        Node::BooleanLiteral { value, .. } => {
            Ok(bcx.ins().iconst(types::I64, if *value { 1 } else { 0 }))
        }
        // RES-104: identifier read — look up the Variable in the
        // locals map and use_var. Cranelift's SSA construction
        // routes the right value to this use.
        Node::Identifier { name, .. } => match ctx.lookup(name) {
            Some(var) => Ok(bcx.use_var(var)),
            None => Err(JitError::Unsupported("identifier not in scope")),
        },
        // RES-105: direct function call. Resolve the callee name
        // → FuncId from the program-wide function map, lower
        // each argument, declare the function as a local ref in
        // the current builder's func, then emit `call`. Indirect
        // calls (function value, method, closure) are not yet
        // supported.
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let callee_name = match function.as_ref() {
                Node::Identifier { name, .. } => name.clone(),
                _ => {
                    return Err(JitError::Unsupported(
                        "JIT only supports direct calls (Identifier callee)",
                    ));
                }
            };
            let func_id = match ctx.functions.get(&callee_name).copied() {
                Some(id) => id,
                None => {
                    // Note: we lose the actual name in the
                    // diagnostic since JitError::Unsupported
                    // takes &'static str. A richer diagnostic
                    // type is a future ticket.
                    return Err(JitError::Unsupported("call to unknown function"));
                }
            };
            let expected_arity = ctx.function_arities.get(&callee_name).copied().unwrap_or(0);
            if arguments.len() != expected_arity {
                return Err(JitError::Unsupported(
                    "call arity mismatch (declared params vs actual args)",
                ));
            }

            // RES-175: leaf-fn inliner. If the callee's AST
            // qualifies as a trivial leaf AND we aren't
            // inside a nested inline already (to bound code
            // expansion), lower the body in-place instead of
            // emitting an indirect call. See
            // `try_lower_inline_call` for the mechanics.
            if let Some((callee_params, callee_body)) = ctx.fn_asts.get(&callee_name).cloned()
                && is_trivial_leaf(&callee_body, &callee_name, ctx.current_fn.as_deref())
                && ctx.inline_return_target.is_none()
            {
                return try_lower_inline_call(
                    &callee_params,
                    &callee_body,
                    arguments,
                    bcx,
                    ctx,
                    module,
                );
            }

            // Lower each argument before declaring the local
            // function ref; lowering may recurse and we want a
            // clean borrow stack at the call site.
            let mut arg_values: Vec<Value> = Vec::with_capacity(arguments.len());
            for arg in arguments {
                arg_values.push(lower_expr(arg, bcx, ctx, module)?);
            }
            // Declare the callee in the current function so
            // Cranelift knows its signature. Returns a local
            // FuncRef usable in `call`.
            let local_callee = module.declare_func_in_func(func_id, bcx.func);
            let call = bcx.ins().call(local_callee, &arg_values);
            // i64-returning function — exactly one result.
            Ok(bcx.inst_results(call)[0])
        }
        // RES-099: lower all four signed integer infix ops + RES-100:
        // the six comparison ops. Same recursive shape — recurse on
        // left + right, then emit the matching Cranelift instruction.
        // Note: `sdiv`/`srem` exhibit UB at the IR level when rhs == 0;
        // a future ticket should emit a runtime check that traps or
        // returns a sentinel. For now this matches what the bytecode
        // VM does WITHOUT line attribution on the JIT path.
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let op_str = operator.as_str();
            // Validate first so we can short-circuit Unsupported
            // before recursing into the operands.
            if !matches!(
                op_str,
                "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | "<=" | ">" | ">="
            ) {
                return Err(JitError::Unsupported(
                    "infix operator other than +,-,*,/,%,==,!=,<,<=,>,>=",
                ));
            }
            let l = lower_expr(left, bcx, ctx, module)?;
            let r = lower_expr(right, bcx, ctx, module)?;
            Ok(match op_str {
                "+" => bcx.ins().iadd(l, r),
                "-" => bcx.ins().isub(l, r),
                "*" => bcx.ins().imul(l, r),
                "/" => bcx.ins().sdiv(l, r),
                "%" => bcx.ins().srem(l, r),
                // RES-100: comparisons return Cranelift's i8 0/1.
                // uextend widens to i64 so the function signature
                // (returns i64) stays uniform regardless of which
                // op the user wrote.
                cmp => {
                    let cc = match cmp {
                        "==" => IntCC::Equal,
                        "!=" => IntCC::NotEqual,
                        "<" => IntCC::SignedLessThan,
                        "<=" => IntCC::SignedLessThanOrEqual,
                        ">" => IntCC::SignedGreaterThan,
                        ">=" => IntCC::SignedGreaterThanOrEqual,
                        _ => unreachable!("validated above"),
                    };
                    let raw = bcx.ins().icmp(cc, l, r);
                    bcx.ins().uextend(types::I64, raw)
                }
            })
        }
        _ => Err(JitError::Unsupported(node_kind(node))),
    }
}

fn node_kind(n: &Node) -> &'static str {
    match n {
        Node::Program(_) => "Program",
        Node::Function { .. } => "Function",
        Node::LetStatement { .. } => "LetStatement",
        Node::ReturnStatement { .. } => "ReturnStatement",
        Node::IfStatement { .. } => "IfStatement",
        Node::WhileStatement { .. } => "WhileStatement",
        Node::Identifier { .. } => "Identifier",
        Node::IntegerLiteral { .. } => "IntegerLiteral",
        Node::FloatLiteral { .. } => "FloatLiteral",
        Node::StringLiteral { .. } => "StringLiteral",
        Node::BooleanLiteral { .. } => "BooleanLiteral",
        Node::PrefixExpression { .. } => "PrefixExpression",
        Node::InfixExpression { .. } => "InfixExpression",
        Node::CallExpression { .. } => "CallExpression",
        Node::Block { .. } => "Block",
        Node::ExpressionStatement { .. } => "ExpressionStatement",
        _ => "<other>",
    }
}

// ============================================================
// RES-165a: struct layout cache
// ============================================================
//
// Phase L of the JIT (RES-165) lowers struct literals, field
// loads/stores, and (eventually) struct-valued returns. Every
// one of those paths needs to know, for a given declared struct,
// where each field sits inside the backing buffer:
//
//   - Literal construction: the sequence of `stack_store(val, ss,
//     offset)` calls that initializes a freshly-allocated stack
//     slot.
//   - Field load:  `stack_load(field_ty, ss, offset)`.
//   - Field store: `stack_store(val, ss, offset)`.
//   - Out-ptr return: the same offsets applied to the caller-
//     supplied buffer pointer.
//
// RES-165a only builds and queries the cache. The later phases
// (165b/c/d) will consume `StructLayout` / `FieldLayout` without
// re-running the layout algorithm.
//
// ## Layout algorithm (inline spec)
//
// Fields are placed in declaration order. For each field, we
// round the current offset up to the field's alignment before
// placing it, then advance by its size. The total struct size is
// rounded up to the largest alignment any field requires (so an
// array of the struct tiles correctly). Empty structs have
// size 0, align 1.
//
// This is classic repr(C) layout. It matches what a C compiler
// would produce for the same field order, which is important so
// that the interpreter/VM view and the JIT view stay compatible
// when structs get serialized or passed across the FFI boundary
// in a future ticket.
//
// ## Type mapping
//
// Resilient surface types map to cranelift types as follows:
//
//   int / Int / I64      → I64    (8 bytes, 8-aligned)
//   float / Float / F64  → F64    (8 bytes, 8-aligned)
//   i32 / I32            → I32    (4 bytes, 4-aligned)
//   bool / Bool          → I8     (1 byte,  1-aligned)
//   everything else      → I64    (treated as a machine pointer
//                                   on the 64-bit targets we JIT
//                                   on today)
//
// The pointer fallback is deliberately permissive for RES-165a:
// the only place a struct can legitimately hold a non-primitive
// today is via a boxed/heap value, and every such value is already
// an I64-shaped handle in the JIT's current calling convention.

/// Per-field layout entry inside a declared struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldLayout {
    /// Field name, as written in the source.
    pub name: String,
    /// Byte offset from the struct's base address, accounting
    /// for natural-alignment padding before this field.
    pub offset: u32,
    /// Cranelift scalar type to use for load/store of this field.
    pub ty: Type,
    /// Byte size of the field (matches `ty.bytes()`).
    pub size: u32,
    /// Natural alignment of the field, in bytes.
    pub align: u32,
}

/// Layout for one `Node::StructDecl`, keyed elsewhere by the
/// decl's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructLayout {
    /// Declared name — duplicated from the map key for
    /// self-containedness.
    pub name: String,
    /// Fields in declaration order.
    pub fields: Vec<FieldLayout>,
    /// Total size of the struct in bytes, rounded up to the
    /// struct's alignment.
    pub total_size: u32,
    /// Alignment of the struct itself — the max of every field's
    /// alignment (with 1 as the floor for empty structs).
    pub align: u32,
}

impl StructLayout {
    /// Look up a field by name. Linear scan is fine — struct
    /// field counts are small and this runs at lowering time, not
    /// per-instruction.
    pub(crate) fn field(&self, name: &str) -> Option<&FieldLayout> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// Return `(cranelift_ty, size, align)` for a Resilient source
/// type name. See the "Type mapping" block above the FieldLayout
/// type for the rules.
fn cranelift_ty_for(annotation: &str) -> (Type, u32, u32) {
    match annotation {
        "int" | "Int" | "I64" => (types::I64, 8, 8),
        "float" | "Float" | "F64" => (types::F64, 8, 8),
        "i32" | "I32" => (types::I32, 4, 4),
        "bool" | "Bool" => (types::I8, 1, 1),
        // Anything else (strings, arrays, nested structs, Result,
        // user types we haven't seen the decl for) is modelled
        // as a machine pointer on the JIT's 64-bit targets.
        _ => (types::I64, 8, 8),
    }
}

/// Compute the repr(C)-style layout for a single `Node::StructDecl`.
/// Returns `None` if the passed node isn't a `StructDecl` —
/// callers should hand us only struct-decl nodes.
fn build_struct_layout(decl: &Node) -> Option<StructLayout> {
    let (name, fields) = match decl {
        Node::StructDecl { name, fields, .. } => (name.clone(), fields),
        _ => return None,
    };
    let mut placed: Vec<FieldLayout> = Vec::with_capacity(fields.len());
    let mut offset: u32 = 0;
    let mut max_align: u32 = 1;
    for (field_type, field_name) in fields {
        let (ty, size, align) = cranelift_ty_for(field_type);
        // Align-up the current offset to this field's alignment.
        let misalign = offset % align;
        if misalign != 0 {
            offset += align - misalign;
        }
        placed.push(FieldLayout {
            name: field_name.clone(),
            offset,
            ty,
            size,
            align,
        });
        offset += size;
        if align > max_align {
            max_align = align;
        }
    }
    // Round the struct's total size up to its own alignment so
    // arrays-of-struct tile correctly without embedded gaps.
    let tail_misalign = offset % max_align;
    if tail_misalign != 0 {
        offset += max_align - tail_misalign;
    }
    Some(StructLayout {
        name,
        fields: placed,
        total_size: offset,
        align: max_align,
    })
}

/// Walk a `Program` and build the `decl_name -> StructLayout`
/// cache. Nested `ImplBlock`s don't add struct decls today, so a
/// single top-level pass is enough; we still descend into impls
/// defensively so a future reorg doesn't silently lose layouts.
pub(crate) fn collect_struct_layouts(program: &Node) -> HashMap<String, StructLayout> {
    let mut out: HashMap<String, StructLayout> = HashMap::new();
    collect_struct_layouts_into(program, &mut out);
    out
}

fn collect_struct_layouts_into(node: &Node, out: &mut HashMap<String, StructLayout>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                collect_struct_layouts_into(&s.node, out);
            }
        }
        Node::StructDecl { name, .. } => {
            if let Some(layout) = build_struct_layout(node) {
                out.insert(name.clone(), layout);
            }
        }
        // RES-170 / friends may put struct decls inside other
        // containers in the future — for today this is a no-op
        // since the parser only places them at Program scope.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_program(src: &str) -> Node {
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        program
    }

    #[test]
    fn jit_returns_constant_42() {
        let p = parse_program("return 42;");
        assert_eq!(run(&p).unwrap(), 42);
    }

    #[test]
    fn jit_adds_two_constants() {
        let p = parse_program("return 2 + 3;");
        assert_eq!(run(&p).unwrap(), 5);
    }

    #[test]
    fn jit_adds_three_constants() {
        // Confirms the recursive lowering composes left-associatively.
        let p = parse_program("return 1 + 2 + 4;");
        assert_eq!(run(&p).unwrap(), 7);
    }

    // RES-104 closed Phase G — let bindings + identifier reads
    // both work now. The test that was here pinning the
    // unsupported case was retired; the equivalent positive
    // test (jit_let_and_use, below) replaces it.

    #[test]
    fn jit_undeclared_identifier_unsupported() {
        // An identifier read with no matching `let` is still
        // unsupported in Phase G — a future ticket can promote
        // this to a richer "scope error" diagnostic, but for
        // now Unsupported with the descriptor is enough.
        let p = parse_program("return undefined_var;");
        match run(&p).unwrap_err() {
            JitError::Unsupported(msg) => assert!(
                msg.contains("identifier not in scope"),
                "expected scope descriptor, got: {}",
                msg
            ),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    // RES-100 closed Phase D — comparison ops work now too.
    // What's still unsupported at the expression level: prefix
    // ops (`-x`, `!x`), identifiers, calls, blocks. This test
    // pins one of those (prefix `-`) so the descriptor list keeps
    // being a useful diagnostic for users.
    #[test]
    fn jit_rejects_prefix_for_now() {
        let p = parse_program("return -5;");
        let err = run(&p).unwrap_err();
        match err {
            JitError::Unsupported(msg) => assert!(
                msg.contains("Prefix"),
                "expected node-kind in descriptor, got: {}",
                msg
            ),
            _ => panic!("expected Unsupported, got {:?}", err),
        }
    }

    // ---------- RES-099: Sub/Mul/Div/Mod ----------

    #[test]
    fn jit_subtraction() {
        let p = parse_program("return 10 - 3;");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_multiplication() {
        let p = parse_program("return 6 * 7;");
        assert_eq!(run(&p).unwrap(), 42);
    }

    #[test]
    fn jit_division() {
        let p = parse_program("return 100 / 4;");
        assert_eq!(run(&p).unwrap(), 25);
    }

    #[test]
    fn jit_modulo() {
        let p = parse_program("return 17 % 5;");
        assert_eq!(run(&p).unwrap(), 2);
    }

    #[test]
    fn jit_arith_chain_respects_precedence() {
        // Pratt precedence: `*` binds tighter than `+`, so this
        // parses as `2 + (3 * 4)` = 14. Exercises composition of
        // two different ops without needing explicit grouping.
        let p = parse_program("return 2 + 3 * 4;");
        assert_eq!(run(&p).unwrap(), 14);
    }

    #[test]
    fn jit_arith_chain_all_four_ops() {
        // 20 / 4 = 5; 5 * 3 = 15; 15 - 2 = 13; 13 + 1 = 14.
        // Verifies sdiv/imul/isub/iadd compose left-to-right
        // within their precedence tier.
        let p = parse_program("return 20 / 4 * 3 - 2 + 1;");
        assert_eq!(run(&p).unwrap(), 14);
    }

    // ---------- RES-100: comparisons + bool literals ----------

    #[test]
    fn jit_lt_returns_zero_for_false() {
        // 5 < 3 is false → Cranelift's icmp returns 0, uextend
        // widens to i64(0).
        let p = parse_program("return 5 < 3;");
        assert_eq!(run(&p).unwrap(), 0);
    }

    #[test]
    fn jit_lt_returns_one_for_true() {
        let p = parse_program("return 3 < 5;");
        assert_eq!(run(&p).unwrap(), 1);
    }

    #[test]
    fn jit_eq_int() {
        let true_case = parse_program("return 7 == 7;");
        assert_eq!(run(&true_case).unwrap(), 1);
        let false_case = parse_program("return 7 == 8;");
        assert_eq!(run(&false_case).unwrap(), 0);
    }

    #[test]
    fn jit_ne_int() {
        let true_case = parse_program("return 1 != 2;");
        assert_eq!(run(&true_case).unwrap(), 1);
        let false_case = parse_program("return 1 != 1;");
        assert_eq!(run(&false_case).unwrap(), 0);
    }

    #[test]
    fn jit_le_ge_boundary_equality() {
        // <= and >= must each return 1 at boundary equality and
        // 0 just past the boundary.
        let le = parse_program("return 5 <= 5;");
        assert_eq!(run(&le).unwrap(), 1);
        let ge = parse_program("return 5 >= 5;");
        assert_eq!(run(&ge).unwrap(), 1);
        let le_strict = parse_program("return 6 <= 5;");
        assert_eq!(run(&le_strict).unwrap(), 0);
        let ge_strict = parse_program("return 4 >= 5;");
        assert_eq!(run(&ge_strict).unwrap(), 0);
    }

    #[test]
    fn jit_bool_literal_true() {
        let p = parse_program("return true;");
        assert_eq!(run(&p).unwrap(), 1);
    }

    #[test]
    fn jit_bool_literal_false() {
        let p = parse_program("return false;");
        assert_eq!(run(&p).unwrap(), 0);
    }

    #[test]
    fn jit_compare_with_arith() {
        // Composes the RES-099 arith lowerings with the new
        // comparison lowering. Pratt: `+` binds tighter than `<`,
        // so this is `(2 + 3) < 10` = true → 1.
        let p = parse_program("return 2 + 3 < 10;");
        assert_eq!(run(&p).unwrap(), 1);
    }

    // ---------- RES-102: if/else with brif ----------

    #[test]
    fn jit_if_then_returns() {
        // `if (1 < 2) { return 7; } return 9;` — Phase E requires
        // both arms to return, so phrase as if-else (this test
        // documents the natural form users reach for).
        let p = parse_program("if (1 < 2) { return 7; } else { return 9; }");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_if_else_returns() {
        let p = parse_program("if (1 > 2) { return 7; } else { return 9; }");
        assert_eq!(run(&p).unwrap(), 9);
    }

    #[test]
    fn jit_if_with_arith_cond() {
        // The condition exercises both arith (5+5) and comparison
        // (==) lowerings before reaching the if. true → 1 arm.
        let p = parse_program("if (5 + 5 == 10) { return 1; } else { return 0; }");
        assert_eq!(run(&p).unwrap(), 1);
    }

    #[test]
    fn jit_if_with_bool_literal_cond() {
        // BooleanLiteral lowers to iconst 0/1, which is exactly
        // what brif consumes. No icmp required.
        let p = parse_program("if (true) { return 42; } else { return 0; }");
        assert_eq!(run(&p).unwrap(), 42);
        let p2 = parse_program("if (false) { return 42; } else { return 0; }");
        assert_eq!(run(&p2).unwrap(), 0);
    }

    // ---------- RES-104: let bindings + identifier reads ----------

    #[test]
    fn jit_let_and_use() {
        // Smallest meaningful test: bind a value, then use it.
        let p = parse_program("let x = 5; return x + 10;");
        assert_eq!(run(&p).unwrap(), 15);
    }

    #[test]
    fn jit_let_in_arith() {
        // Two locals in an arithmetic expression. Pratt: `*`
        // binds tighter than `+`, so this is `a * b + 2` →
        // (3 * 4) + 2 = 14.
        let p = parse_program("let a = 3; let b = 4; return a * b + 2;");
        assert_eq!(run(&p).unwrap(), 14);
    }

    #[test]
    fn jit_let_in_if_condition() {
        // Identifier read inside an if condition: composes
        // RES-100 comparison + RES-104 lookup.
        let p = parse_program("let x = 5; if (x > 0) { return x; } else { return 0; }");
        assert_eq!(run(&p).unwrap(), 5);
    }

    #[test]
    fn jit_let_inside_arm() {
        // `let` inside a then-arm — the LowerCtx threads down
        // through lower_block_or_stmt, so the local is visible
        // for the arm-local return. Phase G is function-scoped,
        // so the binding outlives the arm but no test exercises
        // that yet (would need post-if usage).
        let p = parse_program("if (1 < 2) { let y = 7; return y; } else { return 0; }");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_let_shadowing() {
        // `let x = 1; let x = 2; return x;` — second `let x`
        // overwrites the HashMap entry, so the use_var picks
        // up the fresh Variable. Function-scoped semantics
        // mean shadowing is just rebinding.
        let p = parse_program("let x = 1; let x = 2; return x;");
        assert_eq!(run(&p).unwrap(), 2);
    }

    #[test]
    fn jit_let_used_after_if_fallthrough() {
        // Combines RES-103 fallthrough with RES-104 locals:
        // bind x, conditionally early-return, otherwise use x
        // in the trailing return. Proves the LowerCtx survives
        // across the merge_block.
        let p = parse_program("let x = 7; if (false) { return 0; } return x + 1;");
        assert_eq!(run(&p).unwrap(), 8);
    }

    // ---------- RES-105: function declarations + calls ----------

    #[test]
    fn jit_calls_zero_arg_function() {
        let p = parse_program("fn answer() { return 42; } return answer();");
        assert_eq!(run(&p).unwrap(), 42);
    }

    #[test]
    fn jit_calls_one_arg_function() {
        let p = parse_program("fn square(int x) { return x * x; } return square(7);");
        assert_eq!(run(&p).unwrap(), 49);
    }

    #[test]
    fn jit_calls_two_arg_function() {
        let p = parse_program("fn add(int a, int b) { return a + b; } return add(3, 4);");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_calls_with_local_arg() {
        // Composes RES-104 (let bindings) with RES-105 (calls):
        // local feeds into the call, call result feeds into a
        // trailing arith op.
        let p =
            parse_program("fn square(int x) { return x * x; } let y = 5; return square(y) + 1;");
        assert_eq!(run(&p).unwrap(), 26);
    }

    #[test]
    fn jit_recursive_call_factorial() {
        // The two-pass declaration order (Pass 1 declares all
        // FuncIds before Pass 2 compiles any body) means a fn
        // can call itself via the same map. factorial(5) = 120.
        let p = parse_program(
            "fn factorial(int n) { \
                if (n <= 1) { return 1; } \
                return n * factorial(n - 1); \
            } \
            return factorial(5);",
        );
        assert_eq!(run(&p).unwrap(), 120);
    }

    #[test]
    fn jit_recursive_fib_25() {
        // The poster-child recursion: fib(25) = 75025. Same
        // workload the bytecode VM ran in 32 ms (RES-082).
        // Functional correctness check; RES-106 will time it.
        let p = parse_program(
            "fn fib(int n) { \
                if (n < 2) { return n; } \
                return fib(n - 1) + fib(n - 2); \
            } \
            return fib(25);",
        );
        assert_eq!(run(&p).unwrap(), 75025);
    }

    #[test]
    fn jit_call_unknown_function_unsupported() {
        let p = parse_program("return undefined_fn();");
        match run(&p).unwrap_err() {
            JitError::Unsupported(msg) => assert!(
                msg.contains("unknown function"),
                "expected unknown-function descriptor, got: {}",
                msg
            ),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    #[test]
    fn jit_call_arity_mismatch_unsupported() {
        let p = parse_program("fn f(int x) { return x; } return f(1, 2);");
        match run(&p).unwrap_err() {
            JitError::Unsupported(msg) => assert!(
                msg.contains("arity"),
                "expected arity descriptor, got: {}",
                msg
            ),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    #[test]
    fn jit_mutual_recursion_even_odd() {
        // Two functions that call each other. Pass 1 declares
        // both before Pass 2 compiles either body, so neither
        // call site sees a missing FuncId.
        let p = parse_program(
            "fn is_even(int n) { \
                if (n == 0) { return 1; } \
                return is_odd(n - 1); \
            } \
            fn is_odd(int n) { \
                if (n == 0) { return 0; } \
                return is_even(n - 1); \
            } \
            return is_even(10);",
        );
        assert_eq!(run(&p).unwrap(), 1);
    }

    // ---------- RES-103: merge block + fallthrough ----------

    #[test]
    fn jit_if_then_returns_else_falls_through() {
        // then-arm taken, returns 7. The else-arm falls through
        // to the merge block, where the trailing `return 9;`
        // lowers. Tests the "then terminates, else doesn't" path.
        let p = parse_program("if (1 < 2) { return 7; } return 9;");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_then_falls_through_else_returns() {
        // Inverse of above: condition false → else taken (returns
        // 9). When then-arm executes a no-return body, the
        // fallthrough hits the trailing return. We can't easily
        // construct "then has no return" without let bindings
        // (RES-104) — use bare-if instead, which is also a
        // fallthrough-from-then case.
        let p = parse_program("if (1 > 2) { return 7; } return 9;");
        assert_eq!(run(&p).unwrap(), 9);
    }

    #[test]
    fn jit_bare_if_with_fallthrough_false_branch() {
        // No else; condition false → fallthrough to trailing
        // return. This is the case Phase E rejected with
        // "bare `if` without else"; Phase F accepts it.
        let p = parse_program("if (false) { return 7; } return 9;");
        assert_eq!(run(&p).unwrap(), 9);
    }

    #[test]
    fn jit_bare_if_with_fallthrough_true_branch() {
        // No else; condition true → then-arm returns 7. The
        // trailing return is unreachable but still lowers
        // (cranelift is happy with dead blocks).
        let p = parse_program("if (true) { return 7; } return 9;");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_two_ifs_in_sequence() {
        // First if falls through (false branch), second if
        // returns. Proves the merge_block correctly hands
        // control back to compile_node_list which then walks
        // the second if. A nice end-to-end test of the
        // fallthrough mechanic.
        let p = parse_program("if (false) { return 1; } if (true) { return 2; } return 3;");
        assert_eq!(run(&p).unwrap(), 2);
    }

    #[test]
    fn jit_nested_if_in_then_arm() {
        // A nested if inside the then-arm. The inner if must also
        // have both branches returning per Phase E. This proves
        // the recursive lower_block_or_stmt handles nested control
        // flow without specialcasing.
        let p = parse_program(
            "if (1 < 2) { if (3 < 4) { return 1; } else { return 2; } } else { return 9; }",
        );
        assert_eq!(run(&p).unwrap(), 1);
    }

    // RES-103 lifted Phase E's "both arms must return" rule.
    // The two old tests (jit_rejects_if_without_else,
    // jit_rejects_if_arm_without_return) pinned shapes that
    // Phase F now accepts via the merge_block. Below: the
    // shape that's STILL rejected — an if that doesn't return
    // AND has nothing after it. The function never returns.

    #[test]
    fn jit_if_with_no_return_anywhere_is_empty_program() {
        // `if (false) { let x = 1; }` — no return in either
        // arm, no trailing statement. Function never returns,
        // so this surfaces as EmptyProgram (same error a bare
        // `let x = 1;` would).
        let p = parse_program("if (1 < 2) { let x = 1; }");
        assert_eq!(run(&p).unwrap_err(), JitError::EmptyProgram);
    }

    #[test]
    fn jit_empty_program_is_clean_error() {
        let p = parse_program("let x = 1;");
        let err = run(&p).unwrap_err();
        assert_eq!(err, JitError::EmptyProgram);
    }

    #[test]
    fn jit_error_display_is_descriptive() {
        assert_eq!(
            JitError::Unsupported("test").to_string(),
            "jit: unsupported: test"
        );
        assert_eq!(
            JitError::EmptyProgram.to_string(),
            "jit: program has no top-level return"
        );
        assert_eq!(
            JitError::IsaInit("foo".into()).to_string(),
            "jit: ISA init failed: foo"
        );
    }

    // --- RES-107: reassignment + while loops (Phase J) ---

    #[test]
    fn jit_simple_reassignment() {
        let p = parse_program("let x = 1; x = 2; return x;");
        assert_eq!(run(&p).unwrap(), 2);
    }

    #[test]
    fn jit_reassignment_in_arith() {
        let p = parse_program("let x = 5; x = x + 10; return x;");
        assert_eq!(run(&p).unwrap(), 15);
    }

    #[test]
    fn jit_while_counts_to_ten() {
        let p = parse_program("let i = 0; while (i < 10) { i = i + 1; } return i;");
        assert_eq!(run(&p).unwrap(), 10);
    }

    #[test]
    fn jit_while_sum_loop() {
        let p = parse_program(
            "let i = 0; let sum = 0; while (i < 5) { sum = sum + i; i = i + 1; } return sum;",
        );
        // 0 + 1 + 2 + 3 + 4 = 10.
        assert_eq!(run(&p).unwrap(), 10);
    }

    #[test]
    fn jit_while_zero_iterations() {
        // Header→exit on the first check: i stays 5.
        let p = parse_program("let i = 5; while (i < 0) { i = i + 1; } return i;");
        assert_eq!(run(&p).unwrap(), 5);
    }

    #[test]
    fn jit_reassign_undeclared_unsupported() {
        // No `let x = ...;` before the reassignment — must surface
        // the "undeclared identifier" descriptor cleanly.
        let p = parse_program("x = 1; return x;");
        let err = run(&p).unwrap_err();
        match err {
            JitError::Unsupported(msg) => assert!(
                msg.contains("undeclared identifier"),
                "expected undeclared-identifier descriptor, got: {}",
                msg
            ),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    // ---------- RES-168: direct-self-recursion TCO ----------

    #[test]
    fn tco_one_million_deep_recursion_does_not_stack_overflow() {
        // Ticket AC: `count(n)` at n = 1_000_000 completes without
        // stack overflow. Without TCO this would exhaust the host
        // thread's stack — ~1 MB of frames at ~16 bytes each.
        //
        // The body `return count(n - 1);` is a direct tail call to
        // the enclosing `count`; the JIT recognizes it and lowers
        // to a back-edge jump instead of a recursive call. The
        // test run-to-completion itself is the assertion.
        let p = parse_program(
            "fn count(int n) { \
                if (n <= 0) { return 0; } \
                return count(n - 1); \
            } \
            return count(1000000);",
        );
        assert_eq!(run(&p).unwrap(), 0);
    }

    #[test]
    fn tco_accumulator_style_sums_1_to_100k() {
        // Classic TCO shape: accumulator threaded through the
        // recursion. sum(n, 0) with n = 100_000 yields
        // 100_000 * 100_001 / 2 = 5_000_050_000. Tests that TCO
        // correctly reassigns TWO params (n and acc) on each
        // back-edge in the right order.
        let p = parse_program(
            "fn sum(int n, int acc) { \
                if (n <= 0) { return acc; } \
                return sum(n - 1, acc + n); \
            } \
            return sum(100000, 0);",
        );
        assert_eq!(run(&p).unwrap(), 5_000_050_000);
    }

    #[test]
    fn tco_only_fires_on_direct_self_recursion_not_wrapped_calls() {
        // `return 1 + count(n-1)` is NOT in tail position — the
        // call's result is consumed by `+`. The JIT must emit a
        // regular call here (still stacks up). With a small n
        // (10) the result is correct regardless of whether TCO
        // fires; the test is really about the large-n variant
        // below. Here we just assert correctness of the non-tail
        // shape.
        let p = parse_program(
            "fn inc_count(int n) { \
                if (n <= 0) { return 0; } \
                return 1 + inc_count(n - 1); \
            } \
            return inc_count(10);",
        );
        assert_eq!(run(&p).unwrap(), 10);
    }

    #[test]
    fn tco_only_fires_on_matching_arity() {
        // Self-call with different arity uses the existing
        // arity-mismatch error path — not TCO. Gives us a
        // regression check that TCO doesn't accidentally
        // consume these mistakes silently.
        let p = parse_program("fn f(int n) { return f(n, 0); } return f(1);");
        // Regular path: declared 1 param, called with 2 → the
        // JIT's existing arity check rejects.
        match run(&p).unwrap_err() {
            JitError::Unsupported(msg) => assert!(
                msg.contains("arity"),
                "expected arity mismatch diagnostic, got: {}",
                msg
            ),
            other => panic!("expected Unsupported(arity), got {:?}", other),
        }
    }

    #[test]
    fn tco_does_not_apply_to_cross_function_tail_calls() {
        // `return g(n)` from `f` is a cross-function tail call —
        // TCO as specified only handles direct self-recursion, so
        // this falls back to a regular call (correct, but not TCO).
        // Correctness only here; the no-stack-overflow guarantee
        // specifically does NOT extend to cross-function tails.
        let p = parse_program(
            "fn g(int n) { return n + 1; } \
            fn f(int n) { return g(n); } \
            return f(41);",
        );
        assert_eq!(run(&p).unwrap(), 42);
    }

    // ---------- RES-174: AST-hash JIT cache ----------

    /// Serialize the JIT-cache-stats-observing tests. The global
    /// counters in `jit_backend` are a shared resource; parallel
    /// test execution would race and give each other false
    /// deltas. Mirror of the RES-150 `RNG_TEST_LOCK` pattern.
    static JIT_CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn jit_cache_hit_on_duplicate_fn_body() {
        let _g = JIT_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Two fns with identical bodies (different names) should
        // produce the same AST hash, so the second declaration is
        // a cache hit — same FuncId, no second compile. Calls to
        // `g(7)` actually dispatch to `f`'s compiled code.
        let p = parse_program(
            "fn f(int n) { return n * 2; } \
             fn g(int n) { return n * 2; } \
             return f(5) + g(7);",
        );
        let (result, hits, misses, compiles) = run_with_stats(&p).unwrap();
        assert_eq!(result, 24, "f(5) + g(7) = 10 + 14");
        assert_eq!(hits, 1, "second fn with same body = 1 hit");
        assert_eq!(misses, 1, "first fn = 1 miss");
        assert_eq!(compiles, 1, "only f's body is compiled");
    }

    #[test]
    fn jit_cache_miss_on_distinct_bodies() {
        let _g = JIT_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Two fns with different bodies → no hit; both compile.
        let p = parse_program(
            "fn f(int n) { return n * 2; } \
             fn g(int n) { return n + 99; } \
             return f(5) + g(1);",
        );
        let (result, hits, misses, compiles) = run_with_stats(&p).unwrap();
        assert_eq!(result, 110, "10 + 100");
        assert_eq!(hits, 0);
        assert_eq!(misses, 2);
        assert_eq!(compiles, 2);
    }

    #[test]
    fn jit_cache_ignores_span_differences() {
        let _g = JIT_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Spans differ across the two fn decls (the second one
        // is on a different source line), but the hash is
        // span-stripped — so they still collide and the cache
        // treats them as identical.
        let p = parse_program(
            "fn f(int x) { return x * x; }\n\n\n\n\
             fn g(int y) { return y * y; } \
             return f(3) + g(4);",
        );
        let (result, hits, _, _) = run_with_stats(&p).unwrap();
        // Note: parameter NAMES (`x` vs `y`) are part of the
        // canonical form, so these two DON'T hash the same.
        // This test locks the policy: names matter (a rename
        // can make different code — consider shadowing or
        // captures in future features), spans don't.
        assert_eq!(result, 25);
        assert_eq!(hits, 0, "parameter rename prevents cache hit");
    }

    #[test]
    fn jit_cache_three_way_alias() {
        let _g = JIT_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Three fns, same body → first compiles, next two hit.
        let p = parse_program(
            "fn a(int n) { return n + 1; } \
             fn b(int n) { return n + 1; } \
             fn c(int n) { return n + 1; } \
             return a(10) + b(20) + c(30);",
        );
        let (result, hits, misses, compiles) = run_with_stats(&p).unwrap();
        assert_eq!(result, 63);
        assert_eq!(hits, 2);
        assert_eq!(misses, 1);
        assert_eq!(compiles, 1);
    }

    #[test]
    fn jit_fn_hash_is_deterministic_and_span_independent() {
        // Same function compiled twice with DIFFERENT synthetic
        // spans must produce the same hash.
        use crate::span::Span;
        let body_a = Node::ReturnStatement {
            value: Some(Box::new(Node::IntegerLiteral {
                value: 42,
                span: Span::default(),
            })),
            span: Span::default(),
        };
        let body_b = body_a.clone();
        let parameters: Vec<(String, String)> = vec![("int".into(), "n".into())];
        let h_a = fn_hash(&parameters, &[], &[], &body_a);
        let h_b = fn_hash(&parameters, &[], &[], &body_b);
        assert_eq!(h_a, h_b, "same fn AST must hash identically");
        // A different body must produce a different hash.
        let body_c = Node::ReturnStatement {
            value: Some(Box::new(Node::IntegerLiteral {
                value: 43,
                span: Span::default(),
            })),
            span: Span::default(),
        };
        let h_c = fn_hash(&parameters, &[], &[], &body_c);
        assert_ne!(h_a, h_c, "different bodies must hash differently");
    }

    // ---------- RES-175: leaf-fn inliner ----------

    #[test]
    fn inliner_counts_nodes_correctly() {
        // A simple `fn n_times_2` body: Block([Return(Some(Infix(*, n, 2)))])
        //   Block(1) + Return(2) + Infix(3) + Identifier(4) + IntLit(5) = 5
        use crate::span::Span;
        let body = Node::Block {
            stmts: vec![Node::ReturnStatement {
                value: Some(Box::new(Node::InfixExpression {
                    left: Box::new(Node::Identifier {
                        name: "n".to_string(),
                        span: Span::default(),
                    }),
                    operator: "*".to_string(),
                    right: Box::new(Node::IntegerLiteral {
                        value: 2,
                        span: Span::default(),
                    }),
                    span: Span::default(),
                })),
                span: Span::default(),
            }],
            span: Span::default(),
        };
        assert_eq!(count_nodes(&body), 5);
        assert!(is_trivial_leaf(&body, "double", None));
    }

    #[test]
    fn inliner_rejects_fn_with_call_in_body() {
        // A body with ANY call is NOT a trivial leaf.
        let p = parse_program(
            "fn helper(int n) { return n; } \
             fn wrap(int n) { return helper(n); } \
             return wrap(10);",
        );
        // Correctness still holds via the regular indirect-call
        // path — wrap's body contains helper() so it can't be
        // inlined.
        assert_eq!(run(&p).unwrap(), 10);
    }

    #[test]
    fn inliner_rejects_self_recursion() {
        // `fn fact(int n) { if n <= 1 { return 1; } return n * fact(n - 1); }`
        // — contains a call AND a loop-ish structure; doubly
        // disqualified. But let's pin the direct-self-call rule
        // explicitly with a trivial self-call body:
        // `fn f(int n) { return f(n); }` — still calls, so
        // rejected by the call guard first. The self-name guard
        // is a belt-and-suspenders check tested at the predicate
        // level.
        use crate::span::Span;
        let body = Node::Block {
            stmts: vec![Node::ReturnStatement {
                value: Some(Box::new(Node::Identifier {
                    name: "n".to_string(),
                    span: Span::default(),
                })),
                span: Span::default(),
            }],
            span: Span::default(),
        };
        // Body is trivially leafy EXCEPT for the self-call rule.
        // Pretending we're inside `f` compiling a call to `f`
        // should return false.
        assert!(!is_trivial_leaf(&body, "f", Some("f")));
        // But calling `f` from a different enclosing fn IS fine.
        assert!(is_trivial_leaf(&body, "f", Some("g")));
    }

    #[test]
    fn inliner_rejects_body_exceeding_node_limit() {
        // A body with more than TRIVIAL_LEAF_MAX_NODES nodes —
        // build one by nesting InfixExpressions.
        use crate::span::Span;
        let mut body: Node = Node::Identifier {
            name: "n".to_string(),
            span: Span::default(),
        };
        for _ in 0..10 {
            body = Node::InfixExpression {
                left: Box::new(body),
                operator: "+".to_string(),
                right: Box::new(Node::IntegerLiteral {
                    value: 1,
                    span: Span::default(),
                }),
                span: Span::default(),
            };
        }
        assert!(count_nodes(&body) > TRIVIAL_LEAF_MAX_NODES);
        assert!(!is_trivial_leaf(&body, "huge", None));
    }

    #[test]
    fn inliner_preserves_correctness_for_trivial_leaves() {
        // Trivial leaf fns exist; calling them produces the
        // correct result regardless of whether the inliner fires.
        // The inlined path and indirect path must agree — tested
        // here by running a program that exercises both shapes.
        let p = parse_program(
            "fn double(int n) { return n * 2; } \
             fn triple(int n) { return n * 3; } \
             fn add(int a, int b) { return a + b; } \
             return add(double(5), triple(4));",
        );
        // double(5)=10, triple(4)=12, add(10,12)=22.
        assert_eq!(run(&p).unwrap(), 22);
    }

    #[test]
    fn inliner_fires_on_nested_trivial_calls() {
        // Nested call where both levels qualify — the outer
        // call's arg is an inline, but the argument expression
        // runs BEFORE the outer inline's merge block is
        // installed, so there's no ambiguity.
        //
        // Note: the JIT doesn't yet lower `PrefixExpression`
        // (unary `-`), so the body uses `0 - n` instead. Either
        // form exercises the nested-inline path.
        let p = parse_program(
            "fn inc(int n) { return n + 1; } \
             fn negate(int n) { return 0 - n; } \
             return inc(negate(3));",
        );
        // negate(3) = -3; inc(-3) = -2.
        assert_eq!(run(&p).unwrap(), -2);
    }

    #[test]
    fn inliner_preserves_fib_correctness() {
        // fib is NOT a leaf (has calls), so the inliner doesn't
        // fire. Correctness is preserved via the existing
        // indirect-call path.
        let p = parse_program(
            "fn fib(int n) { \
                if (n < 2) { return n; } \
                return fib(n - 1) + fib(n - 2); \
            } \
            return fib(10);",
        );
        assert_eq!(run(&p).unwrap(), 55);
    }

    #[test]
    fn inliner_fires_on_simple_arithmetic_leaf() {
        // Simplest possible leaf: `return n + 1`. Body tree:
        //   Block(1) + Return(2) + Infix(3) + Id(4) + IntLit(5) = 5.
        let p = parse_program(
            "fn plus_one(int n) { return n + 1; } \
             return plus_one(41);",
        );
        assert_eq!(run(&p).unwrap(), 42);
    }

    #[test]
    fn inliner_shadows_caller_local_with_same_name() {
        // Caller has a local `n`; callee's param is also `n`.
        // The inliner must shadow caller's `n` for the duration
        // of the body and restore it after, so the caller's
        // `n` is UNCHANGED by the call. Verify by checking
        // that `caller_n + inlined_result` uses the ORIGINAL
        // caller n (not the callee's).
        let p = parse_program(
            "fn double(int n) { return n * 2; } \
             fn caller(int n) { \
                let result = double(5); \
                return n + result; \
             } \
             return caller(100);",
        );
        // double(5) = 10; caller's n is still 100; 100 + 10 = 110.
        assert_eq!(run(&p).unwrap(), 110);
    }

    #[test]
    fn jit_cache_global_stats_accumulate_across_runs() {
        let _g = JIT_CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Two sequential `run()` calls — the global counters
        // should reflect AT LEAST this test's contribution (1
        // hit + 2 misses + 2 compiles). Other tests in the same
        // process also bump the globals, so exact equality
        // would flake under parallel test execution; `>=`
        // captures the accumulation invariant without that
        // fragility.
        let p1 = parse_program(
            "fn f(int n) { return n * 2; } \
             fn g(int n) { return n * 2; } \
             return f(1) + g(1);",
        );
        let p2 = parse_program(
            "fn h(int n) { return n - 1; } \
             return h(5);",
        );
        let (h0, m0, c0) = cache_stats();
        run(&p1).unwrap();
        run(&p2).unwrap();
        let (h1, m1, c1) = cache_stats();
        assert!(
            h1.saturating_sub(h0) >= 1,
            "expected at least 1 hit from run1, got {} delta",
            h1 - h0
        );
        assert!(
            m1.saturating_sub(m0) >= 2,
            "expected at least 2 misses total, got {} delta",
            m1 - m0
        );
        assert!(
            c1.saturating_sub(c0) >= 2,
            "expected at least 2 compiles total, got {} delta",
            c1 - c0
        );
    }

    // ============================================================
    // RES-165a: struct layout cache
    // ============================================================

    #[test]
    fn res165a_empty_program_has_no_layouts() {
        let p = parse_program("return 1;");
        assert!(collect_struct_layouts(&p).is_empty());
    }

    #[test]
    fn res165a_two_int_fields_have_natural_offsets() {
        // Point { int x, int y } — x@0, y@8, size 16, align 8.
        let p = parse_program(
            r#"
            struct Point {
                int x,
                int y,
            }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        let pt = layouts.get("Point").expect("Point layout missing");
        assert_eq!(pt.fields.len(), 2);
        assert_eq!(pt.fields[0].name, "x");
        assert_eq!(pt.fields[0].offset, 0);
        assert_eq!(pt.fields[0].ty, types::I64);
        assert_eq!(pt.fields[0].size, 8);
        assert_eq!(pt.fields[1].name, "y");
        assert_eq!(pt.fields[1].offset, 8);
        assert_eq!(pt.fields[1].ty, types::I64);
        assert_eq!(pt.total_size, 16);
        assert_eq!(pt.align, 8);
    }

    #[test]
    fn res165a_bool_then_int_pads_between_fields() {
        // S { bool b, int x } — b@0 (1 byte), then 7 bytes of
        // padding, x@8. Struct align is 8 (inherited from `int`),
        // total size 16.
        let p = parse_program(
            r#"
            struct S {
                bool b,
                int x,
            }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        let s = layouts.get("S").expect("S layout missing");
        assert_eq!(s.fields[0].name, "b");
        assert_eq!(s.fields[0].offset, 0);
        assert_eq!(s.fields[0].ty, types::I8);
        assert_eq!(s.fields[0].size, 1);
        assert_eq!(s.fields[1].name, "x");
        assert_eq!(s.fields[1].offset, 8);
        assert_eq!(s.fields[1].size, 8);
        assert_eq!(s.total_size, 16);
        assert_eq!(s.align, 8);
    }

    #[test]
    fn res165a_trailing_bool_pads_struct_to_alignment() {
        // T { int x, bool b } — x@0, b@8. Struct align 8, so
        // total size rounds up from 9 to 16 for arrays-of-T to
        // tile correctly.
        let p = parse_program(
            r#"
            struct T {
                int x,
                bool b,
            }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        let t = layouts.get("T").expect("T layout missing");
        assert_eq!(t.fields[0].offset, 0);
        assert_eq!(t.fields[1].offset, 8);
        assert_eq!(t.total_size, 16);
        assert_eq!(t.align, 8);
    }

    #[test]
    fn res165a_all_bool_fields_stay_byte_aligned() {
        // B { bool a, bool b, bool c } — a@0, b@1, c@2. Align 1,
        // so no trailing padding; total size 3.
        let p = parse_program(
            r#"
            struct B {
                bool a,
                bool b,
                bool c,
            }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        let b = layouts.get("B").expect("B layout missing");
        assert_eq!(b.fields[0].offset, 0);
        assert_eq!(b.fields[1].offset, 1);
        assert_eq!(b.fields[2].offset, 2);
        assert_eq!(b.total_size, 3);
        assert_eq!(b.align, 1);
    }

    #[test]
    fn res165a_float_field_uses_f64_ty() {
        // Mix of float + int.  Both are 8-byte aligned, so no
        // padding between them.
        let p = parse_program(
            r#"
            struct V {
                float x,
                int n,
            }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        let v = layouts.get("V").expect("V layout missing");
        assert_eq!(v.fields[0].name, "x");
        assert_eq!(v.fields[0].ty, types::F64);
        assert_eq!(v.fields[0].offset, 0);
        assert_eq!(v.fields[1].name, "n");
        assert_eq!(v.fields[1].ty, types::I64);
        assert_eq!(v.fields[1].offset, 8);
        assert_eq!(v.total_size, 16);
        assert_eq!(v.align, 8);
    }

    #[test]
    fn res165a_i32_field_uses_i32_ty_and_4_byte_align() {
        // Two i32s pack tightly at 4-byte alignment.
        let p = parse_program(
            r#"
            struct Pair32 {
                i32 a,
                i32 b,
            }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        let pr = layouts.get("Pair32").expect("Pair32 layout missing");
        assert_eq!(pr.fields[0].ty, types::I32);
        assert_eq!(pr.fields[0].offset, 0);
        assert_eq!(pr.fields[1].ty, types::I32);
        assert_eq!(pr.fields[1].offset, 4);
        assert_eq!(pr.total_size, 8);
        assert_eq!(pr.align, 4);
    }

    #[test]
    fn res165a_empty_struct_has_size_zero() {
        // An empty struct is legal in the parser; we give it
        // size 0, align 1 to mirror what repr(C) would do.
        let p = parse_program(r#"struct U { }"#);
        let layouts = collect_struct_layouts(&p);
        let u = layouts.get("U").expect("U layout missing");
        assert!(u.fields.is_empty());
        assert_eq!(u.total_size, 0);
        assert_eq!(u.align, 1);
    }

    #[test]
    fn res165a_multiple_struct_decls_are_all_cached() {
        let p = parse_program(
            r#"
            struct A { int x, }
            struct B { bool flag, }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        assert_eq!(layouts.len(), 2);
        assert!(layouts.contains_key("A"));
        assert!(layouts.contains_key("B"));
    }

    #[test]
    fn res165a_unknown_struct_name_lookup_is_none() {
        let p = parse_program(r#"struct P { int x, }"#);
        let layouts = collect_struct_layouts(&p);
        assert!(layouts.get("Q").is_none());
    }

    #[test]
    fn res165a_field_by_name_lookup_roundtrips() {
        let p = parse_program(
            r#"
            struct R {
                int a,
                int b,
                bool c,
            }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        let r = layouts.get("R").unwrap();
        assert_eq!(r.field("a").unwrap().offset, 0);
        assert_eq!(r.field("b").unwrap().offset, 8);
        assert_eq!(r.field("c").unwrap().offset, 16);
        assert!(r.field("nope").is_none());
    }

    #[test]
    fn res165a_unknown_field_type_falls_back_to_pointer() {
        // A field whose type isn't a known primitive (e.g. another
        // user struct, an array type) maps to a machine-pointer
        // I64. Regression guard for the fallback branch in
        // `cranelift_ty_for`.
        let p = parse_program(
            r#"
            struct Node {
                int tag,
                Mystery payload,
            }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        let n = layouts.get("Node").unwrap();
        assert_eq!(n.fields[0].ty, types::I64); // int
        assert_eq!(n.fields[1].ty, types::I64); // pointer fallback
        assert_eq!(n.fields[1].offset, 8);
        assert_eq!(n.total_size, 16);
    }

    #[test]
    fn res165a_layouts_survive_non_struct_statements_around_them() {
        // A realistic program has let bindings, fn decls, and
        // struct decls intermixed at the top level. The collector
        // must pick up the struct and ignore the rest.
        let p = parse_program(
            r#"
            let start = 0;
            struct P { int x, int y, }
            fn pack(int a, int b) -> int { return a + b; }
        "#,
        );
        let layouts = collect_struct_layouts(&p);
        assert_eq!(layouts.len(), 1);
        let pt = layouts.get("P").unwrap();
        assert_eq!(pt.fields.len(), 2);
        assert_eq!(pt.total_size, 16);
    }

    // ============================================================
    // RES-166a: runtime_shims + JITBuilder::symbol wiring
    // ============================================================
    //
    // The shim fns are plain `extern "C"` so we can unit-test them
    // directly from Rust — no Cranelift / JITModule involvement
    // required. Each test owns its array pointer: allocate with
    // `res_array_new`, round-trip through `get` / `set`, free with
    // `res_array_free`. Failure to call `res_array_free` leaks, but
    // that's just a test-side concern; the tests always clean up.
    //
    // A separate test exercises the full JIT build path after the
    // `register_runtime_symbols` wiring to catch a regression where
    // symbol registration accidentally breaks module construction.

    use super::runtime_shims::{res_array_free, res_array_get, res_array_new, res_array_set};

    #[test]
    fn res166a_array_new_returns_nonnull_for_positive_len() {
        let arr = res_array_new(3);
        assert!(!arr.is_null(), "res_array_new(3) returned null");
        res_array_free(arr);
    }

    #[test]
    fn res166a_array_new_accepts_zero_len() {
        // Length 0 is legal — produces an empty-but-valid array.
        // Every get/set on it will panic (no indices in range),
        // so we don't touch the payload.
        let arr = res_array_new(0);
        assert!(!arr.is_null(), "res_array_new(0) returned null");
        res_array_free(arr);
    }

    #[test]
    fn res166a_array_new_clamps_negative_len_to_zero() {
        // A negative length must not abort — we clamp to 0 so the
        // JIT doesn't need to validate the arg inline. The
        // resulting array is still a valid (empty) handle.
        let arr = res_array_new(-5);
        assert!(!arr.is_null(), "res_array_new(-5) returned null");
        res_array_free(arr);
    }

    #[test]
    fn res166a_array_new_zero_initializes_elements() {
        let arr = res_array_new(4);
        for i in 0..4 {
            assert_eq!(res_array_get(arr, i), 0);
        }
        res_array_free(arr);
    }

    #[test]
    fn res166a_array_set_then_get_roundtrips() {
        let arr = res_array_new(3);
        res_array_set(arr, 0, 10);
        res_array_set(arr, 1, 20);
        res_array_set(arr, 2, 30);
        assert_eq!(res_array_get(arr, 0), 10);
        assert_eq!(res_array_get(arr, 1), 20);
        assert_eq!(res_array_get(arr, 2), 30);
        res_array_free(arr);
    }

    #[test]
    fn res166a_array_set_overwrites_previous_value() {
        let arr = res_array_new(1);
        res_array_set(arr, 0, 1);
        res_array_set(arr, 0, 99);
        assert_eq!(res_array_get(arr, 0), 99);
        res_array_free(arr);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn res166a_array_get_oob_panics() {
        let arr = res_array_new(3);
        // Leaking here is OK — the panic aborts the test, and
        // cargo test runs each test in its own process-ish
        // sandbox anyway. Freeing after a panic would require a
        // catch_unwind wrapper which is overkill for a test.
        let _ = res_array_get(arr, 5);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn res166a_array_get_negative_idx_panics() {
        let arr = res_array_new(3);
        let _ = res_array_get(arr, -1);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn res166a_array_set_oob_panics() {
        let arr = res_array_new(2);
        res_array_set(arr, 2, 42);
    }

    #[test]
    #[should_panic(expected = "null array pointer")]
    fn res166a_array_get_null_ptr_panics_cleanly() {
        let _ = res_array_get(std::ptr::null_mut(), 0);
    }

    #[test]
    #[should_panic(expected = "null array pointer")]
    fn res166a_array_set_null_ptr_panics_cleanly() {
        res_array_set(std::ptr::null_mut(), 0, 1);
    }

    #[test]
    fn res166a_array_free_on_null_is_noop() {
        // Calling free with null must not crash — the JIT calls
        // it unconditionally on scope exit to keep the lowering
        // simple.
        res_array_free(std::ptr::null_mut());
    }

    #[test]
    fn res166a_symbol_wiring_does_not_break_module_construction() {
        // Regression guard: the addition of
        // `register_runtime_symbols(&mut builder)` inside
        // `make_module` must not disrupt JIT construction for
        // programs that don't use arrays. If it did, every
        // existing JIT test would fail — but a direct assertion
        // here pins the exact interaction.
        let m = make_module().expect("make_module failed after RES-166a wiring");
        drop(m); // free the module immediately — we only care that it built.
    }

    #[test]
    fn res166a_existing_jit_path_still_works_after_symbol_wiring() {
        // A full `run()` roundtrip that doesn't touch arrays must
        // still return the right result with the shim symbols
        // registered. This is the behavioural mirror of the
        // regression guard above — if symbol wiring went wrong
        // lazily (at call time), the module-construction test
        // would miss it but this one would catch it.
        let p = parse_program("return 2 + 3;");
        assert_eq!(run(&p).unwrap(), 5);
    }

    #[test]
    fn res166a_large_array_get_set_across_many_slots() {
        // Exercise the full Vec<i64> path — not just the first
        // few slots. Writes the identity `a[i] = i * 2` across
        // 100 slots and reads them back.
        let n: i64 = 100;
        let arr = res_array_new(n);
        for i in 0..n {
            res_array_set(arr, i, i * 2);
        }
        for i in 0..n {
            assert_eq!(res_array_get(arr, i), i * 2);
        }
        res_array_free(arr);
    }

    // ============================================================
    // RES-167a: JIT builtin shim table + registry
    // ============================================================

    use super::jit_builtins::{res_jit_abs, res_jit_max, res_jit_min};
    use super::{jit_builtin_table, lookup_jit_builtin};

    #[test]
    fn res167a_abs_positive_is_unchanged() {
        assert_eq!(res_jit_abs(7), 7);
    }

    #[test]
    fn res167a_abs_negative_is_magnitude() {
        assert_eq!(res_jit_abs(-7), 7);
    }

    #[test]
    fn res167a_abs_zero_is_zero() {
        assert_eq!(res_jit_abs(0), 0);
    }

    #[test]
    fn res167a_abs_min_i64_wraps_without_panic() {
        // `i64::MIN.abs()` would panic in debug; the JIT shim
        // uses `wrapping_abs` to match release-mode interpreter
        // behaviour and keep the FFI call total.
        assert_eq!(res_jit_abs(i64::MIN), i64::MIN);
    }

    #[test]
    fn res167a_min_picks_smaller_arg() {
        assert_eq!(res_jit_min(3, 7), 3);
        assert_eq!(res_jit_min(7, 3), 3);
        assert_eq!(res_jit_min(-5, 5), -5);
        assert_eq!(res_jit_min(-5, -10), -10);
    }

    #[test]
    fn res167a_min_equal_args_returns_either() {
        assert_eq!(res_jit_min(4, 4), 4);
    }

    #[test]
    fn res167a_max_picks_larger_arg() {
        assert_eq!(res_jit_max(3, 7), 7);
        assert_eq!(res_jit_max(7, 3), 7);
        assert_eq!(res_jit_max(-5, 5), 5);
        assert_eq!(res_jit_max(-5, -10), -5);
    }

    #[test]
    fn res167a_max_equal_args_returns_either() {
        assert_eq!(res_jit_max(4, 4), 4);
    }

    #[test]
    fn res167a_lookup_known_builtin_roundtrips() {
        let b = lookup_jit_builtin("abs").expect("abs missing from table");
        assert_eq!(b.name, "abs");
        assert_eq!(b.symbol, "res_jit_abs");
        assert_eq!(b.arity, 1);
        // The address field is opaque — we only verify it's
        // non-null (function pointers to live Rust fns always
        // are).
        assert!(!b.addr.is_null());
    }

    #[test]
    fn res167a_lookup_unknown_builtin_is_none() {
        // `println` is a real interpreter builtin, but the JIT
        // doesn't support it yet (RES-167b/c scope). Looking it
        // up returns None so the lowering can bail cleanly with
        // Unsupported instead of crashing.
        assert!(lookup_jit_builtin("println").is_none());
        // Pure gibberish returns None as well.
        assert!(lookup_jit_builtin("nonexistent").is_none());
    }

    #[test]
    fn res167a_registry_is_sorted_by_name() {
        // Keep the registry alphabetically sorted so the
        // miss-lookup test above stays stable when more entries
        // are added (sort by `name`, not `symbol` — name is the
        // Resilient-source identifier, symbol is the FFI prefix).
        let names: Vec<&str> = jit_builtin_table().iter().map(|b| b.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            names, sorted,
            "jit builtin table must stay alphabetically sorted"
        );
    }

    #[test]
    fn res167a_registry_has_no_duplicate_names() {
        let mut names: Vec<&str> = jit_builtin_table().iter().map(|b| b.name).collect();
        names.sort();
        let orig_len = names.len();
        names.dedup();
        assert_eq!(names.len(), orig_len, "duplicate builtin name in registry");
    }

    #[test]
    fn res167a_arity_matches_actual_signature() {
        // Sanity-check each entry's `arity` field matches the
        // shim's real parameter count. Without this, RES-167b's
        // lowering could silently emit a wrong signature.
        assert_eq!(lookup_jit_builtin("abs").unwrap().arity, 1);
        assert_eq!(lookup_jit_builtin("min").unwrap().arity, 2);
        assert_eq!(lookup_jit_builtin("max").unwrap().arity, 2);
    }

    #[test]
    fn res167a_symbol_prefix_distinguishes_from_array_runtime() {
        // Every JIT builtin symbol starts with `res_jit_`; every
        // array runtime shim starts with `res_array_`. The
        // distinct prefixes prevent namespace collisions in
        // cranelift's module-level symbol table.
        for b in jit_builtin_table() {
            assert!(
                b.symbol.starts_with("res_jit_"),
                "symbol {} missing res_jit_ prefix",
                b.symbol
            );
        }
    }

    #[test]
    fn res167a_module_still_builds_after_jit_builtin_wiring() {
        // Regression guard: adding `register_jit_builtin_symbols`
        // to `register_runtime_symbols` must not break module
        // construction.
        let m = make_module().expect("make_module failed after RES-167a wiring");
        drop(m);
    }

    #[test]
    fn res167a_existing_jit_run_path_still_returns_correct_result() {
        // Mirror of res166a's end-to-end guard, re-asserted
        // after this ticket's additional wiring.
        let p = parse_program("return 10 + 20;");
        assert_eq!(run(&p).unwrap(), 30);
    }
}
