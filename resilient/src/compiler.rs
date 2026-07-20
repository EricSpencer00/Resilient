//! RES-076 + RES-081: AST → bytecode compiler.
//!
//! Walks a `Node::Program` and emits a `Program { main, functions }`
//! for the VM to execute. Supports the subset covered by RES-076
//! (int arithmetic, let bindings, identifiers, return) plus RES-081
//! (top-level function declarations + calls).
//!
//! Locals are resolved at compile time to `u16` frame-relative
//! indices; the runtime never sees identifier strings. That's half
//! the perf win over the tree walker.

#![allow(dead_code)]

use crate::bytecode::{CatchArm, Chunk, CompileError, Function, LiveHandlerEntry, Op, Program};
use crate::{ChainAccess, Node, Value};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static ENUM_INDEX: RefCell<HashMap<String, Vec<crate::EnumVariant>>> = RefCell::new(HashMap::new());
    /// RES-3994: struct name → declared field names (declaration order),
    /// pre-scanned the same way `ENUM_INDEX` is. Used by `Node::StructLiteral`
    /// compilation to resolve `{ ..base, f: v }` struct-update syntax — the
    /// bytecode compiler otherwise has no way to know which fields `base`
    /// must supply at the call site, since `Op::StructLiteral` only carries
    /// the explicit field list.
    static STRUCT_FIELD_INDEX: RefCell<HashMap<String, Vec<String>>> = RefCell::new(HashMap::new());
}

/// Tracks break/continue patch sites accumulated while compiling a loop body.
///
/// Created fresh for each While/ForIn loop. A `Vec<LoopState>` stack is
/// threaded through the compilation so labeled `break`/`continue` can
/// target an enclosing loop by name. Unlabeled break/continue always
/// targets the *innermost* (last) entry.
struct LoopState {
    /// PC of the back-edge target. For `while` this is the condition check;
    /// for `for-in` it's the index-increment code (set after body compilation
    /// via `set_continue_target`).
    continue_target: usize,
    /// `Jump(0)` instruction indices emitted for `break` that need to be
    /// patched to the loop-exit PC once the loop is fully compiled.
    break_patches: Vec<usize>,
    /// `Jump(0)` instruction indices emitted for `continue` that need to be
    /// patched to `continue_target`. Used by `for-in` loops, where the target
    /// is not yet known when the body is compiled.
    continue_patches: Vec<usize>,
    /// RES-3993: `Jump(0)` instruction indices emitted for `break <expr>`
    /// (`Node::BreakWith`) when this loop is compiled in *expression*
    /// position (`value_mode == true`, see `compile_while_expr`). These
    /// jump straight past the loop's default `Void` result to the PC
    /// where the loop's value is expected on the stack — unlike
    /// `break_patches`, which target the `Void`-push stub since a plain
    /// `break;`/`break label;` never carries a value. Left empty (and
    /// unused) for statement-position loops.
    break_value_patches: Vec<usize>,
    /// RES-3993: true when this loop was compiled by `compile_while_expr`
    /// (i.e. it is used for its value, e.g. `let x = loop { ... };`).
    /// `Node::BreakWith` consults the *innermost* loop's `value_mode` to
    /// decide whether `break <expr>` should leave its value on the stack
    /// (`value_mode == true`) or evaluate-and-discard it like a plain
    /// `break` (`value_mode == false`, the loop's result is never read).
    value_mode: bool,
    /// RES-2502: optional label for labeled break/continue.
    label: Option<String>,
}

impl LoopState {
    fn new(continue_target: usize) -> Self {
        LoopState {
            continue_target,
            break_patches: Vec::new(),
            continue_patches: Vec::new(),
            break_value_patches: Vec::new(),
            value_mode: false,
            label: None,
        }
    }

    fn with_label(continue_target: usize, label: Option<String>) -> Self {
        LoopState {
            continue_target,
            break_patches: Vec::new(),
            continue_patches: Vec::new(),
            break_value_patches: Vec::new(),
            value_mode: false,
            label,
        }
    }

    /// Retroactively fix up all `continue` patch sites — called by `for-in`
    /// after the index-increment code is in place.
    fn set_continue_target(&mut self, target: usize) {
        self.continue_target = target;
    }
}

/// RES-2532: flag bit marking a slot as a global (main-frame local)
/// rather than a function-frame local. Encoded in the `locals` HashMap
/// so function bodies resolve globals without changing every function
/// signature in the compiler.
const GLOBAL_FLAG: u16 = 0x8000;

/// RES-3914: flag bit marking a slot as "boxed" — its runtime value is
/// a `Value::Cell` handle (RES-328's shared-cell store) rather than a
/// raw value. Set the first time *any* nested closure captures the
/// name by mutable upvalue; every capture and every direct access
/// after that point routes through `Cell.get()` / `Cell.set()` so the
/// defining scope and every capturing closure observe the same
/// mutations, instead of each closure snapshotting an independent copy
/// (the root cause of RES-3914's crash-on-return and
/// wrong-value-on-interleaved-mutation bugs). Distinct bit from
/// `GLOBAL_FLAG` — globals never get boxed, since `LoadGlobal`/
/// `StoreGlobal` already address one shared slot directly.
const BOXED_FLAG: u16 = 0x4000;

/// RES-4046: flag bit marking a slot as a function-scoped `static let`
/// binding. Like `GLOBAL_FLAG`, this is encoded in the `locals` HashMap
/// so ordinary identifier reads/writes resolve to `LoadStatic`/
/// `StoreStatic` without threading a new parameter through the whole
/// compiler. Backing storage is the VM's per-function statics table,
/// keyed by the function's own chunk index, so the value persists
/// across separate calls — unlike a plain local, which lives in the
/// per-call locals slab and resets every call. Never combined with
/// `BOXED_FLAG`: statics are never boxed, for the same reason globals
/// aren't (see `BOXED_FLAG`'s doc comment) — `LoadStatic`/`StoreStatic`
/// already address one persistent slot directly.
const STATIC_FLAG: u16 = 0x2000;

/// RES-3914 / RES-4046: mask off `GLOBAL_FLAG`, `BOXED_FLAG`, and
/// `STATIC_FLAG`, leaving the raw frame-relative (or main-frame, or
/// static-table) slot index.
fn raw_slot(slot: u16) -> u16 {
    slot & !(GLOBAL_FLAG | BOXED_FLAG | STATIC_FLAG)
}

/// Emit a load instruction for a slot that may be local, global, or a
/// function-scoped static.
fn local_load_op(slot: u16) -> Op {
    if slot & GLOBAL_FLAG != 0 {
        Op::LoadGlobal(slot & !GLOBAL_FLAG)
    } else if slot & STATIC_FLAG != 0 {
        Op::LoadStatic(slot & !STATIC_FLAG)
    } else {
        Op::LoadLocal(slot)
    }
}

/// Emit a store instruction for a slot that may be local, global, or a
/// function-scoped static.
fn local_store_op(slot: u16) -> Op {
    if slot & GLOBAL_FLAG != 0 {
        Op::StoreGlobal(slot & !GLOBAL_FLAG)
    } else if slot & STATIC_FLAG != 0 {
        Op::StoreStatic(slot & !STATIC_FLAG)
    } else {
        Op::StoreLocal(slot)
    }
}

/// RES-3914: emit a read of `slot`, transparently unwrapping a boxed
/// (`BOXED_FLAG`) slot through `Cell.get()`. Used everywhere a plain
/// `local_load_op` would otherwise be emitted for an identifier that
/// might have been captured-by-mutable-upvalue.
fn emit_identifier_load(chunk: &mut Chunk, slot: u16, line: u32) -> Result<(), CompileError> {
    if slot & BOXED_FLAG != 0 {
        chunk.emit(Op::LoadLocal(raw_slot(slot)), line);
        let method_const = chunk.add_string_constant("get")?;
        chunk.emit(
            Op::CallMethod {
                method_const,
                arity: 0,
            },
            line,
        );
    } else {
        chunk.emit(local_load_op(slot), line);
    }
    Ok(())
}

/// RES-2532: pre-scan top-level statements to collect global variable
/// names and their main-frame slot indices. Called before Pass 2
/// (function body compilation) so function bodies can reference
/// top-level `let` bindings via `LoadGlobal`/`StoreGlobal`.
///
/// The slot assignment must exactly mirror what `compile_stmt` +
/// `compile_control_flow` + `compile_for_in` do for `next_local`.
fn prescan_globals(stmts: &[crate::Spanned<Node>]) -> HashMap<String, u16> {
    let mut globals = HashMap::new();
    let mut slot: u16 = 0;
    for spanned in stmts {
        if matches!(
            spanned.node,
            Node::Function { .. }
                | Node::Extern { .. }
                | Node::RegionDecl { .. }
                | Node::StructDecl { .. }
                | Node::NewtypeDecl { .. }
        ) {
            continue;
        }
        prescan_stmt_slots(&spanned.node, &mut globals, &mut slot);
    }
    globals
}

fn prescan_stmt_slots(node: &Node, globals: &mut HashMap<String, u16>, slot: &mut u16) {
    match node {
        Node::LetStatement { name, .. } => {
            globals.insert(name.clone(), *slot);
            *slot += 1;
        }
        Node::StaticLet { name, .. } => {
            globals.insert(name.clone(), *slot);
            *slot += 1;
        }
        Node::LetTupleDestructure { names, .. } => {
            *slot += 1;
            for name in names {
                globals.insert(name.clone(), *slot);
                *slot += 1;
            }
        }
        Node::LetDestructureStruct { fields, .. } => {
            *slot += 1;
            for (_field, local_name) in fields {
                globals.insert(local_name.clone(), *slot);
                *slot += 1;
            }
        }
        Node::ForInStatement { body, .. } => {
            *slot += 4;
            prescan_stmt_slots(body.as_ref(), globals, slot);
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            prescan_stmt_slots(consequence.as_ref(), globals, slot);
            if let Some(alt) = alternative {
                prescan_stmt_slots(alt.as_ref(), globals, slot);
            }
        }
        Node::WhileStatement { body, .. } => {
            prescan_stmt_slots(body.as_ref(), globals, slot);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                prescan_stmt_slots(s, globals, slot);
            }
        }
        Node::LiveBlock { body, .. } | Node::UnsafeBlock { body, .. } => {
            prescan_stmt_slots(body.as_ref(), globals, slot);
        }
        _ => {}
    }
}

/// Compile a parsed program into a bytecode `Program` ready for the VM.
///
/// Steps:
/// 1. Pre-pass: find every top-level `fn` and index it by name so
///    call sites can refer to it regardless of source order (mirrors
///    the tree-walker's function-hoist in `eval_program`).
/// 2. Compile each function body into its own `Chunk`.
/// 3. Compile the remaining top-level statements into `main`.
///
/// A trailing `Op::Return` is appended to `main` unconditionally —
/// if the program ended with an explicit `return EXPR;` this is
/// unreachable and harmless; otherwise it terminates the VM with
/// `Value::Void`.
pub fn compile(program: &Node) -> Result<Program, CompileError> {
    // RES-3992: the tree-walker resolves top-level `const NAME = expr;`
    // declarations into a `self.consts` table that every identifier
    // lookup checks *before* locals (`Interpreter::eval_program` /
    // `Node::Identifier` in `eval`). The bytecode compiler has no
    // equivalent runtime-const scope — a `Node::Const` compiles to a
    // no-op (see `compile_stmt` / `compile_stmt_in_fn` below) and
    // every later reference to the const's name fell through to
    // `CompileError::UnknownIdentifier`. Inline resolved consts as
    // literals throughout the AST before compiling so the const's
    // *name* never needs runtime resolution — matches the tree-walker
    // output for every example in the differential corpus that only
    // uses consts in literal-foldable positions.
    let inlined;
    let program: &Node = match program {
        Node::Program(stmts) => {
            let resolved = resolve_top_level_consts(stmts);
            if resolved.is_empty() {
                program
            } else {
                inlined = inline_consts(program, &resolved);
                &inlined
            }
        }
        _ => program,
    };
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Err(CompileError::Unsupported("non-Program root")),
    };

    // Pre-pass 0 (FFI v2): resolve all extern blocks so foreign symbols
    // are available before any call-site compilation. Builds an
    // ffi_index: name → u16 parallel to fn_index.
    //
    // RES-1577: pre-size `ffi_index` and `foreign_syms` to the total
    // extern-decl count. Same shape as RES-1461's `fn_index` pre-size;
    // skips the default-bucket rehash chain for programs with many
    // FFI symbols. One linear pass over `stmts` to count, mirroring
    // the existing `fn_count` block below.
    #[cfg(feature = "ffi")]
    let ffi_count: usize = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::Extern { decls, .. } => Some(decls.len()),
            _ => None,
        })
        .sum();
    #[cfg(feature = "ffi")]
    let mut ffi_loader = crate::ffi::ForeignLoader::new();
    #[cfg(feature = "ffi")]
    let mut ffi_index: HashMap<String, u16> = HashMap::with_capacity(ffi_count);
    #[cfg(feature = "ffi")]
    let mut foreign_syms: Vec<std::sync::Arc<crate::ffi::ForeignSymbol>> =
        Vec::with_capacity(ffi_count);
    #[cfg(feature = "ffi")]
    {
        for spanned in stmts {
            if let Node::Extern { library, decls, .. } = &spanned.node {
                ffi_loader
                    .resolve_block(library, decls)
                    .map_err(|e| CompileError::FfiError(e.to_string()))?;
                for d in decls {
                    if d.is_variadic {
                        return Err(CompileError::Unsupported(
                            "variadic extern calls are supported by the tree-walker only",
                        ));
                    }
                    if let Some(sym) = ffi_loader.lookup(&d.resilient_name) {
                        if ffi_index.len() >= u16::MAX as usize {
                            return Err(CompileError::Unsupported(
                                "too many foreign symbols (>65535)",
                            ));
                        }
                        let idx = foreign_syms.len() as u16;
                        ffi_index.insert(d.resilient_name.clone(), idx);
                        foreign_syms.push(sym);
                    }
                }
            }
        }
    }
    // On non-ffi builds, ffi_index is empty — call sites fall through to
    // the normal fn_index lookup and surface a "function not found" error.
    #[cfg(not(feature = "ffi"))]
    let ffi_index: HashMap<String, u16> = HashMap::new();

    // RES-3993: trait name → default method bodies (method name, param
    // names incl. `self`, body), pre-scanned the same way ENUM_INDEX /
    // STRUCT_FIELD_INDEX are below. Mirrors the tree-walker's
    // `Node::TraitDecl` eval arm (`self.trait_method_defaults`), which
    // registers every trait method carrying a default body so a later
    // `ImplBlock` that doesn't override it can inject the default under
    // the impl's own `<Struct>$<method>` mangled name. The bytecode
    // compiler needs the same lookup available *before* fn_index's
    // pre-pass below, since a non-overriding `ImplBlock` must reserve a
    // fn_index/functions slot for the synthesized default just like any
    // other method.
    let mut trait_defaults: HashMap<String, Vec<(String, Vec<String>, Node)>> = HashMap::new();
    for spanned in stmts {
        if let Node::TraitDecl { name, methods, .. } = &spanned.node {
            let defaults: Vec<(String, Vec<String>, Node)> = methods
                .iter()
                .filter_map(|m| {
                    m.default_body
                        .as_ref()
                        .map(|body| (m.name.clone(), m.params.clone(), (**body).clone()))
                })
                .collect();
            if !defaults.is_empty() {
                trait_defaults.insert(name.clone(), defaults);
            }
        }
    }

    // RES-1461: pre-size `fn_index` and `functions` to the actual
    // top-level Function count. The previous shape used
    // `HashMap::new()` / `Vec::new()` and grew them entry-by-entry,
    // triggering reallocations as they crossed the default-bucket
    // boundaries. Most programs have at least a handful of functions;
    // a one-shot count is essentially free (linear over top-level
    // statements, same shape as the loop below). Mirrors RES-1365's
    // struct-fields pre-size pattern and RES-1399's actor
    // resolved_fields pre-size.
    //
    // RES-3993: an `ImplBlock` also contributes one slot per
    // non-overridden trait default it inherits — undercounting here
    // would leave `functions` too short for the synthesized defaults
    // pass 1/pass 2 (below) register/compile, panicking on the
    // `functions[top_idx] = ..` write.
    let fn_count = stmts
        .iter()
        .map(|s| match &s.node {
            Node::Function { .. } => 1,
            Node::ImplBlock {
                methods,
                struct_name,
                trait_name,
                ..
            } => {
                let explicit = methods
                    .iter()
                    .filter(|m| matches!(m, Node::Function { .. }))
                    .count();
                let inherited = trait_name
                    .as_ref()
                    .and_then(|t| trait_defaults.get(t))
                    .map(|defaults| {
                        let overridden: std::collections::HashSet<&str> = methods
                            .iter()
                            .filter_map(|m| {
                                if let Node::Function { name, .. } = m {
                                    name.strip_prefix(&format!("{struct_name}$"))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        defaults
                            .iter()
                            .filter(|(mname, ..)| !overridden.contains(mname.as_str()))
                            .count()
                    })
                    .unwrap_or(0);
                explicit + inherited
            }
            // RES-3993: `mod name { fn f() {..} }` — see the matching
            // fn_index/pass-2 arms below. Only directly-nested `fn`s are
            // counted; a `mod` containing an `ImplBlock` is rare enough
            // (no example exercises it) that it's left for a follow-up
            // rather than duplicating the trait-default machinery above
            // inside module scope too.
            Node::ModuleDecl { body, .. } => body
                .iter()
                .filter(|m| matches!(m, Node::Function { .. }))
                .count(),
            _ => 0,
        })
        .sum::<usize>();

    // Pre-pass: function name → index in the `functions` table.
    let mut fn_index: HashMap<String, u16> = HashMap::with_capacity(fn_count);
    let mut next_fn_idx: u16 = 0;
    for spanned in stmts {
        match &spanned.node {
            Node::Function {
                name, parameters, ..
            } => {
                if parameters.len() > u8::MAX as usize {
                    return Err(CompileError::Unsupported("fn with >255 params"));
                }
                if next_fn_idx == u16::MAX {
                    return Err(CompileError::Unsupported("program has > 65535 functions"));
                }
                fn_index.insert(name.clone(), next_fn_idx);
                next_fn_idx += 1;
            }
            Node::ImplBlock {
                methods,
                struct_name,
                trait_name,
                ..
            } => {
                let mut overridden: std::collections::HashSet<&str> =
                    std::collections::HashSet::new();
                for m in methods {
                    if let Node::Function {
                        name, parameters, ..
                    } = m
                    {
                        if parameters.len() > u8::MAX as usize {
                            return Err(CompileError::Unsupported("fn with >255 params"));
                        }
                        if next_fn_idx == u16::MAX {
                            return Err(CompileError::Unsupported("program has > 65535 functions"));
                        }
                        if let Some(bare) = name.strip_prefix(&format!("{struct_name}$")) {
                            overridden.insert(bare);
                        }
                        fn_index.insert(name.clone(), next_fn_idx);
                        next_fn_idx += 1;
                    }
                }
                // RES-3993: reserve a fn_index slot for every trait
                // default this impl block doesn't override — see
                // `trait_defaults` doc comment above. Pass 2 (below)
                // must synthesize+compile these in the exact same
                // per-ImplBlock order so `own_fn_idx` (assigned by
                // `top_idx`, which increments once per `compile_fn_body`
                // call) lines up with the index reserved here.
                if let Some(trait_nm) = trait_name
                    && let Some(defaults) = trait_defaults.get(trait_nm)
                {
                    for (method_name, _params, _body) in defaults {
                        if overridden.contains(method_name.as_str()) {
                            continue;
                        }
                        if next_fn_idx == u16::MAX {
                            return Err(CompileError::Unsupported("program has > 65535 functions"));
                        }
                        fn_index.insert(format!("{struct_name}${method_name}"), next_fn_idx);
                        next_fn_idx += 1;
                    }
                }
            }
            // RES-3993: `mod name { fn f(..) {..} }` — the tree-walker
            // (`crate::modules::eval_module`) registers every directly
            // nested `fn` under the prefixed key `"<mod>::<fn>"`, and the
            // parser already collapses a `math::add(..)` call site into
            // `Node::Identifier { name: "math::add" }` — so registering
            // fn_index under that same prefixed key is enough to make
            // `math::add(3, 4)` resolve through the ordinary
            // `fn_index.get(callee_name)` call path with no further
            // compiler changes.
            Node::ModuleDecl {
                name: mod_name,
                body,
                ..
            } => {
                for m in body {
                    if let Node::Function {
                        name, parameters, ..
                    } = m
                    {
                        if parameters.len() > u8::MAX as usize {
                            return Err(CompileError::Unsupported("fn with >255 params"));
                        }
                        if next_fn_idx == u16::MAX {
                            return Err(CompileError::Unsupported("program has > 65535 functions"));
                        }
                        fn_index.insert(format!("{mod_name}::{name}"), next_fn_idx);
                        next_fn_idx += 1;
                    }
                }
            }
            _ => {}
        }
    }

    // Pre-scan enum declarations so CallExpression / StructLiteral can
    // resolve payload constructors like `Option::Some(x)` or
    // `Result::Err { msg: "..." }`.
    ENUM_INDEX.with(|ei| {
        let mut idx = ei.borrow_mut();
        idx.clear();
        for spanned in stmts {
            if let Node::EnumDecl { name, variants, .. } = &spanned.node {
                idx.insert(name.clone(), variants.clone());
            }
        }
    });

    // RES-3994: pre-scan struct declarations so `Node::StructLiteral`'s
    // `{ ..base, f: v }` struct-update syntax can resolve which fields
    // `base` must supply (see `STRUCT_FIELD_INDEX` doc comment above).
    STRUCT_FIELD_INDEX.with(|si| {
        let mut idx = si.borrow_mut();
        idx.clear();
        for spanned in stmts {
            if let Node::StructDecl { name, fields, .. } = &spanned.node {
                idx.insert(
                    name.clone(),
                    fields.iter().map(|(_ty, fname)| fname.clone()).collect(),
                );
            }
        }
    });

    // RES-2532: pre-scan top-level `let` bindings so function bodies
    // can reference them via LoadGlobal / StoreGlobal.
    let globals = prescan_globals(stmts);

    // Pass 2: compile each function body in declaration order.
    // RES-2538: pre-allocate slots for all top-level functions so that
    // nested function definitions (compiled during body traversal) get
    // indices *after* the top-level range. This keeps fn_index (pass 1)
    // valid even when nested fns are pushed to the Vec during compilation.
    let mut functions: Vec<Function> = Vec::with_capacity(fn_count);
    let placeholder = || Function {
        name: String::new(),
        arity: 0,
        chunk: Chunk::with_capacity(0),
        local_count: 0,
        upvalue_source_slots: Box::default(),
        fails: Box::default(),
        postcheck: None,
    };
    for _ in 0..fn_count {
        functions.push(placeholder());
    }
    let mut top_idx: usize = 0;
    let mut compile_fn_body = |name: &str,
                               parameters: &[(String, String)],
                               body: &Node,
                               fn_line: u32,
                               fails: Box<[String]>,
                               ensures: &[Node],
                               recovers_to: &Option<Box<Node>>,
                               functions: &mut Vec<Function>,
                               next_fn_idx: &mut u16|
     -> Result<(), CompileError> {
        let arity = parameters.len() as u8;
        let mut chunk = Chunk::with_capacity(128);
        let cap = parameters.len().saturating_mul(2).max(8) + globals.len();
        let mut locals: HashMap<String, u16> = HashMap::with_capacity(cap);
        for (gname, &gslot) in &globals {
            locals.insert(gname.clone(), gslot | GLOBAL_FLAG);
        }
        let mut next_local: u16 = 0;
        for (_type_name, pname) in parameters {
            locals.insert(pname.clone(), next_local);
            next_local += 1;
        }
        let inner = match body {
            Node::Block { stmts: b, .. } => b.as_slice(),
            single => std::slice::from_ref(single),
        };
        compile_fn_body_stmts(
            inner,
            &mut chunk,
            &mut locals,
            &mut next_local,
            &fn_index,
            &ffi_index,
            functions,
            next_fn_idx,
            fn_line,
        )?;
        chunk.emit(Op::ReturnFromCall, 0);
        let own_fn_idx = top_idx as u16;
        // RES-4017: only functions in a `#[mutual_tail_call]` group may
        // tail-call into a *different* function's frame; everything
        // else keeps the RES-384 self-recursion-only behavior (empty
        // set here means `rewrite_tail_calls` only ever matches
        // `own_fn_idx`).
        let mutual_targets = if crate::mutual_tco::is_mutual_tail_call(name) {
            crate::mutual_tco::mutual_tail_call_indices(&fn_index)
        } else {
            std::collections::HashSet::new()
        };
        rewrite_tail_calls(&mut chunk, own_fn_idx, &mutual_targets);
        crate::const_fold::optimize_if_enabled(&mut chunk)
            .map_err(|_| CompileError::InternalError("constant folder failed"))?;
        crate::peephole::optimize(&mut chunk)
            .map_err(|_| CompileError::InternalError("peephole optimizer failed"))?;
        crate::dce::eliminate(&mut chunk);
        // RES-4041: synthesize the postcondition-check function (if any)
        // *before* recording this fn's own entry — the postcheck may
        // itself push nested functions onto `functions` (e.g. a clause
        // calling another fn), and this fn's own slot (`top_idx`) is
        // already reserved regardless of how many entries get appended
        // after it.
        let postcheck = build_postcheck_function(
            name,
            parameters,
            ensures,
            recovers_to,
            &fn_index,
            &ffi_index,
            functions,
            next_fn_idx,
            fn_line,
        )?;
        functions[top_idx] = Function {
            name: name.to_string(),
            arity,
            chunk,
            local_count: next_local,
            upvalue_source_slots: Box::default(),
            fails,
            postcheck,
        };
        top_idx += 1;
        Ok(())
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function {
                name,
                parameters,
                body,
                fails,
                ensures,
                recovers_to,
                ..
            } => {
                compile_fn_body(
                    name,
                    parameters,
                    body,
                    spanned.span.start.line as u32,
                    fails.clone().into_boxed_slice(),
                    ensures,
                    recovers_to,
                    &mut functions,
                    &mut next_fn_idx,
                )?;
            }
            Node::ImplBlock {
                methods,
                struct_name,
                trait_name,
                ..
            } => {
                let mut overridden: std::collections::HashSet<&str> =
                    std::collections::HashSet::new();
                for m in methods {
                    if let Node::Function {
                        name,
                        parameters,
                        body,
                        ensures,
                        recovers_to,
                        ..
                    } = m
                    {
                        if let Some(bare) = name.strip_prefix(&format!("{struct_name}$")) {
                            overridden.insert(bare);
                        }
                        let line = node_line(m).unwrap_or(spanned.span.start.line as u32);
                        compile_fn_body(
                            name,
                            parameters,
                            body,
                            line,
                            Box::default(),
                            ensures,
                            recovers_to,
                            &mut functions,
                            &mut next_fn_idx,
                        )?;
                    }
                }
                // RES-3993: synthesize+compile every non-overridden
                // trait default — see the matching fn_index reservation
                // (pass 1, above) and the `trait_defaults` doc comment.
                // Iterates the identical `trait_defaults[trait_nm]`
                // sequence with the identical `overridden` filter so
                // `top_idx` advances in lockstep with the slots pass 1
                // reserved.
                if let Some(trait_nm) = trait_name
                    && let Some(defaults) = trait_defaults.get(trait_nm)
                {
                    for (method_name, params, body) in defaults {
                        if overridden.contains(method_name.as_str()) {
                            continue;
                        }
                        let mangled = format!("{struct_name}${method_name}");
                        let param_pairs: Vec<(String, String)> =
                            params.iter().map(|p| (String::new(), p.clone())).collect();
                        let line = node_line(body).unwrap_or(spanned.span.start.line as u32);
                        compile_fn_body(
                            &mangled,
                            &param_pairs,
                            body,
                            line,
                            Box::default(),
                            &[],
                            &None,
                            &mut functions,
                            &mut next_fn_idx,
                        )?;
                    }
                }
            }
            // RES-3993: compile each nested `mod` function under its
            // `"<mod>::<fn>"` mangled name — see the matching fn_index
            // reservation (pass 1, above) for the full rationale.
            Node::ModuleDecl {
                name: mod_name,
                body,
                ..
            } => {
                for m in body {
                    if let Node::Function {
                        name,
                        parameters,
                        body,
                        ensures,
                        recovers_to,
                        ..
                    } = m
                    {
                        let mangled = format!("{mod_name}::{name}");
                        let line = node_line(m).unwrap_or(spanned.span.start.line as u32);
                        compile_fn_body(
                            &mangled,
                            parameters,
                            body,
                            line,
                            Box::default(),
                            ensures,
                            recovers_to,
                            &mut functions,
                            &mut next_fn_idx,
                        )?;
                    }
                }
            }
            _ => {}
        }
    }

    // Pass 3: compile the remaining top-level statements into `main`.
    // RES-1720: pre-size — top-level body usually emits a handful of
    // setup ops + per-stmt calls. 64 fits the common case.
    let mut main = Chunk::with_capacity(64);
    // RES-1716: pre-size `main_locals` — same shape as RES-1461 for
    // `fn_index`. Top-level `let` / `const` / `static` bindings flow
    // into this map; typical programs have 5-20 entries. Pre-sizing
    // to 16 fits the common case in one allocation.
    let mut main_locals: HashMap<String, u16> = HashMap::with_capacity(16);
    let mut main_next_local: u16 = 0;
    // Skip fn/extern decls — handled in earlier passes.
    // RES-391: `region <Name>;` is compile-time metadata only;
    // it emits no code in either the tree-walker or the VM.
    // RES-335: `struct <Name> { ... }` decls are likewise
    // compile-time metadata — the `StructLiteral` opcode carries
    // the type name directly and does not consult a decl table.
    //
    // RES-3997: collect the surviving statements first (rather than
    // compiling in the same loop) so `compile_top_level_stmts` can split
    // off the trailing one — a bare top-level expression-statement with
    // nothing after it is the program's implicit result value (what
    // `vm::run` returns, and what `--vm`'s CLI driver prints when
    // non-Void), exactly mirroring how a function body's trailing bare
    // expression is its implicit return value. Every *other* statement
    // is compiled normally and has its value popped if unused.
    let top_level_stmts: Vec<(&Node, u32)> = stmts
        .iter()
        .filter(|spanned| {
            !matches!(
                spanned.node,
                Node::Function { .. }
                    | Node::Extern { .. }
                    | Node::RegionDecl { .. }
                    | Node::StructDecl { .. }
                    | Node::ImplBlock { .. }
                    | Node::NewtypeDecl { .. }
            )
        })
        .map(|spanned| (&spanned.node, spanned.span.start.line as u32))
        .collect();
    compile_top_level_stmts(
        &top_level_stmts,
        &mut main,
        &mut main_locals,
        &mut main_next_local,
        &fn_index,
        &ffi_index,
        &mut functions,
        &mut next_fn_idx,
    )?;
    main.emit(Op::Return, 0);
    // RES-298: constant fold the main chunk before peephole runs.
    crate::const_fold::optimize_if_enabled(&mut main)
        .map_err(|_| CompileError::InternalError("constant folder failed"))?;
    // RES-172: peephole pass over the main chunk too.
    crate::peephole::optimize(&mut main)
        .map_err(|_| CompileError::InternalError("peephole optimizer failed"))?;
    // RES-297: dead code elimination over main chunk.
    crate::dce::eliminate(&mut main);

    let mut prog = Program {
        main,
        functions,
        #[cfg(feature = "ffi")]
        foreign_syms,
    };
    // RES-365: function inlining pass over the assembled program.
    // Replaces `Op::Call(idx)` to small leaf functions with the
    // callee's bytecode body, eliminating call-frame overhead. Gated
    // behind `RESILIENT_INLINE=1` so default behavior stays
    // bit-identical (matches the `const_fold::optimize_if_enabled`
    // discipline — the existing test suite pins specific opcode
    // sequences that inlining would change).
    crate::inline::optimize_if_enabled(&mut prog)
        .map_err(|_| CompileError::InternalError("inliner failed"))?;
    Ok(prog)
}

/// RES-3992: resolve every top-level `const NAME = expr;` declaration
/// into its literal `Value`, mirroring `Interpreter::const_eval_program`
/// (the tree-walker's canonical const-resolution pre-pass — see RES-361).
/// Reuses `Interpreter::eval_const_expr` directly so the bytecode
/// compiler can never diverge from the tree-walker's notion of what a
/// "compile-time constant" is.
///
/// A const whose value expression isn't foldable (circular reference,
/// non-constant sub-expression) is silently left out of the returned
/// map rather than erroring here — `static_assert::check` in the shared
/// typechecker pass already surfaces that as a diagnostic before either
/// backend runs, so skipping it just means a later reference to that
/// name falls through to the pre-existing `UnknownIdentifier` error
/// instead of this pre-pass raising a second, redundant one.
fn resolve_top_level_consts(stmts: &[crate::span::Spanned<Node>]) -> HashMap<String, Value> {
    let mut resolved: HashMap<String, Value> = HashMap::new();
    for stmt in stmts {
        let Node::Const { name, value, .. } = &stmt.node else {
            continue;
        };
        let mut evaluating = vec![name.clone()];
        if let Ok(v) = crate::Interpreter::eval_const_expr(value, &resolved, &mut evaluating) {
            resolved.insert(name.clone(), v);
        }
    }
    resolved
}

/// RES-3992: convert a resolved const `Value` back into the literal AST
/// node the bytecode compiler already knows how to emit a `Op::Const`
/// for. Consts only ever resolve to one of these scalar shapes — see
/// the allowed-subexpression list on `Interpreter::eval_const_expr` —
/// so `None` here is unreachable in practice, but is handled instead of
/// unwrapped to keep this pass a no-op on any future const value shape
/// rather than a panic.
fn const_value_to_literal(value: &Value, span: crate::span::Span) -> Option<Node> {
    match value {
        Value::Int(v) => Some(Node::IntegerLiteral { value: *v, span }),
        Value::Float(v) => Some(Node::FloatLiteral { value: *v, span }),
        Value::Bool(v) => Some(Node::BooleanLiteral { value: *v, span }),
        Value::String(s) => Some(Node::StringLiteral {
            value: s.clone(),
            span,
        }),
        Value::Char(c) => Some(Node::CharLiteral { value: *c, span }),
        _ => None,
    }
}

/// RES-3992: rewrite every `Node::Identifier` that names a resolved
/// top-level const into its literal value, everywhere in the AST. This
/// mirrors the tree-walker's `Node::Identifier` eval arm, which checks
/// `self.consts` *before* the local environment (RES-361) — so a const
/// reference is inlined here unconditionally, without tracking local
/// shadowing, to stay byte-for-byte consistent with that oracle.
///
/// Structural coverage follows the same partial-recursion shape as
/// `devirtualize::rewrite_node`: every statement/expression kind that
/// can plausibly carry a const reference in the differential corpus is
/// handled explicitly; declaration-only forms (struct/enum/trait decls,
/// `use`, etc.) that can never contain one pass through unchanged via
/// the catch-all.
fn inline_consts(node: &Node, resolved: &HashMap<String, Value>) -> Node {
    match node {
        Node::Identifier { name, span } => resolved
            .get(name)
            .and_then(|v| const_value_to_literal(v, *span))
            .unwrap_or_else(|| node.clone()),
        Node::Program(stmts) => Node::Program(
            stmts
                .iter()
                .map(|s| crate::span::Spanned::new(inline_consts(&s.node, resolved), s.span))
                .collect(),
        ),
        Node::Function {
            name,
            parameters,
            defaults,
            body,
            requires,
            ensures,
            return_type,
            span,
            pure,
            effects,
            type_params,
            type_param_bounds,
            fails,
            recovers_to,
            is_pub,
        } => Node::Function {
            name: name.clone(),
            parameters: parameters.clone(),
            defaults: defaults.clone(),
            body: Box::new(inline_consts(body, resolved)),
            requires: requires
                .iter()
                .map(|r| inline_consts(r, resolved))
                .collect(),
            ensures: ensures.iter().map(|e| inline_consts(e, resolved)).collect(),
            return_type: return_type.clone(),
            span: *span,
            pure: *pure,
            effects: *effects,
            type_params: type_params.clone(),
            type_param_bounds: type_param_bounds.clone(),
            fails: fails.clone(),
            recovers_to: recovers_to
                .as_ref()
                .map(|r| Box::new(inline_consts(r, resolved))),
            is_pub: *is_pub,
        },
        Node::Block { stmts, span } => Node::Block {
            stmts: stmts.iter().map(|s| inline_consts(s, resolved)).collect(),
            span: *span,
        },
        Node::LetStatement {
            name,
            value,
            type_annot,
            span,
            is_const,
        } => Node::LetStatement {
            name: name.clone(),
            value: Box::new(inline_consts(value, resolved)),
            type_annot: type_annot.clone(),
            span: *span,
            is_const: *is_const,
        },
        Node::ExpressionStatement { expr, span } => Node::ExpressionStatement {
            expr: Box::new(inline_consts(expr, resolved)),
            span: *span,
        },
        Node::ReturnStatement { value, span } => Node::ReturnStatement {
            value: value.as_ref().map(|v| Box::new(inline_consts(v, resolved))),
            span: *span,
        },
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            span,
        } => Node::IfStatement {
            condition: Box::new(inline_consts(condition, resolved)),
            consequence: Box::new(inline_consts(consequence, resolved)),
            alternative: alternative
                .as_ref()
                .map(|a| Box::new(inline_consts(a, resolved))),
            span: *span,
        },
        Node::WhileStatement {
            condition,
            body,
            invariants,
            span,
            label,
        } => Node::WhileStatement {
            condition: Box::new(inline_consts(condition, resolved)),
            body: Box::new(inline_consts(body, resolved)),
            invariants: invariants
                .iter()
                .map(|i| inline_consts(i, resolved))
                .collect(),
            span: *span,
            label: label.clone(),
        },
        Node::ForInStatement {
            name,
            iterable,
            body,
            invariants,
            span,
            label,
        } => Node::ForInStatement {
            name: name.clone(),
            iterable: Box::new(inline_consts(iterable, resolved)),
            body: Box::new(inline_consts(body, resolved)),
            invariants: invariants
                .iter()
                .map(|i| inline_consts(i, resolved))
                .collect(),
            span: *span,
            label: label.clone(),
        },
        Node::Assignment { name, value, span } => Node::Assignment {
            name: name.clone(),
            value: Box::new(inline_consts(value, resolved)),
            span: *span,
        },
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Node::InfixExpression {
            left: Box::new(inline_consts(left, resolved)),
            operator,
            right: Box::new(inline_consts(right, resolved)),
            span: *span,
        },
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator,
            right: Box::new(inline_consts(right, resolved)),
            span: *span,
        },
        Node::CallExpression {
            function,
            arguments,
            span,
        } => Node::CallExpression {
            function: Box::new(inline_consts(function, resolved)),
            arguments: arguments
                .iter()
                .map(|a| inline_consts(a, resolved))
                .collect(),
            span: *span,
        },
        Node::ArrayLiteral { items, span } => Node::ArrayLiteral {
            items: items.iter().map(|i| inline_consts(i, resolved)).collect(),
            span: *span,
        },
        Node::TupleLiteral { items, span } => Node::TupleLiteral {
            items: items.iter().map(|i| inline_consts(i, resolved)).collect(),
            span: *span,
        },
        Node::IndexExpression {
            target,
            index,
            span,
        } => Node::IndexExpression {
            target: Box::new(inline_consts(target, resolved)),
            index: Box::new(inline_consts(index, resolved)),
            span: *span,
        },
        Node::FieldAccess {
            target,
            field,
            span,
        } => Node::FieldAccess {
            target: Box::new(inline_consts(target, resolved)),
            field: field.clone(),
            span: *span,
        },
        Node::StructLiteral {
            name,
            fields,
            base,
            span,
        } => Node::StructLiteral {
            name: name.clone(),
            fields: fields
                .iter()
                .map(|(k, v)| (k.clone(), inline_consts(v, resolved)))
                .collect(),
            base: base.as_ref().map(|b| Box::new(inline_consts(b, resolved))),
            span: *span,
        },
        Node::ImplBlock {
            trait_name,
            struct_name,
            methods,
            span,
            associated_type_impls,
        } => Node::ImplBlock {
            trait_name: trait_name.clone(),
            struct_name: struct_name.clone(),
            methods: methods.iter().map(|m| inline_consts(m, resolved)).collect(),
            span: *span,
            associated_type_impls: associated_type_impls.clone(),
        },
        Node::Match {
            scrutinee,
            arms,
            span,
        } => Node::Match {
            scrutinee: Box::new(inline_consts(scrutinee, resolved)),
            arms: arms
                .iter()
                .map(|(pat, guard, body)| {
                    (
                        pat.clone(),
                        guard.as_ref().map(|g| inline_consts(g, resolved)),
                        inline_consts(body, resolved),
                    )
                })
                .collect(),
            span: *span,
        },
        other => other.clone(),
    }
}

/// RES-3914: compile `name = value;`, shared by `compile_stmt` (main
/// chunk) and `compile_stmt_in_fn` (function bodies). If `name`'s slot
/// is boxed (captured by mutable upvalue — see `BOXED_FLAG`), the
/// write routes through `Cell.set()` so every closure sharing the
/// capture — and the defining scope itself — observes the mutation.
/// `Cell.set()` returns `Value::Void`, which is discarded into a
/// scratch local so the assignment stays stack-neutral, matching the
/// plain `StoreLocal`/`StoreGlobal` path.
#[allow(clippy::too_many_arguments)]
fn compile_assignment(
    name: &str,
    value: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    let idx = *locals
        .get(name)
        .ok_or_else(|| CompileError::UnknownIdentifier(name.to_string()))?;
    if idx & BOXED_FLAG != 0 {
        chunk.emit(Op::LoadLocal(raw_slot(idx)), line);
        compile_expr(
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        let method_const = chunk.add_string_constant("set")?;
        chunk.emit(
            Op::CallMethod {
                method_const,
                arity: 1,
            },
            line,
        );
        if *next_local == u16::MAX {
            return Err(CompileError::TooManyLocals);
        }
        let scratch = *next_local;
        *next_local += 1;
        chunk.emit(Op::StoreLocal(scratch), line);
    } else {
        compile_expr(
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        chunk.emit(local_store_op(idx), line);
    }
    Ok(())
}

/// Compile a top-level (main-chunk) statement. Bare expression
/// statements leak their value onto the operand stack, which `Return`
/// picks up as the program result — useful for the RES-076 smoke
/// test that parses `2 + 3 * 4;`.
///
/// `loop_stack` holds the stack of enclosing loop states so that labeled
/// break/continue can target an outer loop by name.
#[allow(clippy::too_many_arguments)]
fn compile_stmt(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_stack: &mut Vec<LoopState>,
) -> Result<(), CompileError> {
    match node {
        Node::LetStatement { name, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-401: `let (a, b, c) = expr;` in top-level (main chunk).
        Node::LetTupleDestructure { names, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let tmp_idx = *next_local;
            *next_local += 1;
            chunk.emit(Op::StoreLocal(tmp_idx), line);
            for (i, name) in names.iter().enumerate() {
                if *next_local == u16::MAX {
                    return Err(CompileError::TooManyLocals);
                }
                let slot = *next_local;
                *next_local += 1;
                locals.insert(name.clone(), slot);
                chunk.emit(Op::LoadLocal(tmp_idx), line);
                let idx_const = chunk.add_constant(Value::Int(i as i64))?;
                chunk.emit(Op::Const(idx_const), line);
                chunk.emit(Op::LoadIndex, line);
                chunk.emit(Op::StoreLocal(slot), line);
            }
            Ok(())
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            compile_expr(
                v,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Return, line);
            Ok(())
        }
        Node::ReturnStatement { value: None, .. } => {
            chunk.emit(Op::Return, line);
            Ok(())
        }
        // RES-3997: same discard-and-pop treatment as the in-fn path
        // below — a top-level bare `expr;` statement must not leak its
        // value onto the shared operand stack either.
        Node::ExpressionStatement { expr: inner, .. } => {
            compile_expr(
                inner,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Pop, line);
            Ok(())
        }
        Node::IfStatement { .. }
        | Node::WhileStatement { .. }
        | Node::ForInStatement { .. }
        | Node::Block { .. } => compile_control_flow(
            node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        ),
        Node::Assignment { name, value, .. } => compile_assignment(
            name,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-171a/RES-171c: `a[i] = v` and `a[i0][i1]...[iN] = v`.
        // Depth-1 lowering: LoadLocal(a), <i>, <v>, StoreIndex, StoreLocal(a).
        // Depth-N lowering: temp-local staging through compile_index_assignment.
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => compile_index_assignment(
            target,
            index,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-335: `p.field = v;` where `p` is a bare Identifier.
        // Lowered as:
        //   LoadLocal(p), <v>, SetField { field }, StoreLocal(p)
        // The struct on top of the stack after `SetField` IS the
        // mutated one (VM dispatch pushes it back), so writing it
        // through `StoreLocal` commits the update. Mirrors the
        // `IndexAssignment` lowering.
        Node::FieldAssignment {
            target,
            field,
            value,
            ..
        } => {
            // RES-1430: borrow target name as &str — see comment on
            // the IndexAssignment arm above.
            let local_name: &str = match target.as_ref() {
                Node::Identifier { name, .. } => name.as_str(),
                _ => {
                    return Err(CompileError::Unsupported(
                        "nested field assignment (non-identifier target)",
                    ));
                }
            };
            let slot = *locals
                .get(local_name)
                .ok_or_else(|| CompileError::UnknownIdentifier(local_name.to_string()))?;
            chunk.emit(local_load_op(slot), line);
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let fname_idx = chunk.add_string_constant(field)?;
            chunk.emit(
                Op::SetField {
                    name_const: fname_idx,
                },
                line,
            );
            chunk.emit(local_store_op(slot), line);
            Ok(())
        }
        Node::Break { .. } => {
            let ls = loop_stack
                .last_mut()
                .ok_or(CompileError::Unsupported("break outside loop"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.break_patches.push(patch);
            Ok(())
        }
        // RES-3993: `break <expr>;` — always evaluate `expr` (matching the
        // tree-walker's `Node::BreakWith` eval, which runs unconditionally
        // regardless of whether the enclosing loop's own value is used).
        // If the innermost loop is compiled in expression position
        // (`value_mode`, set only by `compile_while_expr`), leave the
        // value on the stack and jump to the loop's value-carrying exit.
        // Otherwise (the common statement-position loop) the loop's
        // result is never read, so pop the value immediately — same
        // stack-neutral shape as a plain `break;` — and jump to the
        // ordinary `break_patches` exit.
        Node::BreakWith { value, .. } => {
            let value_mode = loop_stack
                .last()
                .ok_or(CompileError::Unsupported("break outside loop"))?
                .value_mode;
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let ls = loop_stack.last_mut().unwrap();
            if value_mode {
                let patch = chunk.emit(Op::Jump(0), line);
                ls.break_value_patches.push(patch);
            } else {
                chunk.emit(Op::Pop, line);
                let patch = chunk.emit(Op::Jump(0), line);
                ls.break_patches.push(patch);
            }
            Ok(())
        }
        Node::Continue { .. } => {
            let ls = loop_stack
                .last_mut()
                .ok_or(CompileError::Unsupported("continue outside loop"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.continue_patches.push(patch);
            Ok(())
        }
        Node::BreakLabel { label, .. } => {
            let ls = loop_stack
                .iter_mut()
                .rev()
                .find(|ls| ls.label.as_deref() == Some(label.as_str()))
                .ok_or(CompileError::Unsupported("break label not found"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.break_patches.push(patch);
            Ok(())
        }
        Node::ContinueLabel { label, .. } => {
            let ls = loop_stack
                .iter_mut()
                .rev()
                .find(|ls| ls.label.as_deref() == Some(label.as_str()))
                .ok_or(CompileError::Unsupported("continue label not found"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.continue_patches.push(patch);
            Ok(())
        }
        // RES-break-continue: `assert cond[, msg];` — evaluate the
        // condition; if falsy push the message and fail. Lowered as:
        //   <cond>
        //   JumpIfTrue(past_fail)
        //   Const(msg)
        //   AssertFail
        // past_fail:
        Node::Assert {
            condition, message, ..
        } => compile_assert(
            condition,
            message,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        Node::Function {
            name,
            parameters,
            body,
            ensures,
            recovers_to,
            ..
        } => compile_nested_fn(
            name,
            parameters,
            body,
            ensures,
            recovers_to,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        Node::Extern { .. } => Err(CompileError::Unsupported("nested extern decl")),
        // RES-390: actor / cluster decls are compile-time-only
        // verifier constructs. The bytecode backend emits nothing
        // for them — the interpreter also treats them as no-ops.
        Node::ActorDecl { .. } | Node::ClusterDecl { .. } => Ok(()),
        // RES-155: `let StructName { field, other_field: local } = expr;`
        // Compile the value, store in a temp slot, then emit
        // GetField + StoreLocal for each (field_name, local_name) pair.
        Node::LetDestructureStruct { fields, value, .. } => compile_let_destructure_struct(
            fields,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-384b: `static let NAME = EXPR;` — the VM has no separate
        // statics store; compile as a regular local binding. The
        // "initialize only once" semantic is not preserved in bytecode
        // (single-execution model), but the value is accessible by name.
        Node::StaticLet { name, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-361: `const NAME = EXPR;` is pre-evaluated by the const_eval
        // pass before bytecode compilation. Nothing to emit at runtime.
        Node::Const { .. } => Ok(()),
        // RES-2660: static_assert is evaluated at compile time. No-op in codegen.
        Node::StaticAssert { .. } => Ok(()),
        // RES-3995: `live { body }` — full retry/backoff/invariant/timeout
        // semantics, matching the tree-walker's `eval_live_block`. See
        // `compile_live_block` for the bytecode shape and `vm::run_inner`
        // for the retry-loop execution.
        Node::LiveBlock {
            body,
            invariants,
            backoff,
            backoff_kind,
            timeout,
            max_retries,
            ..
        } => compile_live_block(
            body,
            invariants,
            backoff,
            *backoff_kind,
            timeout,
            *max_retries,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
            false,
        ),
        // RES-4024: the MMIO-wrapper block keyword lifts the typechecker's
        // capability gate on volatile MMIO intrinsics but is otherwise
        // identical to a plain block at runtime (see `parse_unsafe_block`'s
        // doc comment and the tree-walker's `Node::UnsafeBlock =>
        // self.eval(body)`). Compile the body exactly like `LiveBlock` —
        // previously this was grouped with the declaration-only nodes
        // below and silently dropped, so `--vm` skipped the entire block
        // body.
        Node::UnsafeBlock { body, .. } => compile_stmt(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        ),
        // RES-3996: `assume(cond[, msg]);` halts at runtime like `assert`
        // when the condition is false (see `compile_assume`). Previously
        // grouped with the declaration-only no-op arm below, which
        // silently dropped the runtime check under `--vm`.
        Node::Assume {
            condition, message, ..
        } => compile_assume(
            condition,
            message,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // Verification-only construct: emit nothing at runtime.
        Node::InvariantStatement { .. } => Ok(()),
        // Type-level / declaration-only constructs: no runtime bytecode.
        // All type information is handled at parse/typecheck time.
        Node::EnumDecl { name, variants, .. } => {
            emit_unit_enum_variants(name, variants, chunk, locals, next_local, line)
        }
        Node::StructDecl { .. }
        | Node::TraitDecl { .. }
        | Node::ImplBlock { .. }
        // RES-2689: BlanketImpl is declaration-only; lower_program already
        // injected the concrete ImplBlocks before compilation reaches here.
        | Node::BlanketImpl { .. }
        | Node::TypeAlias { .. }
        | Node::NewtypeDecl { .. }
        | Node::RegionDecl { .. }
        | Node::SupervisorDecl { .. }
        | Node::ModuleDecl { .. }
        | Node::Use { .. } => Ok(()),
        // RES-3993: `bench "name" { ... }` blocks are silently skipped during
        // normal program execution — the tree-walker's `Node::BenchBlock` arm
        // is a bare `Ok(Value::Void)` no-op, since bench bodies are collected
        // and run separately by the `rz bench` subcommand, not by `rz`/
        // `rz --vm`. Mirror that here rather than falling through to the
        // generic `Unsupported` catch-all below.
        Node::BenchBlock { .. } => Ok(()),
        Node::TryCatch { body, handlers, .. } => compile_try_catch(
            body,
            handlers,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
            false,
        ),
        // RES-3993: `if let` / `while let` desugar to a bare `Node::Match`
        // statement (see `compile_match_stmt`'s doc comment) — route it
        // the same way `IfStatement`/`WhileStatement` are routed above.
        Node::Match {
            scrutinee, arms, ..
        } => compile_match_stmt(
            scrutinee,
            arms,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        ),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// RES-2544: compile a `try { body } catch Variant { handler }` block.
#[allow(clippy::too_many_arguments)]
fn compile_try_catch(
    body: &[Node],
    handlers: &[(String, Vec<Node>)],
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_stack: &mut Vec<LoopState>,
    in_fn: bool,
) -> Result<(), CompileError> {
    let arms: Vec<CatchArm> = handlers
        .iter()
        .map(|(variant, _)| CatchArm {
            variant: variant.clone(),
            handler_pc: 0,
        })
        .collect();
    let handler_idx = chunk.add_try_handler(arms)?;
    chunk.emit(Op::EnterTry(handler_idx), line);

    for stmt in body {
        let stmt_line = node_line(stmt).unwrap_or(line);
        if in_fn {
            compile_stmt_in_fn(
                stmt,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                stmt_line,
                loop_stack,
            )?;
        } else {
            compile_stmt(
                stmt,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                stmt_line,
                loop_stack,
            )?;
        }
    }
    chunk.emit(Op::ExitTry, line);
    let jmp_end = chunk.emit(Op::Jump(0), line);

    let mut end_jumps = vec![jmp_end];
    for (arm_idx, (_, handler_body)) in handlers.iter().enumerate() {
        let handler_pc = chunk.code.len();
        chunk.patch_try_handler(handler_idx, arm_idx, handler_pc);
        for stmt in handler_body {
            let stmt_line = node_line(stmt).unwrap_or(line);
            if in_fn {
                compile_stmt_in_fn(
                    stmt,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    stmt_line,
                    loop_stack,
                )?;
            } else {
                compile_stmt(
                    stmt,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    stmt_line,
                    loop_stack,
                )?;
            }
        }
        let jmp = chunk.emit(Op::Jump(0), line);
        end_jumps.push(jmp);
    }

    let end_pc = chunk.code.len();
    for jmp in end_jumps {
        chunk.patch_jump(jmp, end_pc)?;
    }
    Ok(())
}

/// RES-3995: `live { ... }` / `live invariant EXPR { ... }` lowering.
///
/// Emits `EnterLive(handler_idx)`, then the body, then each invariant
/// clause as an `assert`-shaped check (a false invariant fails exactly
/// like a body-level error), then `ExitLive`. The retry/backoff/
/// timeout/invariant *semantics* all live in `vm::run_inner`, which
/// recursively re-runs the bytecode between `EnterLive` and `ExitLive`
/// on any error until the block's retry budget is exhausted — mirrors
/// the tree-walker's `eval_live_block` exactly (same retry-count
/// arithmetic, same env-snapshot-and-restore-on-retry behavior). This
/// function only emits the *static* bytecode shape; it does not know
/// or care how many times the VM ends up executing it.
#[allow(clippy::too_many_arguments)]
fn compile_live_block(
    body: &Node,
    invariants: &[Node],
    backoff: &Option<crate::BackoffConfig>,
    backoff_kind: crate::BackoffKind,
    timeout: &Option<Box<Node>>,
    max_retries: Option<u32>,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_stack: &mut Vec<LoopState>,
    in_fn: bool,
) -> Result<(), CompileError> {
    // RES-142: unpack the `within <duration>` clause into a flat `u64`
    // ns budget — mirrors the tree-walker's `Node::LiveBlock` eval arm.
    let timeout_ns = timeout.as_ref().and_then(|n| match n.as_ref() {
        Node::DurationLiteral { nanos, .. } => Some(*nanos),
        _ => None,
    });
    let handler_idx = chunk.add_live_handler(LiveHandlerEntry {
        body_start_pc: 0,
        max_retries: max_retries.unwrap_or(crate::DEFAULT_LIVE_MAX_RETRIES),
        backoff: *backoff,
        backoff_kind,
        timeout_ns,
    })?;
    let enter_pc = chunk.emit(Op::EnterLive(handler_idx), line);
    chunk.set_live_handler_body_start(handler_idx, enter_pc + 1);

    if in_fn {
        compile_stmt_in_fn(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        )?;
    } else {
        compile_stmt(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        )?;
    }

    // RES-036: a failing invariant retries exactly like a body error —
    // lower each clause as a plain assert positioned after the body,
    // still between EnterLive/ExitLive so the retry-catch mechanism in
    // the VM sees it the same way it sees any other runtime error.
    for inv in invariants {
        let inv_line = node_line(inv).unwrap_or(line);
        compile_assert(
            inv,
            &None,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            inv_line,
        )?;
    }

    chunk.emit(Op::ExitLive, line);
    Ok(())
}

/// Shared assert-lowering logic used by both `compile_stmt` and
/// RES-2540: bind unit enum variants as locals so `Color::Green` resolves.
/// RES-3916: if `name` is a `Type::Variant` reference to a **zero-arg**
/// enum variant recorded in `ENUM_INDEX`, intern its `Value::EnumVariant`
/// constant and return the pool index. Returns `Ok(None)` when `name`
/// isn't a qualified zero-arg variant, so the caller can fall through to
/// its normal unknown-identifier path. Mirrors the tuple/named
/// constructor lookups in the `CallExpression` / `StructLiteral` arms.
fn zero_arg_enum_variant_const(name: &str, chunk: &mut Chunk) -> Result<Option<u16>, CompileError> {
    let Some((type_name, variant_name)) = crate::split_qualified(name) else {
        return Ok(None);
    };
    let is_unit = ENUM_INDEX.with(|ei| {
        ei.borrow()
            .get(type_name)
            .and_then(|vs| vs.iter().find(|v| v.name == variant_name))
            .is_some_and(|v| matches!(v.payload, crate::EnumPayload::None))
    });
    if !is_unit {
        return Ok(None);
    }
    let val = Value::EnumVariant {
        type_name: type_name.to_string(),
        variant: variant_name.to_string(),
        payload: crate::EnumValuePayload::None,
    };
    Ok(Some(chunk.add_constant(val)?))
}

/// RES-3915: if `name` is a `::`-qualified reference to a **tuple-payload**
/// enum variant recorded in `ENUM_INDEX`, intern a `Value::EnumConstructor`
/// carrying its declared arity and return the pool index. Returns `Ok(None)`
/// when `name` isn't a qualified tuple variant, so the caller falls through
/// to its normal unknown-identifier path.
///
/// A bare `Color::Rgb` reference in expression position (passed to a
/// higher-order function, stored in a local) becomes a first-class
/// constructor value; a later `Op::CallClosure` turns it into the
/// corresponding `Value::EnumVariant`. This mirrors the interpreter's
/// `Value::EnumConstructor` → `enum_ctors::apply_constructor` path.
/// Named-payload variants are intentionally excluded — the interpreter
/// doesn't support them as first-class constructors either.
fn tuple_enum_constructor_const(
    name: &str,
    chunk: &mut Chunk,
) -> Result<Option<u16>, CompileError> {
    let Some((type_name, variant_name)) = crate::split_qualified(name) else {
        return Ok(None);
    };
    let arity = ENUM_INDEX.with(|ei| {
        ei.borrow()
            .get(type_name)
            .and_then(|vs| vs.iter().find(|v| v.name == variant_name))
            .and_then(|v| match &v.payload {
                crate::EnumPayload::Tuple(types) => Some(types.len()),
                _ => None,
            })
    });
    let Some(arity) = arity else {
        return Ok(None);
    };
    let val = Value::EnumConstructor {
        type_name: type_name.to_string(),
        variant: variant_name.to_string(),
        arity,
    };
    Ok(Some(chunk.add_constant(val)?))
}

/// RES-3993: if `name` is a declared tuple struct (`struct Point(int,
/// int);`), return its positional field names (`["0", "1", ...]`) in
/// order. `STRUCT_FIELD_INDEX` (pre-scanned before pass 1, see its doc
/// comment) already stores every struct's declared field names —
/// including tuple structs, whose fields the parser names consecutive
/// decimal integers — so detecting one is the same "all names are
/// `0..field_count`" check `crate::tuple_struct::is_tuple_struct` uses
/// on the (type, name) pairs available at `StructDecl` eval time; here
/// only the names survive into `STRUCT_FIELD_INDEX`; but the tuple-struct
/// property is a name-shape property, so checking names alone is
/// sufficient and doesn't need the field types.
fn tuple_struct_field_names(name: &str) -> Option<Vec<String>> {
    STRUCT_FIELD_INDEX.with(|si| {
        si.borrow().get(name).and_then(|fields| {
            let is_tuple =
                !fields.is_empty() && fields.iter().enumerate().all(|(i, f)| f == &i.to_string());
            is_tuple.then(|| fields.clone())
        })
    })
}

fn emit_unit_enum_variants(
    enum_name: &str,
    variants: &[crate::EnumVariant],
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    for v in variants {
        if !matches!(v.payload, crate::EnumPayload::None) {
            continue;
        }
        let key = format!("{}::{}", enum_name, v.name);
        let val = Value::EnumVariant {
            type_name: enum_name.to_string(),
            variant: v.name.clone(),
            payload: crate::EnumValuePayload::None,
        };
        let const_idx = chunk.add_constant(val)?;
        chunk.emit(Op::Const(const_idx), line);
        let slot = *next_local;
        if slot == u16::MAX {
            return Err(CompileError::TooManyLocals);
        }
        *next_local += 1;
        locals.insert(key, slot);
        chunk.emit(local_store_op(slot), line);
    }
    Ok(())
}

/// `compile_stmt_in_fn`. Emits:
///   `<cond>; JumpIfTrue(past_fail); Const(msg); AssertFail`
#[allow(clippy::too_many_arguments)]
fn compile_assert(
    condition: &Node,
    message: &Option<Box<Node>>,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    compile_assert_like(
        condition,
        message,
        Op::AssertFail,
        "assertion failed",
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )
}

/// RES-3996: `assume(cond[, msg]);` — same runtime shape as `assert`
/// (halt immediately when the condition is false, dead code after it
/// never executes) but lowered to the dedicated `AssumeFail` opcode so
/// the VM's diagnostic reads "ASSUME VIOLATED" like the tree-walker's
/// `eval_assume`, instead of being silently dropped as a no-op (the
/// bug this ticket fixes — see `UNSUPPORTED_BY_VM` in differential.rs).
#[allow(clippy::too_many_arguments)]
fn compile_assume(
    condition: &Node,
    message: &Option<Box<Node>>,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    compile_assert_like(
        condition,
        message,
        Op::AssumeFail,
        "assumption failed",
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )
}

/// Shared lowering for `assert`/`assume`: evaluate `condition`, and if
/// falsy, push the failure message and emit `fail_op` (which halts the
/// VM unconditionally — see `dce.rs`'s terminator list). `default_msg`
/// is pushed when no explicit message expression is given.
#[allow(clippy::too_many_arguments)]
fn compile_assert_like(
    condition: &Node,
    message: &Option<Box<Node>>,
    fail_op: Op,
    default_msg: &str,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    compile_expr(
        condition,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    let jt = chunk.emit(Op::JumpIfTrue(0), line);
    // RES-2508: push the failure message onto the stack.  String literals
    // are embedded as constants; all other expressions are compiled so
    // they evaluate at runtime — matching the tree-walker interpreter.
    if let Some(msg_node) = message {
        match msg_node.as_ref() {
            Node::StringLiteral { value: s, .. } => {
                let msg_idx = chunk.add_string_constant(s)?;
                chunk.emit(Op::Const(msg_idx), line);
            }
            // RES-2612: interned strings compile the same as regular strings
            Node::StringInternLiteral { content: s, .. } => {
                let msg_idx = chunk.add_string_constant(s)?;
                chunk.emit(Op::Const(msg_idx), line);
            }
            _ => {
                compile_expr(
                    msg_node,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
            }
        }
    } else {
        let msg_idx = chunk.add_string_constant(default_msg)?;
        chunk.emit(Op::Const(msg_idx), line);
    }
    chunk.emit(fail_op, line);
    let past_fail = chunk.code.len();
    chunk.patch_jump(jt, past_fail)?;
    Ok(())
}

/// RES-155: `let StructName { f1, f2: local } = expr;` lowering.
/// Evaluates the RHS once into a temp slot, then for each
/// `(field_name, local_name)` pair emits `LoadLocal(tmp) + GetField +
/// StoreLocal(new_slot)`. After this, `local_name` is accessible in
/// subsequent code via `LoadLocal`.
#[allow(clippy::too_many_arguments)]
fn compile_let_destructure_struct(
    fields: &[(String, String)],
    value: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    compile_expr(
        value,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    if *next_local == u16::MAX {
        return Err(CompileError::TooManyLocals);
    }
    let tmp_idx = *next_local;
    *next_local += 1;
    chunk.emit(Op::StoreLocal(tmp_idx), line);
    for (field_name, local_name) in fields {
        if *next_local == u16::MAX {
            return Err(CompileError::TooManyLocals);
        }
        let slot = *next_local;
        *next_local += 1;
        locals.insert(local_name.clone(), slot);
        chunk.emit(Op::LoadLocal(tmp_idx), line);
        let fname_idx = chunk.add_string_constant(field_name)?;
        chunk.emit(
            Op::GetField {
                name_const: fname_idx,
            },
            line,
        );
        chunk.emit(Op::StoreLocal(slot), line);
    }
    Ok(())
}

/// RES-083: compile if/while/block statements that share the same
/// locals environment as the enclosing scope. `Block` is flattened:
/// its inner statements are compiled inline (no new scope frame yet
/// — matches the tree walker's semantics).
///
/// `loop_stack` threads the stack of enclosing loop states down through
/// Block and IfStatement arms; WhileStatement and ForInStatement push
/// a fresh entry and pop it after the body is compiled.
#[allow(clippy::too_many_arguments)]
fn compile_control_flow(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_stack: &mut Vec<LoopState>,
) -> Result<(), CompileError> {
    match node {
        // RES-4005: same lexical-scoping fix as `compile_control_flow_in_fn`
        // — clone `locals` per block so an inner shadowing `let` can't
        // overwrite the outer scope's slot for the rest of compilation.
        Node::Block { stmts, .. } => {
            let mut block_locals = locals.clone();
            for s in stmts {
                compile_stmt(
                    s,
                    chunk,
                    &mut block_locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    loop_stack,
                )?;
            }
            Ok(())
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            compile_expr(
                condition,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_stmt(
                consequence,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                loop_stack,
            )?;
            if let Some(alt) = alternative {
                let jmp_end = chunk.emit(Op::Jump(0), line);
                let else_target = chunk.code.len();
                chunk.patch_jump(jif, else_target)?;
                compile_stmt(
                    alt,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    loop_stack,
                )?;
                let end = chunk.code.len();
                chunk.patch_jump(jmp_end, end)?;
            } else {
                let end = chunk.code.len();
                chunk.patch_jump(jif, end)?;
            }
            Ok(())
        }
        Node::WhileStatement {
            condition,
            body,
            label,
            ..
        } => {
            let loop_start = chunk.code.len();
            compile_expr(
                condition,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            loop_stack.push(LoopState::with_label(loop_start, label.clone()));
            compile_stmt(
                body,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                loop_stack,
            )?;
            let inner = loop_stack.pop().unwrap();
            let jmp = chunk.emit(Op::Jump(0), line);
            chunk.patch_jump(jmp, loop_start)?;
            let end = chunk.code.len();
            chunk.patch_jump(jif, end)?;
            for p in inner.break_patches {
                chunk.patch_jump(p, end)?;
            }
            for p in inner.continue_patches {
                chunk.patch_jump(p, loop_start)?;
            }
            Ok(())
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            label,
            ..
        } => compile_for_in(
            name,
            iterable,
            body,
            label,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            /* in_fn */ false,
            loop_stack,
        ),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// RES-2538: compile a nested `fn` definition inside a function body.
/// The inner function is compiled into the functions table (like a
/// top-level fn) and bound as a zero-capture closure in a local
/// variable so call sites can resolve it via `CallClosure`.
#[allow(clippy::too_many_arguments)]
fn compile_nested_fn(
    name: &str,
    parameters: &[(String, String)],
    body: &Node,
    ensures: &[Node],
    recovers_to: &Option<Box<Node>>,
    outer_chunk: &mut Chunk,
    outer_locals: &mut HashMap<String, u16>,
    outer_next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    if parameters.len() > u8::MAX as usize {
        return Err(CompileError::Unsupported("fn with >255 params"));
    }
    if *next_fn_idx == u16::MAX {
        return Err(CompileError::Unsupported("program has > 65535 functions"));
    }
    let arity = parameters.len() as u8;

    // RES-4063: run the same free-variable capture-by-value analysis the
    // `Node::FunctionLiteral` compile arm uses, so a named nested `fn`
    // statement (e.g. `fn next() { .. count .. }` inside another fn's
    // body) can close over enclosing locals just like an anonymous
    // closure literal can. `name` itself is never in `outer_locals` yet
    // (only inserted below, after the closure is fully built), so
    // self-recursive calls to `name` inside `body` are unaffected —
    // they keep resolving through `inner_fn_index`, not as an upvalue.
    let param_names: std::collections::HashSet<&str> =
        parameters.iter().map(|(_, n)| n.as_str()).collect();
    let captured = analyze_and_box_captures(body, &param_names, outer_chunk, outer_locals, line)?;
    if captured.len() > u8::MAX as usize {
        return Err(CompileError::Unsupported("fn with >255 captured upvalues"));
    }
    let upvalue_count = captured.len();

    let mut chunk = Chunk::with_capacity(128);
    let cap = parameters.len().saturating_mul(2).max(8);
    let mut locals: HashMap<String, u16> = HashMap::with_capacity(cap);
    let mut next_local: u16 = 0;
    for (_type_name, pname) in parameters {
        locals.insert(pname.clone(), next_local);
        next_local += 1;
    }
    let upvalue_base = install_upvalue_locals_and_prologue(
        &mut locals,
        &mut next_local,
        &mut chunk,
        &captured,
        line,
    );
    let fn_idx = fns.len() as u16;
    fns.push(Function {
        name: name.to_string(),
        arity: 0,
        chunk: Chunk::with_capacity(0),
        local_count: 0,
        upvalue_source_slots: Box::default(),
        fails: Box::default(),
        postcheck: None,
    });
    let mut inner_fn_index = fn_index.clone();
    inner_fn_index.insert(name.to_string(), fn_idx);
    let inner = match body {
        Node::Block { stmts: b, .. } => b.as_slice(),
        single => std::slice::from_ref(single),
    };
    compile_fn_body_stmts(
        inner,
        &mut chunk,
        &mut locals,
        &mut next_local,
        &inner_fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    chunk.emit(Op::ReturnFromCall, 0);
    // RES-4017: `#[mutual_tail_call]` only ever annotates top-level
    // functions (see `mutual_tco::check`'s `Node::Program` scan), so a
    // nested `fn` can never be part of a mutual-recursion group —
    // self-recursion only, same as before.
    rewrite_tail_calls(&mut chunk, fn_idx, &std::collections::HashSet::new());
    crate::const_fold::optimize_if_enabled(&mut chunk)
        .map_err(|_| CompileError::InternalError("constant folder failed"))?;
    crate::peephole::optimize(&mut chunk)
        .map_err(|_| CompileError::InternalError("peephole optimizer failed"))?;
    crate::dce::eliminate(&mut chunk);
    // RES-4063: rewrite StoreLocal ops targeting upvalue pseudo-slots to
    // StoreUpvalue after the optimizer passes above, mirroring
    // `Node::FunctionLiteral` — see `rewrite_store_upvalues`'s doc
    // comment.
    rewrite_store_upvalues(&mut chunk, upvalue_base, upvalue_count);
    // RES-4041: see the matching comment in the top-level `compile_fn_body`
    // closure — synthesize the postcheck fn (if any) before recording this
    // fn's own entry.
    let postcheck = build_postcheck_function(
        name,
        parameters,
        ensures,
        recovers_to,
        &inner_fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    let source_slots: Box<[u16]> = build_upvalue_source_slots(&captured);
    fns[fn_idx as usize] = Function {
        name: name.to_string(),
        arity,
        chunk,
        local_count: next_local,
        upvalue_source_slots: source_slots,
        fails: Box::default(),
        postcheck,
    };
    emit_capture_loads(outer_chunk, &captured, line);
    outer_chunk.emit(
        Op::MakeClosure {
            fn_idx,
            upvalue_count: upvalue_count as u8,
        },
        line,
    );
    if *outer_next_local == u16::MAX {
        return Err(CompileError::TooManyLocals);
    }
    let slot = *outer_next_local;
    *outer_next_local += 1;
    outer_locals.insert(name.to_string(), slot);
    outer_chunk.emit(Op::StoreLocal(slot), line);
    Ok(())
}

/// RES-4063: shared free-variable capture-by-value analysis, factored out
/// of the `Node::FunctionLiteral` compile arm so `compile_nested_fn` (named
/// nested `fn` statements) can reuse the same boxing semantics. Collects
/// every identifier in `body` that isn't one of `param_names` and is bound
/// in the *outer* `locals` map (in insertion order, for deterministic
/// `LoadUpvalue(i)` indices), then boxes each non-global/non-static capture
/// into a shared `Value::Cell` handle in `chunk`/`locals` (skipping any
/// name already boxed by an earlier sibling closure in this same scope).
fn analyze_and_box_captures(
    body: &Node,
    param_names: &std::collections::HashSet<&str>,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    line: u32,
) -> Result<Vec<(u16, String)>, CompileError> {
    let mut captured: Vec<(u16, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_free_vars(body, param_names, locals, &mut captured, &mut seen);

    for (outer_slot, name) in &mut captured {
        if *outer_slot & (GLOBAL_FLAG | STATIC_FLAG) != 0 {
            continue;
        }
        if *outer_slot & BOXED_FLAG == 0 {
            let real = *outer_slot;
            chunk.emit(Op::LoadLocal(real), line);
            let cell_name_const = chunk.add_string_constant("cell")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: cell_name_const,
                    arity: 1,
                },
                line,
            );
            chunk.emit(Op::StoreLocal(real), line);
            let boxed_slot = real | BOXED_FLAG;
            locals.insert(name.clone(), boxed_slot);
            *outer_slot = boxed_slot;
        }
    }
    Ok(captured)
}

/// RES-4063: given the outer-scope `captured` analysis, register each
/// capture as a pseudo-local in the callee's own `fn_locals` (starting at
/// `*fn_next_local`, propagating `BOXED_FLAG` so in-body reads/writes route
/// through `Cell.get()`/`Cell.set()` like the outer scope does), advance
/// `fn_next_local` past them, and emit the `LoadUpvalue(i); StoreLocal(..)`
/// copy-in prologue at function entry. Returns the base slot the upvalue
/// pseudo-locals start at (needed by `rewrite_store_upvalues`).
fn install_upvalue_locals_and_prologue(
    fn_locals: &mut HashMap<String, u16>,
    fn_next_local: &mut u16,
    fn_chunk: &mut Chunk,
    captured: &[(u16, String)],
    line: u32,
) -> u16 {
    let upvalue_base = *fn_next_local;
    for (i, (outer_slot, name)) in captured.iter().enumerate() {
        let slot = upvalue_base + i as u16;
        let slot = if *outer_slot & BOXED_FLAG != 0 {
            slot | BOXED_FLAG
        } else {
            slot
        };
        fn_locals.insert(name.clone(), slot);
    }
    *fn_next_local += captured.len() as u16;
    for i in 0..captured.len() {
        fn_chunk.emit(Op::LoadUpvalue(i as u16), line);
        fn_chunk.emit(Op::StoreLocal(upvalue_base + i as u16), line);
    }
    upvalue_base
}

/// RES-4063: rewrite `StoreLocal` ops targeting the upvalue pseudo-slots
/// (`[upvalue_base, upvalue_base + upvalue_count)`) into `StoreUpvalue`, so
/// mutations of a captured name persist in the closure's upvalue slab for
/// cross-call visibility instead of only updating the local copy.
fn rewrite_store_upvalues(fn_chunk: &mut Chunk, upvalue_base: u16, upvalue_count: usize) {
    if upvalue_count == 0 {
        return;
    }
    let limit = upvalue_base + upvalue_count as u16;
    for op in fn_chunk.code.iter_mut() {
        if let Op::StoreLocal(slot) = *op
            && slot >= upvalue_base
            && slot < limit
        {
            *op = Op::StoreUpvalue {
                upvalue_idx: slot - upvalue_base,
                local_slot: slot,
            };
        }
    }
}

/// RES-4063: build the `Function::upvalue_source_slots` table — the outer
/// slot each upvalue should be written back to on return, or `u16::MAX`
/// (the existing "no write-back target" sentinel) for a static capture,
/// which has no valid caller-frame slot.
fn build_upvalue_source_slots(captured: &[(u16, String)]) -> Box<[u16]> {
    captured
        .iter()
        .map(|(s, _)| {
            if *s & STATIC_FLAG != 0 {
                u16::MAX
            } else {
                raw_slot(*s)
            }
        })
        .collect::<Vec<_>>()
        .into()
}

/// RES-4063: emit, in the *outer* chunk, the load of each captured value
/// onto the stack immediately before `Op::MakeClosure` — `LoadGlobal`/
/// `LoadStatic`/`LoadLocal` per the capture's flag (non-global, non-static
/// captures were boxed by `analyze_and_box_captures` above, so the
/// `LoadLocal` case always loads the shared `Value::Cell` handle, never a
/// value snapshot).
fn emit_capture_loads(chunk: &mut Chunk, captured: &[(u16, String)], line: u32) {
    for (outer_slot, _) in captured {
        if *outer_slot & GLOBAL_FLAG != 0 {
            chunk.emit(Op::LoadGlobal(raw_slot(*outer_slot)), line);
        } else if *outer_slot & STATIC_FLAG != 0 {
            chunk.emit(Op::LoadStatic(raw_slot(*outer_slot)), line);
        } else {
            chunk.emit(Op::LoadLocal(raw_slot(*outer_slot)), line);
        }
    }
}

/// RES-4041: synthesize a standalone bytecode `Function` that evaluates
/// `name`'s `ensures`/`recovers_to` postconditions, given its own
/// parameters plus the return value. `vm::run_dispatch_loop` calls the
/// result automatically on every `Op::ReturnFromCall` for `name` (see
/// the `Function::postcheck` doc comment in `bytecode.rs`) — mirroring
/// the tree-walking interpreter's post-body check (`lib.rs`, the
/// `ensures`/`recovers_to` block in the `Value::Function` call-
/// evaluation arm): bind `result` to the return value, evaluate each
/// `ensures` clause in declaration order, then the optional
/// `recovers_to` clause, and raise a `Contract violation in fn {name}:
/// ...` error on the first falsy one.
///
/// Returns `Ok(None)` when `name` declares neither clause — the common
/// case, and free of any runtime cost.
///
/// Scope (RES-4041): a clause may reference `name`'s own parameters and
/// `result` only — exactly what every `ensures`/`recovers_to` clause in
/// `examples/*.rz` does. A clause referencing an outer `let`-bound
/// local from the function body (not a parameter) fails to compile
/// here with `CompileError::UnknownIdentifier` — a loud, honest
/// compile-time failure rather than a silent runtime divergence from
/// the interpreter. Full free-variable capture (mirroring
/// `collect_free_vars`'s closure-upvalue handling) is a follow-up if
/// that shape is ever exercised.
#[allow(clippy::too_many_arguments)]
fn build_postcheck_function(
    name: &str,
    parameters: &[(String, String)],
    ensures: &[Node],
    recovers_to: &Option<Box<Node>>,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<Option<u16>, CompileError> {
    if ensures.is_empty() && recovers_to.is_none() {
        return Ok(None);
    }
    if parameters.len() >= u8::MAX as usize {
        return Err(CompileError::Unsupported("fn with >255 params"));
    }
    let arity = parameters.len() as u8 + 1;
    let mut chunk = Chunk::with_capacity(32);
    let mut locals: HashMap<String, u16> = HashMap::with_capacity(parameters.len() + 2);
    let mut next_local: u16 = 0;
    for (_type_name, pname) in parameters {
        locals.insert(pname.clone(), next_local);
        next_local += 1;
    }
    // RES-035/RES-392: `result` is bound alongside the parameters,
    // exactly like the interpreter binds it into the same call
    // environment the parameters already live in.
    locals.insert("result".to_string(), next_local);
    let result_slot = next_local;
    next_local += 1;

    let name_const = chunk.add_string_constant(name)?;
    for clause in ensures {
        compile_contract_clause(
            clause,
            false,
            name_const,
            &mut chunk,
            &mut locals,
            &mut next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
    }
    if let Some(rec) = recovers_to {
        compile_contract_clause(
            rec,
            true,
            name_const,
            &mut chunk,
            &mut locals,
            &mut next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
    }
    // Every clause held — return the (unused) result value so this
    // function's own `ReturnFromCall` has something to pop.
    chunk.emit(Op::LoadLocal(result_slot), line);
    chunk.emit(Op::ReturnFromCall, line);

    crate::const_fold::optimize_if_enabled(&mut chunk)
        .map_err(|_| CompileError::InternalError("constant folder failed"))?;
    crate::peephole::optimize(&mut chunk)
        .map_err(|_| CompileError::InternalError("peephole optimizer failed"))?;
    crate::dce::eliminate(&mut chunk);

    let fn_idx = fns.len() as u16;
    fns.push(Function {
        name: format!("{name}$postcheck"),
        arity,
        chunk,
        local_count: next_local,
        upvalue_source_slots: Box::default(),
        fails: Box::default(),
        postcheck: None,
    });
    Ok(Some(fn_idx))
}

/// RES-4119: synthesize a standalone bytecode `Function` that evaluates
/// a `defer <expr>;` call. `outer_locals`/`local_count` are the
/// enclosing fn's `locals` map and `next_local` counter *as of the
/// `defer` statement's position* — the thunk reuses those same name →
/// slot bindings as its own parameters (slots `0..local_count`), so
/// identifiers inside `expr` resolve exactly as they did in the
/// enclosing fn body. `Op::DeferPush` just records this thunk's index
/// on the current frame; `Op::ReturnFromCall` invokes it later (in LIFO
/// order, isolated per `run_postcheck`), reading its args from the
/// frame's *live* locals at that point — not a snapshot taken at the
/// `defer` site. This mirrors the tree-walking interpreter's
/// `Node::DeferStatement` eval arm: its "captured environment" is
/// `Rc<RefCell<..>>`-shared with the live one, so reassignments to
/// those locals between the `defer` statement and the function's exit
/// ARE visible to the deferred call.
///
/// The evaluated value is discarded (deferred exprs run for side
/// effects only, exactly like the interpreter's defer drain), and the
/// thunk returns `Void`.
#[allow(clippy::too_many_arguments)]
fn build_defer_function(
    expr: &Node,
    outer_locals: &HashMap<String, u16>,
    local_count: u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<u16, CompileError> {
    if local_count as usize >= u8::MAX as usize {
        return Err(CompileError::Unsupported(
            "defer site with >255 live locals",
        ));
    }
    let arity = local_count as u8;
    let mut chunk = Chunk::with_capacity(16);
    let mut locals: HashMap<String, u16> = outer_locals.clone();
    let mut next_local = local_count;

    compile_expr(
        expr,
        &mut chunk,
        &mut locals,
        &mut next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    chunk.emit(Op::Pop, line);
    let void_idx = chunk.add_constant(Value::Void)?;
    chunk.emit(Op::Const(void_idx), line);
    chunk.emit(Op::ReturnFromCall, line);

    crate::const_fold::optimize_if_enabled(&mut chunk)
        .map_err(|_| CompileError::InternalError("constant folder failed"))?;
    crate::peephole::optimize(&mut chunk)
        .map_err(|_| CompileError::InternalError("peephole optimizer failed"))?;
    crate::dce::eliminate(&mut chunk);

    let fn_idx = fns.len() as u16;
    fns.push(Function {
        name: "$defer".to_string(),
        arity,
        chunk,
        local_count: next_local,
        upvalue_source_slots: Box::default(),
        fails: Box::default(),
        postcheck: None,
    });
    Ok(fn_idx)
}

/// RES-4041: compile one `ensures`/`recovers_to` clause inside a
/// synthesized postcheck function (see `build_postcheck_function`).
/// Evaluates `clause`, and if falsy, loads `result` (already bound as
/// an ordinary local — see the caller) and raises
/// `Op::ContractViolation` — the same JumpIfTrue-guarded shape as
/// `compile_assert_like`.
#[allow(clippy::too_many_arguments)]
fn compile_contract_clause(
    clause: &Node,
    is_recovers_to: bool,
    name_const: u16,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    compile_expr(
        clause,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    let jt = chunk.emit(Op::JumpIfTrue(0), line);
    let result_slot = locals["result"];
    chunk.emit(Op::LoadLocal(result_slot), line);
    let clause_const = chunk.add_string_constant(&crate::format_contract_expr(clause))?;
    chunk.emit(
        Op::ContractViolation {
            name_const,
            clause_const,
            is_recovers_to,
        },
        line,
    );
    let past_fail = chunk.code.len();
    chunk.patch_jump(jt, past_fail)?;
    Ok(())
}

/// Compile a statement inside a `fn` body. Same as `compile_stmt`
/// except `return EXPR;` emits `ReturnFromCall` instead of `Return`
/// — a bare `return` at program scope halts the VM; one inside a
/// function returns to the caller.
#[allow(clippy::too_many_arguments)]
fn compile_stmt_in_fn(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_stack: &mut Vec<LoopState>,
) -> Result<(), CompileError> {
    match node {
        Node::LetStatement { name, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-401: `let (a, b, c) = expr;` inside a function body.
        Node::LetTupleDestructure { names, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let tmp_idx = *next_local;
            *next_local += 1;
            chunk.emit(Op::StoreLocal(tmp_idx), line);
            for (i, name) in names.iter().enumerate() {
                if *next_local == u16::MAX {
                    return Err(CompileError::TooManyLocals);
                }
                let slot = *next_local;
                *next_local += 1;
                locals.insert(name.clone(), slot);
                chunk.emit(Op::LoadLocal(tmp_idx), line);
                let idx_const = chunk.add_constant(Value::Int(i as i64))?;
                chunk.emit(Op::Const(idx_const), line);
                chunk.emit(Op::LoadIndex, line);
                chunk.emit(Op::StoreLocal(slot), line);
            }
            Ok(())
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            compile_expr(
                v,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::ReturnFromCall, line);
            Ok(())
        }
        Node::ReturnStatement { value: None, .. } => {
            // `return;` inside a fn body returns Void — push a Void
            // constant so ReturnFromCall has something to transfer.
            let idx = chunk.add_constant(Value::Void)?;
            chunk.emit(Op::Const(idx), line);
            chunk.emit(Op::ReturnFromCall, line);
            Ok(())
        }
        // RES-3997: a bare `expr;` statement discards its value — the
        // expression is compiled normally (leaving exactly one value on
        // the operand stack, per `compile_expr`'s invariant) and that
        // value is immediately popped so it doesn't leak into whatever
        // the next statement/expression evaluates.
        Node::ExpressionStatement { expr: inner, .. } => {
            compile_expr(
                inner,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Pop, line);
            Ok(())
        }
        Node::IfStatement { .. }
        | Node::WhileStatement { .. }
        | Node::ForInStatement { .. }
        | Node::Block { .. } => compile_control_flow_in_fn(
            node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        ),
        // RES-4119: `defer <expr>;` — synthesize a standalone thunk
        // function over the locals declared so far in this fn (see
        // `build_defer_function`) and register it with `Op::DeferPush`.
        // The VM (`vm::run_dispatch_loop`'s `Op::DeferPush` handler)
        // snapshots the current locals and pushes `(fn_idx, snapshot)`
        // onto the frame's defer stack, drained LIFO by every
        // `Op::ReturnFromCall` for this fn — mirrors the tree-walking
        // interpreter's `Node::DeferStatement` eval arm (`lib.rs`).
        Node::DeferStatement { expr, .. } => {
            let fn_idx = build_defer_function(
                expr, locals, *next_local, fn_index, ffi_index, fns, next_fn_idx, line,
            )?;
            chunk.emit(Op::DeferPush(fn_idx), line);
            Ok(())
        }
        Node::Assignment { name, value, .. } => compile_assignment(
            name,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-171a/RES-171c: `a[i] = v` and `a[i0][i1]...[iN] = v`.
        // Shares the compile_index_assignment helper with compile_stmt.
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => compile_index_assignment(
            target,
            index,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-335: `p.field = v;` inside a fn body. Mirrors the
        // `compile_stmt` arm above; duplicated because the two
        // dispatchers handle `return` differently.
        Node::FieldAssignment {
            target,
            field,
            value,
            ..
        } => {
            // RES-1430: borrow target name as &str — see comment on
            // the compile_stmt IndexAssignment arm.
            let local_name: &str = match target.as_ref() {
                Node::Identifier { name, .. } => name.as_str(),
                _ => {
                    return Err(CompileError::Unsupported(
                        "nested field assignment (non-identifier target)",
                    ));
                }
            };
            let slot = *locals
                .get(local_name)
                .ok_or_else(|| CompileError::UnknownIdentifier(local_name.to_string()))?;
            chunk.emit(local_load_op(slot), line);
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let fname_idx = chunk.add_string_constant(field)?;
            chunk.emit(
                Op::SetField {
                    name_const: fname_idx,
                },
                line,
            );
            chunk.emit(local_store_op(slot), line);
            Ok(())
        }
        Node::Break { .. } => {
            let ls = loop_stack
                .last_mut()
                .ok_or(CompileError::Unsupported("break outside loop"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.break_patches.push(patch);
            Ok(())
        }
        // RES-3993: mirrors the `compile_stmt` arm above — see its comment
        // for the value_mode/break_value_patches rationale.
        Node::BreakWith { value, .. } => {
            let value_mode = loop_stack
                .last()
                .ok_or(CompileError::Unsupported("break outside loop"))?
                .value_mode;
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let ls = loop_stack.last_mut().unwrap();
            if value_mode {
                let patch = chunk.emit(Op::Jump(0), line);
                ls.break_value_patches.push(patch);
            } else {
                chunk.emit(Op::Pop, line);
                let patch = chunk.emit(Op::Jump(0), line);
                ls.break_patches.push(patch);
            }
            Ok(())
        }
        Node::Continue { .. } => {
            let ls = loop_stack
                .last_mut()
                .ok_or(CompileError::Unsupported("continue outside loop"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.continue_patches.push(patch);
            Ok(())
        }
        Node::BreakLabel { label, .. } => {
            let ls = loop_stack
                .iter_mut()
                .rev()
                .find(|ls| ls.label.as_deref() == Some(label.as_str()))
                .ok_or(CompileError::Unsupported("break label not found"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.break_patches.push(patch);
            Ok(())
        }
        Node::ContinueLabel { label, .. } => {
            let ls = loop_stack
                .iter_mut()
                .rev()
                .find(|ls| ls.label.as_deref() == Some(label.as_str()))
                .ok_or(CompileError::Unsupported("continue label not found"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.continue_patches.push(patch);
            Ok(())
        }
        Node::Assert {
            condition, message, ..
        } => compile_assert(
            condition,
            message,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-155: struct destructuring inside a function body.
        Node::LetDestructureStruct { fields, value, .. } => compile_let_destructure_struct(
            fields,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-4046: `static let NAME = EXPR;` inside a fn body must
        // persist across separate calls to the function, unlike an
        // ordinary local (which lives in the per-call locals slab and
        // resets every call) — matching the tree-walking interpreter's
        // `self.statics` and top-level `static let` (which is
        // trivially "persistent" since top-level code runs exactly
        // once). `idx` is reused as the index into the VM's
        // per-function statics table (a completely separate storage
        // class from ordinary locals — see `STATIC_FLAG`), so borrowing
        // a number from `next_local`'s counter only "wastes" one unused
        // slot in the per-call locals array; it never collides with a
        // real local.
        //
        // Compiles to a guarded one-time init, mirroring
        // `compile_assert_like`'s `JumpIfTrue` skip shape:
        //   PushStaticInitialized(idx); JumpIfTrue(skip);
        //   <init-expr>; StoreStatic(idx); skip:
        Node::StaticLet { name, value, .. } => {
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx | STATIC_FLAG);

            chunk.emit(Op::PushStaticInitialized(idx), line);
            let jt = chunk.emit(Op::JumpIfTrue(0), line);
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::StoreStatic(idx), line);
            let past_init = chunk.code.len();
            chunk.patch_jump(jt, past_init)?;
            Ok(())
        }
        // RES-361: const decl inside fn body — pre-evaluated, no emission.
        Node::Const { .. } => Ok(()),
        // RES-2660: static_assert — compile-time only, no emission.
        Node::StaticAssert { .. } => Ok(()),
        // RES-3995: `live { body }` inside fn body — see `compile_live_block`.
        Node::LiveBlock {
            body,
            invariants,
            backoff,
            backoff_kind,
            timeout,
            max_retries,
            ..
        } => compile_live_block(
            body,
            invariants,
            backoff,
            *backoff_kind,
            timeout,
            *max_retries,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
            true,
        ),
        // RES-4024: the MMIO-wrapper block keyword inside fn body —
        // compile the body exactly like `LiveBlock`. See the matching
        // comment in `compile_stmt` above; this arm was previously grouped
        // with the declaration-only nodes below and silently dropped the
        // body.
        Node::UnsafeBlock { body, .. } => compile_stmt_in_fn(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        ),
        // RES-3996: `assume(cond[, msg]);` halts at runtime like `assert`
        // when the condition is false (see `compile_assume`). Previously
        // grouped with the declaration-only no-op arm below, which
        // silently dropped the runtime check under `--vm`.
        Node::Assume {
            condition, message, ..
        } => compile_assume(
            condition,
            message,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // Verification-only construct: emit nothing at runtime.
        Node::InvariantStatement { .. } => Ok(()),
        // Type-level / declaration-only constructs: no runtime bytecode.
        Node::EnumDecl { name, variants, .. } => {
            emit_unit_enum_variants(name, variants, chunk, locals, next_local, line)
        }
        Node::StructDecl { .. }
        | Node::TraitDecl { .. }
        | Node::ImplBlock { .. }
        // RES-2689: BlanketImpl is declaration-only; concrete ImplBlocks were
        // already injected by lower_program before compilation.
        | Node::BlanketImpl { .. }
        | Node::TypeAlias { .. }
        | Node::NewtypeDecl { .. }
        | Node::RegionDecl { .. }
        | Node::ActorDecl { .. }
        | Node::ClusterDecl { .. }
        | Node::SupervisorDecl { .. }
        | Node::ModuleDecl { .. }
        | Node::Use { .. } => Ok(()),
        // RES-3993: see the matching `Node::BenchBlock` arm in `compile_stmt`.
        Node::BenchBlock { .. } => Ok(()),
        Node::Function {
            name,
            parameters,
            body,
            ensures,
            recovers_to,
            ..
        } => compile_nested_fn(
            name,
            parameters,
            body,
            ensures,
            recovers_to,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        Node::TryCatch { body, handlers, .. } => compile_try_catch(
            body,
            handlers,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
            true,
        ),
        Node::Extern { .. } => Err(CompileError::Unsupported("nested extern decl")),
        // RES-3993: same `if let`/`while let` desugar-target fix as the
        // top-level `compile_stmt` arm above, routed through the in-fn
        // variant so `return` inside an arm emits `ReturnFromCall`.
        Node::Match {
            scrutinee, arms, ..
        } => compile_match_stmt_in_fn(
            scrutinee,
            arms,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        ),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// Same as `compile_control_flow` but routes nested statements
/// through `compile_stmt_in_fn` so `return` inside a branch emits
/// `ReturnFromCall`. This is the version used by function bodies.
#[allow(clippy::too_many_arguments)]
fn compile_control_flow_in_fn(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_stack: &mut Vec<LoopState>,
) -> Result<(), CompileError> {
    match node {
        // RES-4005: a block introduces a new lexical scope — clone
        // `locals` so `let` bindings inside the block (including ones
        // that shadow an outer name) don't leak into the caller's view
        // once the block exits. Mirrors `compile_block_as_expr`, which
        // already does this for expression-position blocks. `next_local`
        // is intentionally *not* cloned: slot numbers must keep
        // incrementing monotonically so a shadowing `let` gets a fresh
        // slot rather than reusing (and clobbering) the outer one.
        Node::Block { stmts, .. } => {
            let mut block_locals = locals.clone();
            for s in stmts {
                compile_stmt_in_fn(
                    s,
                    chunk,
                    &mut block_locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    loop_stack,
                )?;
            }
            Ok(())
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            compile_expr(
                condition,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_stmt_in_fn(
                consequence,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                loop_stack,
            )?;
            if let Some(alt) = alternative {
                let jmp_end = chunk.emit(Op::Jump(0), line);
                let else_target = chunk.code.len();
                chunk.patch_jump(jif, else_target)?;
                compile_stmt_in_fn(
                    alt,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    loop_stack,
                )?;
                let end = chunk.code.len();
                chunk.patch_jump(jmp_end, end)?;
            } else {
                let end = chunk.code.len();
                chunk.patch_jump(jif, end)?;
            }
            Ok(())
        }
        Node::WhileStatement {
            condition,
            body,
            label,
            ..
        } => {
            let loop_start = chunk.code.len();
            compile_expr(
                condition,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            loop_stack.push(LoopState::with_label(loop_start, label.clone()));
            compile_stmt_in_fn(
                body,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                loop_stack,
            )?;
            let inner = loop_stack.pop().unwrap();
            let jmp = chunk.emit(Op::Jump(0), line);
            chunk.patch_jump(jmp, loop_start)?;
            let end = chunk.code.len();
            chunk.patch_jump(jif, end)?;
            for p in inner.break_patches {
                chunk.patch_jump(p, end)?;
            }
            for p in inner.continue_patches {
                chunk.patch_jump(p, loop_start)?;
            }
            Ok(())
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            label,
            ..
        } => compile_for_in(
            name,
            iterable,
            body,
            label,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            /* in_fn */ true,
            loop_stack,
        ),
        Node::EnumDecl { name, variants, .. } => {
            emit_unit_enum_variants(name, variants, chunk, locals, next_local, line)
        }
        // Type-level / declaration-only constructs: no runtime bytecode.
        Node::StructDecl { .. }
        | Node::TraitDecl { .. }
        | Node::ImplBlock { .. }
        | Node::TypeAlias { .. }
        | Node::NewtypeDecl { .. }
        | Node::RegionDecl { .. }
        | Node::ActorDecl { .. }
        | Node::ClusterDecl { .. }
        | Node::SupervisorDecl { .. }
        | Node::ModuleDecl { .. }
        | Node::Use { .. }
        | Node::UnsafeBlock { .. }
        | Node::Assume { .. }
        | Node::InvariantStatement { .. }
        // RES-3993: see the matching `Node::BenchBlock` arm in `compile_stmt`.
        | Node::BenchBlock { .. } => Ok(()),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// RES-334: compile `for NAME in ITERABLE { BODY }` to bytecode.
///
/// Models iteration on the existing `while`-loop pattern. Three
/// hidden locals carry the iterator state (the array value, the
/// integer index, and the integer length). The loop variable
/// `NAME` becomes a normal local that the body can read by
/// identifier; we re-bind it from `arr[idx]` at the top of every
/// iteration.
///
/// Today only `Value::Array` iteration is wired — strings and
/// half-open integer ranges are out of scope here (no AST node
/// for either yet) and surface as `VmError::TypeMismatch` /
/// `VmError::BuiltinCallFailed` from `LoadIndex` / `len` at run
/// time. The shape `for x in 0..10` parses inside quantifier
/// expressions only; statement position is rejected by the
/// parser before compile is reached.
///
/// Lowered shape (prior to RES-3902, peephole folded the `idx + 1`
/// tail into a single `IncLocal`; that fold was removed as unsound —
/// see `peephole.rs` — so `Op::IncLocal` is no longer emitted, though
/// the VM/disassembler still support it defensively):
///
/// ```text
///   <iterable>
///   StoreLocal arr_slot
///   LoadLocal arr_slot
///   CallBuiltin { "len", arity: 1 }
///   StoreLocal len_slot
///   Const 0
///   StoreLocal idx_slot
/// LOOP_START:
///   LoadLocal idx_slot
///   LoadLocal len_slot
///   Lt
///   JumpIfFalse EXIT
///   LoadLocal arr_slot
///   LoadLocal idx_slot
///   LoadIndex
///   StoreLocal name_slot
///   <body>
///   LoadLocal idx_slot
///   Const 1
///   Add
///   StoreLocal idx_slot
///   Jump LOOP_START
/// EXIT:
/// ```
#[allow(clippy::too_many_arguments)]
fn compile_for_in(
    name: &str,
    iterable: &Node,
    body: &Node,
    label: &Option<String>,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    in_fn: bool,
    loop_stack: &mut Vec<LoopState>,
) -> Result<(), CompileError> {
    // Allocate three hidden locals for the iteration state plus
    // one user-visible slot for the loop variable. Hidden slots
    // get unique synthetic names so they cannot shadow or be
    // reached by any user identifier.
    if (*next_local as usize) + 4 > u16::MAX as usize {
        return Err(CompileError::TooManyLocals);
    }
    let arr_slot = *next_local;
    *next_local += 1;
    let len_slot = *next_local;
    *next_local += 1;
    let idx_slot = *next_local;
    *next_local += 1;
    // Reserve hidden-slot keys that are not valid identifiers so
    // user code with names like "$for_arr" cannot collide. Loop
    // variable goes into the regular `locals` map so the body
    // can read it via Identifier lookup.
    let arr_key = format!("$for_arr@{}", arr_slot);
    let len_key = format!("$for_len@{}", len_slot);
    let idx_key = format!("$for_idx@{}", idx_slot);
    locals.insert(arr_key.clone(), arr_slot);
    locals.insert(len_key.clone(), len_slot);
    locals.insert(idx_key.clone(), idx_slot);
    // Loop-variable slot: shadow any outer binding for the
    // duration of this loop. The previous binding (if any) is
    // restored after the loop body so subsequent statements see
    // the original slot — matches `let`-shadowing semantics.
    let prev_name_slot = locals.get(name).copied();
    let name_slot = *next_local;
    *next_local += 1;
    locals.insert(name.to_string(), name_slot);

    // 1. Evaluate iterable, normalize for iteration, store in arr_slot.
    //    IterPrepare converts maps to their sorted-keys array so the
    //    sequential LoadIndex loop below works uniformly for arrays,
    //    strings, and maps.
    compile_expr(
        iterable,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    chunk.emit(Op::IterPrepare, line);
    chunk.emit(Op::StoreLocal(arr_slot), line);

    // 2. Compute length via the canonical `len` builtin and
    //    store in len_slot. `len` handles arrays, strings, and
    //    any other iterable — the VM's LoadIndex was extended
    //    (RES-334b) to support strings so `for c in "hello"` and
    //    `for i in 0..10` (RES-4000: `Op::IterPrepare` materializes
    //    the `Value::Range` into an array) both work uniformly.
    let len_name_const = chunk.add_string_constant("len")?;
    chunk.emit(Op::LoadLocal(arr_slot), line);
    chunk.emit(
        Op::CallBuiltin {
            name_const: len_name_const,
            arity: 1,
        },
        line,
    );
    chunk.emit(Op::StoreLocal(len_slot), line);

    // 3. idx = 0
    let zero_const = chunk.add_constant(Value::Int(0))?;
    chunk.emit(Op::Const(zero_const), line);
    chunk.emit(Op::StoreLocal(idx_slot), line);

    // 4. Loop test: idx < len.
    let loop_start = chunk.code.len();
    chunk.emit(Op::LoadLocal(idx_slot), line);
    chunk.emit(Op::LoadLocal(len_slot), line);
    chunk.emit(Op::Lt, line);
    let jif = chunk.emit(Op::JumpIfFalse(0), line);

    // 5. name = arr[idx]
    chunk.emit(Op::LoadLocal(arr_slot), line);
    chunk.emit(Op::LoadLocal(idx_slot), line);
    chunk.emit(Op::LoadIndex, line);
    chunk.emit(Op::StoreLocal(name_slot), line);

    // 6. Body. Push a LoopState onto the stack so break/continue (including
    //    labeled variants) can find this loop. `continue` in a for-in loop
    //    skips to the index increment (step 7), whose PC is not yet known —
    //    continue_patches are back-patched below.
    loop_stack.push(LoopState::with_label(0, label.clone())); // continue_target set after body
    if in_fn {
        compile_stmt_in_fn(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        )?;
    } else {
        compile_stmt(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        )?;
    }
    let inner = loop_stack.pop().unwrap();

    // 7. idx = idx + 1 (no longer peephole-folded to IncLocal — RES-3902).
    // This is the `continue` target for this loop — record the PC before
    // emitting the increment so `continue` skips to here.
    let continue_target = chunk.code.len();
    chunk.emit(Op::LoadLocal(idx_slot), line);
    let one_const = chunk.add_constant(Value::Int(1))?;
    chunk.emit(Op::Const(one_const), line);
    chunk.emit(Op::Add, line);
    chunk.emit(Op::StoreLocal(idx_slot), line);

    // 8. Jump back to test.
    let jmp = chunk.emit(Op::Jump(0), line);
    chunk.patch_jump(jmp, loop_start)?;
    let end = chunk.code.len();
    chunk.patch_jump(jif, end)?;

    // Patch break → exit, continue → idx increment.
    for p in inner.break_patches {
        chunk.patch_jump(p, end)?;
    }
    for p in inner.continue_patches {
        chunk.patch_jump(p, continue_target)?;
    }

    // Restore the loop variable's outer binding. The hidden
    // iterator slots stay in `locals` so a later for-loop in
    // the same scope reuses fresh slots (next_local has already
    // moved past them).
    locals.remove(&arr_key);
    locals.remove(&len_key);
    locals.remove(&idx_key);
    if let Some(prev) = prev_name_slot {
        locals.insert(name.to_string(), prev);
    } else {
        locals.remove(name);
    }
    Ok(())
}

/// RES-171c: compile `a[i0][i1]...[iN] = v` for any nesting depth.
///
/// Extracts (root_name, indices[]) from the assignment chain, allocates
/// N-1 hidden temp locals, and emits load/mutate/writeback sequences
/// so all intermediate arrays are updated in value-semantics order.
///
/// For depth=1 this degenerates to the simple `LoadLocal / StoreIndex /
/// StoreLocal` triple (no temps needed).
#[allow(clippy::too_many_arguments)]
fn compile_index_assignment(
    target: &Node,
    outermost_index: &Node,
    value: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    // Walk target chain to collect indices in root-to-leaf order.
    let mut indices_rev: Vec<&Node> = vec![outermost_index];
    let mut cursor: &Node = target;
    let root_name = loop {
        match cursor {
            Node::Identifier { name, .. } => break name.as_str(),
            Node::IndexExpression {
                target: inner_t,
                index: inner_i,
                ..
            } => {
                indices_rev.push(inner_i.as_ref());
                cursor = inner_t.as_ref();
            }
            _ => {
                return Err(CompileError::Unsupported(
                    "non-identifier target in index assignment",
                ));
            }
        }
    };
    indices_rev.reverse();
    let indices: Vec<&Node> = indices_rev;
    let depth = indices.len(); // >= 1

    let root_slot = *locals
        .get(root_name)
        .ok_or_else(|| CompileError::UnknownIdentifier(root_name.to_string()))?;

    if depth == 1 {
        // Fast path: `a[i] = v`.
        chunk.emit(local_load_op(root_slot), line);
        compile_expr(
            indices[0],
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        compile_expr(
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        chunk.emit(Op::StoreIndex, line);
        chunk.emit(local_store_op(root_slot), line);
        return Ok(());
    }

    // Depth >= 2: allocate N-1 temp locals.
    let n_temps = depth - 1;
    if (*next_local as usize) + n_temps > u16::MAX as usize {
        return Err(CompileError::TooManyLocals);
    }
    let temp_base = *next_local;
    *next_local += n_temps as u16;
    let temp_keys: Vec<String> = (0..n_temps)
        .map(|k| format!("$nested_idx@{}", temp_base + k as u16))
        .collect();
    for (k, key) in temp_keys.iter().enumerate() {
        locals.insert(key.clone(), temp_base + k as u16);
    }

    // Phase 1: load each intermediate level into its temp.
    // $t0 = root[i0], $t1 = $t0[i1], ..., $t(N-2) = $t(N-3)[i(N-2)]
    for (k, idx_node) in indices.iter().enumerate().take(n_temps) {
        if k == 0 {
            chunk.emit(local_load_op(root_slot), line);
        } else {
            chunk.emit(Op::LoadLocal(temp_base + (k as u16 - 1)), line);
        }
        compile_expr(
            idx_node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        chunk.emit(Op::LoadIndex, line);
        chunk.emit(Op::StoreLocal(temp_base + k as u16), line);
    }

    // Phase 2: mutate the deepest temp.
    // $t(N-2)[i(N-1)] = v
    let deepest_temp = temp_base + (n_temps as u16 - 1);
    chunk.emit(Op::LoadLocal(deepest_temp), line);
    compile_expr(
        indices[depth - 1],
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    compile_expr(
        value,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    chunk.emit(Op::StoreIndex, line);
    chunk.emit(Op::StoreLocal(deepest_temp), line);

    // Phase 3: write back up the chain.
    // $t(k-1)[i(k)] = $t(k), down to root[i0] = $t0
    for k in (0..n_temps).rev() {
        if k == 0 {
            chunk.emit(local_load_op(root_slot), line);
        } else {
            chunk.emit(Op::LoadLocal(temp_base + (k as u16 - 1)), line);
        }
        compile_expr(
            indices[k],
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        chunk.emit(Op::LoadLocal(temp_base + k as u16), line);
        chunk.emit(Op::StoreIndex, line);
        if k == 0 {
            chunk.emit(local_store_op(root_slot), line);
        } else {
            chunk.emit(Op::StoreLocal(temp_base + (k as u16 - 1)), line);
        }
    }

    // Clean up temp keys from locals map.
    for key in &temp_keys {
        locals.remove(key);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compile_expr(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::IntegerLiteral { value: v, .. } => {
            let idx = chunk.add_constant(Value::Int(*v))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        // RES-083: boolean literals.
        Node::BooleanLiteral { value: b, .. } => {
            let idx = chunk.add_constant(Value::Bool(*b))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        // RES-2683: char literal in VM codegen path.
        Node::CharLiteral { value: c, .. } => {
            let idx = chunk.add_constant(Value::Char(*c))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        // RES-VM (issue #266): string + float literals. Required so
        // calls like `println("hello")` and `sin(1.5)` reach the
        // bytecode VM. The constant pool already accepts `Value::String`
        // and `Value::Float` (used today by struct/field name interning
        // and dedup); routing the literal nodes here lets builtin args
        // round-trip without touching the runtime.
        Node::StringLiteral { value: s, .. } => {
            let idx = chunk.add_string_constant(s)?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        // RES-2612: interned strings compile the same as regular strings
        Node::StringInternLiteral { content: s, .. } => {
            let idx = chunk.add_string_constant(s)?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        Node::FloatLiteral { value: x, .. } => {
            let idx = chunk.add_constant(Value::Float(*x))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        Node::Identifier { name, .. } => {
            if let Some(&idx) = locals.get(name) {
                emit_identifier_load(chunk, idx, line)?;
            } else if crate::lookup_builtin(name).is_some() {
                let name_const = chunk.add_string_constant(name)?;
                chunk.emit(
                    Op::CallBuiltin {
                        name_const,
                        arity: 0,
                    },
                    line,
                );
            } else if let Some(const_idx) = zero_arg_enum_variant_const(name, chunk)? {
                // RES-3916: bare zero-arg enum-variant reference like `E::A`
                // in expression position. `emit_unit_enum_variants` only
                // registers these as locals in the *declaring* scope, so a
                // reference from another function body missed them and hit
                // `UnknownIdentifier`. Resolve against the scope-independent
                // `ENUM_INDEX` (same registry the `E::A(x)` constructor path
                // uses) and emit the variant constant directly.
                chunk.emit(Op::Const(const_idx), line);
            } else if let Some(const_idx) = tuple_enum_constructor_const(name, chunk)? {
                // RES-3915: bare tuple-payload enum-variant reference like
                // `Color::Rgb` used as a first-class value (passed to a
                // higher-order function, stored in a local). Emit a
                // `Value::EnumConstructor` constant; a later `CallClosure`
                // converts it into the corresponding `EnumVariant`, mirroring
                // the interpreter's first-class-constructor path.
                chunk.emit(Op::Const(const_idx), line);
            } else if let Some(&idx) = fn_index.get(name) {
                // RES-3993: bare reference to a named top-level (or
                // impl-block) function used as a first-class value —
                // `let f = pick(true)` returning `double`, `apply(double,
                // 5)` passing it as an argument, a `fn(T) -> T` typed
                // parameter bound to a named function, etc.
                // `CallExpression` already special-cases an `Identifier`
                // *callee* to call `fn_index[name]` directly, but a plain
                // identifier in expression position (not the callee of a
                // call) reached this arm and fell through to
                // `UnknownIdentifier` because only locals, builtins, and
                // enum constructors were considered. Named top-level
                // functions capture nothing (they can only see other
                // globals, which their own body reads via
                // `LoadGlobal`/`LoadStatic`, not upvalues), so wrapping
                // `fn_index[name]` in a zero-upvalue closure is a
                // faithful bytecode analogue of the tree-walker's
                // `Value::Function` — same callable identity, no captured
                // environment to carry.
                chunk.emit(
                    Op::MakeClosure {
                        fn_idx: idx,
                        upvalue_count: 0,
                    },
                    line,
                );
            } else {
                return Err(CompileError::UnknownIdentifier(name.clone()));
            }
            Ok(())
        }
        Node::PrefixExpression {
            operator, right, ..
        } if *operator == "-" => {
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Neg, line);
            Ok(())
        }
        // RES-083: logical negation.
        Node::PrefixExpression {
            operator, right, ..
        } if *operator == "!" => {
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Not, line);
            Ok(())
        }
        // RES-083: short-circuit && desugars to `if lhs { rhs } else { false }`.
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } if *operator == "&&" => {
            compile_expr(
                left,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            // RES-3894: reject a non-bool left operand before the short-circuit
            // jump, matching the interpreter (`Logical '&&' requires bool
            // operands`). A left operand that *is* `false` short-circuits below
            // and the right operand is never asserted — same as the interpreter,
            // which only evaluates the right operand when the left is `true`.
            chunk.emit(Op::AssertBool, line);
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::AssertBool, line);
            let jmp_end = chunk.emit(Op::Jump(0), line);
            // false branch
            let false_target = chunk.code.len();
            chunk.patch_jump(jif, false_target)?;
            let false_const = chunk.add_constant(Value::Bool(false))?;
            chunk.emit(Op::Const(false_const), line);
            let end = chunk.code.len();
            chunk.patch_jump(jmp_end, end)?;
            Ok(())
        }
        // RES-083: short-circuit || desugars to `if !lhs { rhs } else { true }`.
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } if *operator == "||" => {
            compile_expr(
                left,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            // RES-3894: reject a non-bool left operand before it is coerced by
            // `Not`, matching the interpreter. A `true` left operand short-
            // circuits below and the right operand is never asserted.
            chunk.emit(Op::AssertBool, line);
            // Negate lhs so JumpIfFalse skips to "true" when lhs is truthy.
            chunk.emit(Op::Not, line);
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            // lhs was falsy → evaluate rhs
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::AssertBool, line);
            let jmp_end = chunk.emit(Op::Jump(0), line);
            // true branch
            let true_target = chunk.code.len();
            chunk.patch_jump(jif, true_target)?;
            let true_const = chunk.add_constant(Value::Bool(true))?;
            chunk.emit(Op::Const(true_const), line);
            let end = chunk.code.len();
            chunk.patch_jump(jmp_end, end)?;
            Ok(())
        }
        // RES-3993: `??` — Option coalescing. Unlike `&&`/`||`, the
        // interpreter's `eval_infix_expression` does *not* short-circuit
        // `??`: `Node::InfixExpression`'s generic eval path evaluates both
        // `left` and `right` before dispatching on the operator, so
        // `right`'s side effects always run even when `left` is `Some`.
        // Mirror that exactly here — compile both operands unconditionally,
        // then let `Op::Coalesce` pick which value survives.
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } if *operator == "??" => {
            compile_expr(
                left,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Coalesce, line);
            Ok(())
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            compile_expr(
                left,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let op = match *operator {
                "+" => Op::Add,
                "-" => Op::Sub,
                "*" => Op::Mul,
                "/" => Op::Div,
                "%" => Op::Mod,
                // RES-083: comparison ops produce Value::Bool.
                "==" => Op::Eq,
                "!=" => Op::Neq,
                "<" => Op::Lt,
                "<=" => Op::Le,
                ">" => Op::Gt,
                ">=" => Op::Ge,
                // Bitwise ops (integer-only; typechecker enforces operand types).
                "&" => Op::Band,
                "|" => Op::Bor,
                "^" => Op::Bxor,
                "<<" => Op::Shl,
                ">>" => Op::Shr,
                _ => return Err(CompileError::Unsupported("non-arithmetic operator")),
            };
            chunk.emit(op, line);
            Ok(())
        }
        // RES-081: call to a top-level function. Only supports
        // calls where the callee is a bare `Identifier` — indirect
        // call through a function value (closures, lambdas) is out
        // of scope here.
        Node::CallExpression {
            function,
            arguments,
            span: call_expr_span,
        } => {
            // RES-4131: column of the call's `(` token, recorded
            // against every call-site opcode below so the VM's
            // `stacktrace()` can format `<fn> at <file>:<line>:<col>`
            // frames identically to the tree-walker (which uses this
            // same span — see `lib.rs`'s `call_span = *span`).
            let call_col = call_expr_span.start.column as u32;
            // RES-1419: hold the callee name as `&str` through the
            // three index lookups + `lookup_builtin` instead of
            // eagerly cloning to an owned `String`. The previous
            // shape cloned once at the top of the arm and a second
            // time at the `Value::String(callee_name.clone())` call
            // when emitting `Op::CallBuiltin` — so every builtin
            // call paid two `String` allocations. Now we only own
            // the name on the two paths that genuinely need an
            // owned value: the `Value::String` constant for the
            // CallBuiltin name and the `CompileError::UnknownFunction`
            // payload. User-fn and FFI calls (the common case) get
            // through with zero `String` clones from the callee
            // identifier. The `&str` borrows from `function.as_ref()`
            // which is alive for the whole match arm; the recursive
            // `compile_expr(arg, ...)` calls borrow disjoint
            // sub-nodes of `arguments`, so the borrow checker is
            // happy.
            // Support indirect calls: if callee is a local variable (not a named
            // fn/ffi) holding a closure, push it and emit CallClosure { arity }.
            if let Node::Identifier { name, .. } = function.as_ref() {
                let is_named =
                    fn_index.contains_key(name.as_str()) || ffi_index.contains_key(name.as_str());
                if let (false, Some(&slot)) = (is_named, locals.get(name.as_str())) {
                    chunk.emit(Op::LoadLocal(slot), line);
                    let arity = arguments.len();
                    for arg in arguments {
                        compile_expr(
                            arg,
                            chunk,
                            locals,
                            next_local,
                            fn_index,
                            ffi_index,
                            fns,
                            next_fn_idx,
                            line,
                        )?;
                    }
                    if arity > u8::MAX as usize {
                        return Err(CompileError::Unsupported("too many args in indirect call"));
                    }
                    let pc = chunk.emit(
                        Op::CallClosure {
                            arity: arity as u8,
                            source_slot: slot,
                        },
                        line,
                    );
                    chunk.record_call_col(pc, call_col);
                    return Ok(());
                }
            }
            // RES-2542: method call — `target.method(args)` compiles to
            // `CallMethod { method_const, arity }`. The receiver is pushed
            // first, then the arguments, so the VM can prepend it as `self`.
            if let Node::FieldAccess { target, field, .. } = function.as_ref() {
                compile_expr(
                    target,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
                for arg in arguments {
                    compile_expr(
                        arg,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                }
                let method_const = chunk.add_string_constant(field)?;
                let arity = arguments.len();
                if arity > u8::MAX as usize {
                    return Err(CompileError::Unsupported("method call with > 255 args"));
                }
                let pc = chunk.emit(
                    Op::CallMethod {
                        method_const,
                        arity: arity as u8,
                    },
                    line,
                );
                chunk.record_call_col(pc, call_col);
                return Ok(());
            }
            // RES-3993: any other callee expression (an immediately-invoked
            // `Node::FunctionLiteral` like `fn(x) { .. }(10)` — the shape
            // array-comprehension desugaring and nested-closure examples
            // produce — an `IndexExpression`, a parenthesized sub-call, etc.)
            // is compiled generically: evaluate it for its `Value::Closure`
            // / `Value::EnumConstructor`, then dispatch through the same
            // `CallClosure` op the named-local-closure path above uses.
            // `source_slot: u16::MAX` marks it "temporary" (see
            // `Op::CallClosure`'s doc comment) — there's no caller-frame
            // local to write mutated upvalues back to, matching the
            // tree-walker's `eval(function)` → `apply_function` dispatch,
            // which evaluates the callee expression generically with no
            // notion of a "home" binding either.
            if !matches!(function.as_ref(), Node::Identifier { .. }) {
                compile_expr(
                    function,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
                let arity = arguments.len();
                for arg in arguments {
                    compile_expr(
                        arg,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                }
                if arity > u8::MAX as usize {
                    return Err(CompileError::Unsupported("too many args in indirect call"));
                }
                let pc = chunk.emit(
                    Op::CallClosure {
                        arity: arity as u8,
                        source_slot: u16::MAX,
                    },
                    line,
                );
                chunk.record_call_col(pc, call_col);
                return Ok(());
            }
            let callee_name: &str = match function.as_ref() {
                Node::Identifier { name, .. } => name.as_str(),
                _ => unreachable!("non-Identifier callees handled above"),
            };
            // FFI v2: foreign call takes priority over user-defined functions.
            if let Some(&idx) = ffi_index.get(callee_name) {
                for arg in arguments {
                    compile_expr(
                        arg,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                }
                let pc = chunk.emit(Op::CallForeign(idx), line);
                chunk.record_call_col(pc, call_col);
                return Ok(());
            }
            // User-defined function next.
            if let Some(&callee_idx) = fn_index.get(callee_name) {
                // Push args left-to-right so the VM can pop them in reverse
                // and assign to locals 0..arity in source order.
                for arg in arguments {
                    compile_expr(
                        arg,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                }
                let pc = chunk.emit(Op::Call(callee_idx), line);
                chunk.record_call_col(pc, call_col);
                return Ok(());
            }
            // RES-VM (issue #266): fall back to the canonical builtin
            // table. The tree walker dispatches builtins through
            // `Value::Builtin`; the bytecode VM achieves the same by
            // emitting `Op::CallBuiltin { name_const, arity }`. Limit
            // arity to u8 so the opcode stays Copy + 4 bytes; calls
            // with >255 args are rejected before any code is emitted.
            //
            // RES-3993: `array_none` isn't in `lookup_builtin`'s static
            // table — its tree-walker implementation needs
            // `&mut Interpreter` to invoke the caller's predicate
            // closure, so it's special-cased directly in the
            // interpreter's `CallExpression` eval instead of going
            // through `Value::Builtin`. The VM's `Op::CallBuiltin`
            // dispatch has a matching runtime special case
            // (`vm_array_none_builtin`); route the call through the
            // same op here so it reaches that handler instead of
            // falling all the way to `UnknownFunction`.
            if crate::lookup_builtin(callee_name).is_some()
                || crate::stdlib::is_stdlib_function(callee_name)
                || callee_name == "array_none"
                || callee_name == "stacktrace"
            {
                if arguments.len() > u8::MAX as usize {
                    return Err(CompileError::Unsupported("builtin call with > 255 args"));
                }
                let name_const = chunk.add_string_constant(callee_name)?;
                for arg in arguments {
                    compile_expr(
                        arg,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                }
                chunk.emit(
                    Op::CallBuiltin {
                        name_const,
                        arity: arguments.len() as u8,
                    },
                    line,
                );
                return Ok(());
            }
            // Enum tuple constructor: `Type::Variant(arg1, arg2, ...)`
            if let Some((type_name, variant_name)) = crate::split_qualified(callee_name) {
                let is_tuple = ENUM_INDEX.with(|ei| {
                    ei.borrow()
                        .get(type_name)
                        .and_then(|vs| vs.iter().find(|v| v.name == variant_name))
                        .is_some_and(|v| matches!(v.payload, crate::EnumPayload::Tuple(_)))
                });
                if is_tuple {
                    for arg in arguments {
                        compile_expr(
                            arg,
                            chunk,
                            locals,
                            next_local,
                            fn_index,
                            ffi_index,
                            fns,
                            next_fn_idx,
                            line,
                        )?;
                    }
                    let tc = chunk.add_string_constant(type_name)?;
                    let vc = chunk.add_string_constant(variant_name)?;
                    chunk.emit(
                        Op::MakeEnumTuple {
                            type_const: tc,
                            variant_const: vc,
                            arity: arguments.len() as u16,
                        },
                        line,
                    );
                    return Ok(());
                }
            }
            // RES-3993: tuple-struct named constructor — `Point(0, 0)`
            // for `struct Point(int, int);`. The tree-walker registers a
            // synthetic `Value::Function` under the struct's own name at
            // `StructDecl` eval time (`crate::tuple_struct::make_constructor`)
            // whose body is just a `StructLiteral` with positional fields
            // `"0"`, `"1"`, ... bound to the call arguments — there's no
            // fn_index slot to route through here, so emit that same
            // `StructLiteral` construction directly instead of trying to
            // manufacture a callable.
            if let Some(field_names) = tuple_struct_field_names(callee_name) {
                if arguments.len() != field_names.len() {
                    return Err(CompileError::Unsupported(
                        "tuple struct constructor called with wrong argument count",
                    ));
                }
                for (field_name, arg) in field_names.iter().zip(arguments) {
                    let fname_idx = chunk.add_string_constant(field_name)?;
                    chunk.emit(Op::Const(fname_idx), line);
                    compile_expr(
                        arg,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                }
                let name_const = chunk.add_string_constant(callee_name)?;
                chunk.emit(
                    Op::StructLiteral {
                        name_const,
                        field_count: field_names.len() as u16,
                    },
                    line,
                );
                return Ok(());
            }
            Err(CompileError::UnknownFunction(callee_name.to_string()))
        }
        // RES-171a: `[a, b, c]` literal → emit each item's expression
        // left-to-right, then `Op::MakeArray { len }` which pops them
        // all into a `Value::Array`.
        Node::ArrayLiteral { items, .. } => {
            if items.len() > u16::MAX as usize {
                return Err(CompileError::Unsupported("array literal with >65535 items"));
            }
            for item in items {
                compile_expr(
                    item,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
            }
            chunk.emit(
                Op::MakeArray {
                    len: items.len() as u16,
                },
                line,
            );
            Ok(())
        }
        // RES-171a: `target[index]` read → push target, push index,
        // emit `LoadIndex`. Bounds + type checks happen in the VM.
        // Nested targets (e.g. `a[i][j]`) fall out naturally because
        // `compile_expr(target)` recurses: each `IndexExpression` at
        // an inner position pushes a clone of the sub-array.
        //
        // RES-407: if the typechecker's bounds-check pass discharged
        // `0 <= index < len(target)` at this exact source span, emit
        // the `LoadIndexUnchecked` variant — the runtime check is
        // redundant and the elision is what hot-loop embedded code
        // wants. Falls back to the checked op when the pass hasn't
        // run or didn't prove this site.
        Node::IndexExpression {
            target,
            index,
            span,
        } => {
            compile_expr(
                target,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            compile_expr(
                index,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let op = if crate::bounds_check::is_proven_site(*span) {
                Op::LoadIndexUnchecked
            } else {
                Op::LoadIndex
            };
            chunk.emit(op, line);
            Ok(())
        }
        // RES-335: `Name { f1: e1, f2: e2 }` struct literal. Lowered
        // as alternating `(field-name-const, value)` pushes followed
        // by `StructLiteral { name_const, field_count }`. Field names
        // live in the constant pool so `Op` stays `Copy`.
        Node::StructLiteral {
            name, fields, base, ..
        } => {
            if fields.len() > u16::MAX as usize {
                return Err(CompileError::TooManyFields(name.clone()));
            }
            // Check if this is a named-field enum constructor (Type::Variant { ... }).
            if let Some((type_name, variant_name)) = crate::split_qualified(name) {
                let is_named = ENUM_INDEX.with(|ei| {
                    ei.borrow()
                        .get(type_name)
                        .and_then(|vs| vs.iter().find(|v| v.name == variant_name))
                        .is_some_and(|v| matches!(v.payload, crate::EnumPayload::Named(_)))
                });
                if is_named {
                    for (field_name, field_expr) in fields {
                        let fname_idx = chunk.add_string_constant(field_name)?;
                        chunk.emit(Op::Const(fname_idx), line);
                        compile_expr(
                            field_expr,
                            chunk,
                            locals,
                            next_local,
                            fn_index,
                            ffi_index,
                            fns,
                            next_fn_idx,
                            line,
                        )?;
                    }
                    let tc = chunk.add_string_constant(type_name)?;
                    let vc = chunk.add_string_constant(variant_name)?;
                    chunk.emit(
                        Op::MakeEnumNamed {
                            type_const: tc,
                            variant_const: vc,
                            field_count: fields.len() as u16,
                        },
                        line,
                    );
                    return Ok(());
                }
            }
            let name_const = chunk.add_string_constant(name)?;
            // RES-3994: `{ ..base, f: v }` struct-update syntax. This
            // arm previously dropped `base` on the floor entirely (the
            // old `Node::StructLiteral { name, fields, .. }` pattern
            // ignored it), so the resulting struct was missing every
            // field it didn't explicitly override — surfaced at
            // runtime as `struct Config has no field 'port'` under
            // `--vm`. Mirror the interpreter (lib.rs's `StructLiteral`
            // eval): evaluate `base` once, into a hidden local so its
            // side effects don't re-run per field, then read every
            // declared field this literal doesn't override off of it
            // via `Op::GetField` before the explicit fields.
            if let Some(base_expr) = base {
                if *next_local == u16::MAX {
                    return Err(CompileError::TooManyLocals);
                }
                let base_slot = *next_local;
                *next_local += 1;
                compile_expr(
                    base_expr,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
                chunk.emit(Op::StoreLocal(base_slot), line);

                let declared_fields: Vec<String> = STRUCT_FIELD_INDEX
                    .with(|si| si.borrow().get(name).cloned())
                    .unwrap_or_default();
                let overridden: std::collections::HashSet<&str> =
                    fields.iter().map(|(n, _)| n.as_str()).collect();

                let mut field_count: u16 = 0;
                for fname in &declared_fields {
                    if overridden.contains(fname.as_str()) {
                        continue;
                    }
                    let fname_idx = chunk.add_string_constant(fname)?;
                    chunk.emit(Op::Const(fname_idx), line);
                    chunk.emit(Op::LoadLocal(base_slot), line);
                    let get_field_const = chunk.add_string_constant(fname)?;
                    chunk.emit(
                        Op::GetField {
                            name_const: get_field_const,
                        },
                        line,
                    );
                    field_count = field_count
                        .checked_add(1)
                        .ok_or_else(|| CompileError::TooManyFields(name.clone()))?;
                }
                for (field_name, field_expr) in fields {
                    let fname_idx = chunk.add_string_constant(field_name)?;
                    chunk.emit(Op::Const(fname_idx), line);
                    compile_expr(
                        field_expr,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                    field_count = field_count
                        .checked_add(1)
                        .ok_or_else(|| CompileError::TooManyFields(name.clone()))?;
                }
                chunk.emit(
                    Op::StructLiteral {
                        name_const,
                        field_count,
                    },
                    line,
                );
                return Ok(());
            }
            for (field_name, field_expr) in fields {
                let fname_idx = chunk.add_string_constant(field_name)?;
                chunk.emit(Op::Const(fname_idx), line);
                compile_expr(
                    field_expr,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
            }
            chunk.emit(
                Op::StructLiteral {
                    name_const,
                    field_count: fields.len() as u16,
                },
                line,
            );
            Ok(())
        }
        // RES-335: `target.field` read → push target, emit `GetField`.
        // Nested reads (`a.b.c`) fall out of the recursion because
        // `compile_expr(target)` re-enters this arm for inner
        // `FieldAccess` nodes.
        Node::FieldAccess { target, field, .. } => {
            compile_expr(
                target,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let fname_idx = chunk.add_string_constant(field)?;
            chunk.emit(
                Op::GetField {
                    name_const: fname_idx,
                },
                line,
            );
            Ok(())
        }
        // RES-401: `(a, b, c)` tuple literal — compile each item left-
        // to-right then emit `MakeTuple { len }` to pack them.
        Node::TupleLiteral { items, .. } => {
            if items.len() > u16::MAX as usize {
                return Err(CompileError::Unsupported("tuple literal with >65535 items"));
            }
            for item in items {
                compile_expr(
                    item,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
            }
            chunk.emit(
                Op::MakeTuple {
                    len: items.len() as u16,
                },
                line,
            );
            Ok(())
        }
        // RES-401: `tuple.N` — compile the tuple, push the index as an
        // integer constant, emit `LoadIndex` (which handles both arrays
        // and tuples in the VM). The typechecker ensures `index` is
        // within the declared tuple length.
        Node::TupleIndex { tuple, index, .. } => {
            compile_expr(
                tuple,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let idx_const = chunk.add_constant(Value::Int(*index as i64))?;
            chunk.emit(Op::Const(idx_const), line);
            chunk.emit(Op::LoadIndex, line);
            Ok(())
        }
        // RES-221: interpolated string `"hello {name}!"` — lower to
        // `to_string()` calls on each expr part, then fold all parts
        // (literals are inlined as string constants) with `Op::Add`.
        //
        // Lowering: push N string values, then emit N-1 Add ops.
        // Empty interpolation (no parts) emits a single `""` constant.
        Node::InterpolatedString { parts, .. } => {
            if parts.is_empty() {
                let idx = chunk.add_string_constant("")?;
                chunk.emit(Op::Const(idx), line);
                return Ok(());
            }
            let to_string_idx = chunk.add_string_constant("to_string")?;
            for part in parts {
                match part {
                    crate::string_interp::StringPart::Literal(s) => {
                        let idx = chunk.add_string_constant(s)?;
                        chunk.emit(Op::Const(idx), line);
                    }
                    crate::string_interp::StringPart::Expr(expr) => {
                        compile_expr(
                            expr,
                            chunk,
                            locals,
                            next_local,
                            fn_index,
                            ffi_index,
                            fns,
                            next_fn_idx,
                            line,
                        )?;
                        chunk.emit(
                            Op::CallBuiltin {
                                name_const: to_string_idx,
                                arity: 1,
                            },
                            line,
                        );
                    }
                }
            }
            for _ in 1..parts.len() {
                chunk.emit(Op::Add, line);
            }
            Ok(())
        }
        // RES-163: `match scrutinee { pat => body, ... }` — lower to
        // a sequence of pattern checks followed by JumpIfFalse / Jump
        // instructions. Supports: Wildcard, Literal, Identifier,
        // Range, Or (literal branches only), Bind. Complex patterns
        // (Struct, Enum, Some/None/Ok/Err, Tuple) return Unsupported.
        Node::Match {
            scrutinee, arms, ..
        } => compile_match_expr(
            scrutinee,
            arms,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-148: `{ k1: v1, k2: v2 }` map literal. Lowered to a
        // `map_new()` call followed by N `map_insert(map, k, v)` calls.
        // All three builtins are in the BUILTINS table so the VM's
        // CallBuiltin dispatch can reach them without new opcodes.
        Node::MapLiteral { entries, .. } => {
            let map_new_idx = chunk.add_string_constant("map_new")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: map_new_idx,
                    arity: 0,
                },
                line,
            );
            if !entries.is_empty() {
                let map_insert_idx = chunk.add_string_constant("map_insert")?;
                for (k, v) in entries {
                    compile_expr(
                        k,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                    compile_expr(
                        v,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                    chunk.emit(
                        Op::CallBuiltin {
                            name_const: map_insert_idx,
                            arity: 3,
                        },
                        line,
                    );
                }
            }
            Ok(())
        }
        // RES-149: `#{v1, v2, v3}` set literal. Lowered to a
        // `set_new()` call followed by N `set_insert(set, item)` calls.
        Node::SetLiteral { items, .. } => {
            let set_new_idx = chunk.add_string_constant("set_new")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: set_new_idx,
                    arity: 0,
                },
                line,
            );
            if !items.is_empty() {
                let set_insert_idx = chunk.add_string_constant("set_insert")?;
                for item in items {
                    compile_expr(
                        item,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                    chunk.emit(
                        Op::CallBuiltin {
                            name_const: set_insert_idx,
                            arity: 2,
                        },
                        line,
                    );
                }
            }
            Ok(())
        }
        // RES-169d: `fn(params) { body }` anonymous function literal.
        // Compiles the body as a new Function entry, collects free variables
        // (capture-by-value), and emits MakeClosure.
        Node::FunctionLiteral {
            parameters, body, ..
        } => {
            if parameters.len() > u8::MAX as usize {
                return Err(CompileError::Unsupported("fn literal with >255 params"));
            }
            if *next_fn_idx == u16::MAX {
                return Err(CompileError::Unsupported("too many functions (>65535)"));
            }
            let fn_idx = *next_fn_idx;
            *next_fn_idx += 1;

            // Determine the set of free variables: identifiers in the body that
            // are not the literal's own parameters and are bound in the *outer*
            // locals map. Collect in insertion order for a deterministic capture
            // sequence (needed so LoadUpvalue(i) indices are stable).
            let param_names: std::collections::HashSet<&str> =
                parameters.iter().map(|(_, n)| n.as_str()).collect();
            // RES-3914/RES-4046 boxing semantics now live in
            // `analyze_and_box_captures` (shared with `compile_nested_fn`,
            // RES-4063) — see that function's doc comment.
            let captured = analyze_and_box_captures(body, &param_names, chunk, locals, line)?;

            // Build the closure's local map: params at 0..arity, then upvalues
            // accessible via LoadUpvalue. The body chunk uses LoadUpvalue(i) for
            // captured names, resolved by the inner compilation below.
            let arity = parameters.len() as u8;
            let upvalue_count = captured.len();

            // Build the body chunk for the new Function entry.
            let mut fn_chunk = Chunk::with_capacity(64);
            let mut fn_locals: HashMap<String, u16> =
                HashMap::with_capacity(parameters.len().saturating_mul(2).max(8));
            let mut fn_next_local: u16 = 0;
            for (_, pname) in parameters {
                fn_locals.insert(pname.clone(), fn_next_local);
                fn_next_local += 1;
            }
            // RES-4063: upvalue pseudo-local plumbing (registration +
            // copy-in prologue) now lives in
            // `install_upvalue_locals_and_prologue`, shared with
            // `compile_nested_fn` — see that function's doc comment.
            let upvalue_base = install_upvalue_locals_and_prologue(
                &mut fn_locals,
                &mut fn_next_local,
                &mut fn_chunk,
                &captured,
                line,
            );

            // Compile the body statements.
            let inner_stmts = match body.as_ref() {
                Node::Block { stmts: b, .. } => b.as_slice(),
                single => std::slice::from_ref(single),
            };
            compile_fn_body_stmts(
                inner_stmts,
                &mut fn_chunk,
                &mut fn_locals,
                &mut fn_next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            fn_chunk.emit(Op::ReturnFromCall, 0);

            // RES-2536/RES-4063: rewrite StoreLocal ops that target
            // upvalue slots to StoreUpvalue — see
            // `rewrite_store_upvalues`'s doc comment.
            rewrite_store_upvalues(&mut fn_chunk, upvalue_base, upvalue_count);

            let local_count = fn_next_local;
            // Insert at fn_idx (pre-allocated index). fns may have grown via
            // nested FunctionLiterals; we need to push a placeholder then
            // overwrite it, OR we always push at end (and fn_idx == fns.len()
            // at the time we called *next_fn_idx += 1). Since nested closures
            // also increment next_fn_idx, fn_idx may not equal fns.len() by
            // the time we reach here. Use a placeholder-then-overwrite strategy:
            // extend fns to at least fn_idx+1 with placeholders.
            // RES-4046: a captured static has no valid caller-frame
            // slot to write back into on return (its backing storage
            // is the per-function statics table, not `locals`) —
            // `u16::MAX` is the existing "no write-back target"
            // sentinel (see `CallFrame::source_slot`'s doc comment);
            // `write_back_upvalues`'s bounds check already treats an
            // out-of-range index as a no-op.
            let source_slots: Box<[u16]> = build_upvalue_source_slots(&captured);
            while fns.len() <= fn_idx as usize {
                fns.push(Function {
                    name: "<closure_placeholder>".into(),
                    arity: 0,
                    chunk: Chunk::with_capacity(0),
                    local_count: 0,
                    upvalue_source_slots: Box::default(),
                    fails: Box::default(),
                    postcheck: None,
                });
            }
            fns[fn_idx as usize] = Function {
                name: "<closure>".into(),
                arity,
                chunk: fn_chunk,
                local_count,
                upvalue_source_slots: source_slots,
                fails: Box::default(),
                postcheck: None,
            };

            // RES-4063: capture-load emission now lives in
            // `emit_capture_loads`, shared with `compile_nested_fn` — see
            // that function's doc comment.
            emit_capture_loads(chunk, &captured, line);
            chunk.emit(
                Op::MakeClosure {
                    fn_idx,
                    upvalue_count: upvalue_count as u8,
                },
                line,
            );
            Ok(())
        }
        // RES-152: `b"..."` bytes literal — stored as a Value::Bytes constant.
        Node::BytesLiteral { value, .. } => {
            let idx = chunk.add_constant(Value::Bytes(value.clone()))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        // RES-291 / RES-4000: `lo..hi` / `lo..=hi` range expression.
        // Lowered to the VM-internal `__range(lo, hi, inclusive)`
        // builtin, which constructs a first-class `Value::Range` —
        // mirroring the interpreter's lazy `eval_range_value` (see
        // `lib.rs`) instead of eagerly materializing an `Array`.
        // Before RES-4000 this called the public `array_range(lo, hi)`
        // builtin, so `type_of(1..5)` reported `"array"` under `--vm`
        // vs `"range"` on the interpreter, and range-only operations
        // like `contains(range, x)` had no `(Array, T)` overload.
        Node::Range {
            lo, hi, inclusive, ..
        } => {
            compile_expr(
                lo,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            compile_expr(
                hi,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let incl_idx = chunk.add_constant(Value::Bool(*inclusive))?;
            chunk.emit(Op::Const(incl_idx), line);
            let name_idx = chunk.add_string_constant("__range")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: name_idx,
                    arity: 3,
                },
                line,
            );
            Ok(())
        }
        // RES-921: `target[lo..hi]` / `target[lo..=hi]` slice expression.
        // Lowered to `array_slice(target, lo, hi, inclusive)`.
        // `lo = None` is represented as `Value::Int(0)`;
        // `hi = None` is represented as `Value::Int(-1)` (sentinel: end of array).
        Node::Slice {
            target,
            lo,
            hi,
            inclusive,
            ..
        } => {
            compile_expr(
                target,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            match lo {
                Some(lo_expr) => compile_expr(
                    lo_expr,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?,
                None => {
                    let idx = chunk.add_constant(Value::Int(0))?;
                    chunk.emit(Op::Const(idx), line);
                }
            }
            match hi {
                Some(hi_expr) => compile_expr(
                    hi_expr,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?,
                None => {
                    // RES-2518: use Void as sentinel for "no upper bound" —
                    // Int(-1) collided with the user value -1, causing
                    // xs[0..-1] to return the full array instead of
                    // excluding the last element.
                    let idx = chunk.add_constant(Value::Void)?;
                    chunk.emit(Op::Const(idx), line);
                }
            }
            let incl_idx = chunk.add_constant(Value::Bool(*inclusive))?;
            chunk.emit(Op::Const(incl_idx), line);
            let name_idx = chunk.add_string_constant("array_slice")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: name_idx,
                    arity: 4,
                },
                line,
            );
            Ok(())
        }
        // RES-1857/RES-duration: DurationLiteral is a nanoseconds constant.
        // The nanos value is already computed by the parser; emit it as an Int.
        Node::DurationLiteral { nanos, .. } => {
            let idx = chunk.add_constant(Value::Int(*nanos as i64))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        // RES-newtypes: NewtypeConstruct wraps a value in a one-field struct.
        // The interpreter creates `Struct { name, fields: [("__value", inner)] }`;
        // we replicate that by emitting a string-const for "__value", compiling
        // the inner expression, then emitting StructLiteral with field_count=1.
        Node::NewtypeConstruct {
            type_name, value, ..
        } => {
            let name_const = chunk.add_string_constant(type_name)?;
            let field_name_idx = chunk.add_string_constant("__value")?;
            chunk.emit(Op::Const(field_name_idx), line);
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(
                Op::StructLiteral {
                    name_const,
                    field_count: 1,
                },
                line,
            );
            Ok(())
        }
        // RES-375: TryExpression (`expr?`) — compile the inner expression,
        // then emit TryUnwrap which either leaves the unwrapped value on the
        // stack or triggers an early return from the current function.
        Node::TryExpression { expr: inner, .. } => {
            compile_expr(
                inner,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::TryUnwrap, line);
            Ok(())
        }
        // RES-3993: `object?.field` / `object?.method(args)` — compile
        // `object`, run the shared pre-access unwrap (`OptChainUnwrap`,
        // see its doc comment), branch on the "present" flag it leaves on
        // top of the stack, perform the field/method access on the
        // present path, then wrap the access result with `Some(..)` —
        // mirrors `Interpreter::eval`'s `Node::OptionalChain` arm exactly
        // (unwrap-or-short-circuit, then access, then re-wrap in Some).
        Node::OptionalChain { object, access, .. } => {
            compile_expr(
                object,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::OptChainUnwrap, line);
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            // present path: stack top is the unwrapped-or-passthrough value.
            match access {
                ChainAccess::Field(field) => {
                    let fname_idx = chunk.add_string_constant(field)?;
                    chunk.emit(
                        Op::GetField {
                            name_const: fname_idx,
                        },
                        line,
                    );
                }
                ChainAccess::Method(method, arg_nodes) => {
                    for arg in arg_nodes {
                        compile_expr(
                            arg,
                            chunk,
                            locals,
                            next_local,
                            fn_index,
                            ffi_index,
                            fns,
                            next_fn_idx,
                            line,
                        )?;
                    }
                    let method_const = chunk.add_string_constant(method)?;
                    if arg_nodes.len() > u8::MAX as usize {
                        return Err(CompileError::Unsupported(
                            "optional-chain method call with > 255 args",
                        ));
                    }
                    chunk.emit(
                        Op::CallMethod {
                            method_const,
                            arity: arg_nodes.len() as u8,
                        },
                        line,
                    );
                }
            }
            let some_const = chunk.add_string_constant("Some")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: some_const,
                    arity: 1,
                },
                line,
            );
            let jmp_end = chunk.emit(Op::Jump(0), line);
            // absent path: stack top is already `Option(None)`.
            let absent_target = chunk.code.len();
            chunk.patch_jump(jif, absent_target)?;
            let end = chunk.code.len();
            chunk.patch_jump(jmp_end, end)?;
            Ok(())
        }
        // RES-325: NamedArg — the name is a type-check annotation only;
        // for bytecode purposes just compile the value.
        Node::NamedArg { value, .. } => compile_expr(
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-2516: `if`/`else` in expression position. The parser reuses
        // `Node::IfStatement` for both statement and expression contexts.
        // Compile both branches so each leaves exactly one value on the stack.
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            compile_expr(
                condition,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_block_as_expr(
                consequence,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jmp_end = chunk.emit(Op::Jump(0), line);
            let else_pc = chunk.code.len();
            chunk.patch_jump(jif, else_pc)?;
            if let Some(alt) = alternative {
                compile_block_as_expr(
                    alt,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
            } else {
                let void_idx = chunk.add_constant(Value::Void)?;
                chunk.emit(Op::Const(void_idx), line);
            }
            let end_pc = chunk.code.len();
            chunk.patch_jump(jmp_end, end_pc)?;
            Ok(())
        }
        // RES-3920: a block in expression position — most commonly a
        // block-bodied `match` arm (`5 => { let y = x + 1; println(y); }`)
        // or `if`/`else` branch value. Delegates to the vetted
        // `compile_block_as_expr` (leading statements + last-expr value,
        // empty → Void), the same lowering `if` already uses. Previously
        // this fell through to `Unsupported("Block")`, so any block-bodied
        // match arm failed to compile under `--vm` while the interpreter
        // ran it.
        Node::Block { .. } => compile_block_as_expr(
            node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-3993: `while`/`loop` in expression position (`let x =
        // loop { ...; break v; };` — see `compile_while_expr`). Every
        // other control-flow node already has a distinct expression-
        // position lowering here (`IfStatement` above, `Match` via
        // `compile_match_expr`); `WhileStatement` previously had none,
        // so `break <expr>` had no value channel and fell through to
        // `Unsupported("WhileStatement")`.
        Node::WhileStatement {
            condition,
            body,
            label,
            ..
        } => compile_while_expr(
            condition,
            body,
            label,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-4060: `forall`/`exists` quantifier expressions. See
        // `compile_quantifier_expr` for the lowering shape.
        Node::Quantifier {
            kind,
            var,
            range,
            body,
            ..
        } => compile_quantifier_expr(
            *kind,
            var,
            range,
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// RES-4060: compile `forall VAR in RANGE: BODY` / `exists VAR in RANGE:
/// BODY` to bytecode. Mirrors the tree-walker's
/// `crate::quantifiers::eval_quantifier`: a short-circuiting loop over
/// either a bounded integer range (`lo..hi`) or an arbitrary iterable
/// (array/set/bytes), binding `var` fresh each iteration and evaluating
/// `body` as a boolean expression per witness. `forall` starts `true`
/// and flips to `false` (then exits) on the first false witness;
/// `exists` starts `false` and flips to `true` (then exits) on the
/// first true witness.
///
/// Reuses `compile_for_in`'s iteration shape: the range/iterable side
/// is normalized via `Op::IterPrepare` into an array (the same op
/// `for`-loops use for `Value::Range`/`Value::Set`/`Value::Bytes`), then
/// walked with the hidden `arr`/`len`/`idx` locals. `QuantRange::Range`
/// is lowered exactly like `Node::Range` (`compile_expr`'s
/// `Node::Range` arm above) — a `lo..hi` half-open range via the
/// `__range` builtin — since the quantifier grammar has no `inclusive`
/// form.
#[allow(clippy::too_many_arguments)]
fn compile_quantifier_expr(
    kind: crate::quantifiers::QuantifierKind,
    var: &str,
    range: &crate::quantifiers::QuantRange,
    body: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    if (*next_local as usize) + 5 > u16::MAX as usize {
        return Err(CompileError::TooManyLocals);
    }
    let arr_slot = *next_local;
    *next_local += 1;
    let len_slot = *next_local;
    *next_local += 1;
    let idx_slot = *next_local;
    *next_local += 1;
    let result_slot = *next_local;
    *next_local += 1;
    let arr_key = format!("$quant_arr@{}", arr_slot);
    let len_key = format!("$quant_len@{}", len_slot);
    let idx_key = format!("$quant_idx@{}", idx_slot);
    let result_key = format!("$quant_result@{}", result_slot);
    locals.insert(arr_key.clone(), arr_slot);
    locals.insert(len_key.clone(), len_slot);
    locals.insert(idx_key.clone(), idx_slot);
    locals.insert(result_key.clone(), result_slot);

    // Quantified variable: shadow any outer binding for the duration
    // of the loop, restored afterward — same shape as `compile_for_in`.
    let prev_var_slot = locals.get(var).copied();
    let var_slot = *next_local;
    *next_local += 1;
    locals.insert(var.to_string(), var_slot);

    // 1. Evaluate the range/iterable onto the stack as a single value,
    //    normalize with `IterPrepare`, store in arr_slot.
    match range {
        crate::quantifiers::QuantRange::Range { lo, hi } => {
            compile_expr(
                lo,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            compile_expr(
                hi,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let incl_idx = chunk.add_constant(Value::Bool(false))?;
            chunk.emit(Op::Const(incl_idx), line);
            let name_idx = chunk.add_string_constant("__range")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: name_idx,
                    arity: 3,
                },
                line,
            );
        }
        crate::quantifiers::QuantRange::Iterable(expr) => {
            compile_expr(
                expr,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
        }
    }
    chunk.emit(Op::IterPrepare, line);
    chunk.emit(Op::StoreLocal(arr_slot), line);

    // 2. len = len(arr)
    let len_name_const = chunk.add_string_constant("len")?;
    chunk.emit(Op::LoadLocal(arr_slot), line);
    chunk.emit(
        Op::CallBuiltin {
            name_const: len_name_const,
            arity: 1,
        },
        line,
    );
    chunk.emit(Op::StoreLocal(len_slot), line);

    // 3. idx = 0
    let zero_const = chunk.add_constant(Value::Int(0))?;
    chunk.emit(Op::Const(zero_const), line);
    chunk.emit(Op::StoreLocal(idx_slot), line);

    // 4. result = (kind == Forall) — the vacuous-range answer, flipped
    //    on the first witness that disproves it.
    let initial = matches!(kind, crate::quantifiers::QuantifierKind::Forall);
    let initial_const = chunk.add_constant(Value::Bool(initial))?;
    chunk.emit(Op::Const(initial_const), line);
    chunk.emit(Op::StoreLocal(result_slot), line);

    // 5. Loop test: idx < len.
    let loop_start = chunk.code.len();
    chunk.emit(Op::LoadLocal(idx_slot), line);
    chunk.emit(Op::LoadLocal(len_slot), line);
    chunk.emit(Op::Lt, line);
    let jif = chunk.emit(Op::JumpIfFalse(0), line);

    // 6. var = arr[idx]
    chunk.emit(Op::LoadLocal(arr_slot), line);
    chunk.emit(Op::LoadLocal(idx_slot), line);
    chunk.emit(Op::LoadIndex, line);
    chunk.emit(Op::StoreLocal(var_slot), line);

    // 7. Evaluate the body (must produce a Bool) and short-circuit:
    //    `forall` exits on the first `false` witness; `exists` exits
    //    on the first `true` witness. Otherwise fall through to the
    //    index increment and loop again.
    compile_expr(
        body,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    let short_circuit_jump = match kind {
        crate::quantifiers::QuantifierKind::Forall => chunk.emit(Op::JumpIfTrue(0), line),
        crate::quantifiers::QuantifierKind::Exists => chunk.emit(Op::JumpIfFalse(0), line),
    };
    let flipped = !initial;
    let flipped_const = chunk.add_constant(Value::Bool(flipped))?;
    chunk.emit(Op::Const(flipped_const), line);
    chunk.emit(Op::StoreLocal(result_slot), line);
    let exit_jump = chunk.emit(Op::Jump(0), line);
    let continue_target = chunk.code.len();
    chunk.patch_jump(short_circuit_jump, continue_target)?;

    // 8. idx = idx + 1; loop back to the test.
    chunk.emit(Op::LoadLocal(idx_slot), line);
    let one_const = chunk.add_constant(Value::Int(1))?;
    chunk.emit(Op::Const(one_const), line);
    chunk.emit(Op::Add, line);
    chunk.emit(Op::StoreLocal(idx_slot), line);
    let jmp = chunk.emit(Op::Jump(0), line);
    chunk.patch_jump(jmp, loop_start)?;

    // 9. Exit: leave `result` on the stack as the expression's value.
    let end = chunk.code.len();
    chunk.patch_jump(jif, end)?;
    chunk.patch_jump(exit_jump, end)?;
    chunk.emit(Op::LoadLocal(result_slot), line);

    locals.remove(&arr_key);
    locals.remove(&len_key);
    locals.remove(&idx_key);
    locals.remove(&result_key);
    if let Some(prev) = prev_var_slot {
        locals.insert(var.to_string(), prev);
    } else {
        locals.remove(var);
    }
    Ok(())
}

/// RES-3993: `while`/`loop` used in *expression* position — most
/// commonly `loop { ...; break <expr>; }` used as a `let` binding's
/// value (`loop { }` desugars to a `WhileStatement` with an always-true
/// condition — see `parse_loop_statement` in `lib.rs`). Mirrors
/// `compile_control_flow`'s statement-position `WhileStatement` lowering,
/// but the loop always leaves exactly one value on the stack: `Void` if
/// the loop exits via its condition going false or a plain `break;`/
/// `break label;` (matching the tree-walker's `Ok(Value::Void)`
/// fallthrough at the end of `eval`'s `WhileStatement` arm), or the
/// `break <expr>` value if one fired (matching `Value::BreakWith`
/// short-circuiting `eval` to return that value directly).
///
/// Gets its own fresh, self-contained `loop_stack` — mirroring
/// `compile_block_as_expr`'s `&mut Vec::new()` convention for
/// expression-position statement compilation — so nested loops inside
/// the body push/pop correctly. Break/continue targeting an *enclosing*
/// loop from inside an expression-position loop body shares the same
/// pre-existing gap `compile_block_as_expr` already has for `if`/
/// match-arm blocks (no outer `loop_stack` is threaded through
/// expression-position compilation at all) — out of scope for this fix.
#[allow(clippy::too_many_arguments)]
fn compile_while_expr(
    condition: &Node,
    body: &Node,
    label: &Option<String>,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    let loop_start = chunk.code.len();
    compile_expr(
        condition,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    let jif = chunk.emit(Op::JumpIfFalse(0), line);
    let mut inner_loop_state = LoopState::with_label(loop_start, label.clone());
    inner_loop_state.value_mode = true;
    let mut loop_stack = vec![inner_loop_state];
    compile_stmt_in_fn(
        body,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
        &mut loop_stack,
    )?;
    let inner = loop_stack.pop().unwrap();
    let jmp = chunk.emit(Op::Jump(0), line);
    chunk.patch_jump(jmp, loop_start)?;
    let void_exit = chunk.code.len();
    chunk.patch_jump(jif, void_exit)?;
    for p in inner.break_patches {
        chunk.patch_jump(p, void_exit)?;
    }
    let void_idx = chunk.add_constant(Value::Void)?;
    chunk.emit(Op::Const(void_idx), line);
    let after = chunk.code.len();
    for p in inner.break_value_patches {
        chunk.patch_jump(p, after)?;
    }
    for p in inner.continue_patches {
        chunk.patch_jump(p, loop_start)?;
    }
    Ok(())
}

/// Compile a node in "expression" position — it must leave exactly one
/// value on the stack. For `Block` nodes, all statements except the last
/// are compiled as statements (no stack residue); the last statement is
/// compiled as an expression. For non-`Block` nodes, delegates to
/// `compile_expr`.
#[allow(clippy::too_many_arguments)]
fn compile_block_as_expr(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::Block { stmts, .. } => {
            if stmts.is_empty() {
                let void_idx = chunk.add_constant(Value::Void)?;
                chunk.emit(Op::Const(void_idx), line);
                return Ok(());
            }
            let mut block_locals = locals.clone();
            let (leading, last) = stmts.split_at(stmts.len() - 1);
            for stmt in leading {
                let stmt_line = node_line(stmt).unwrap_or(line);
                compile_stmt_in_fn(
                    stmt,
                    chunk,
                    &mut block_locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    stmt_line,
                    &mut Vec::new(),
                )?;
            }
            let last_node = &last[0];
            let last_line = node_line(last_node).unwrap_or(line);
            match last_node {
                Node::ExpressionStatement { expr, .. } => compile_expr(
                    expr,
                    chunk,
                    &mut block_locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    last_line,
                ),
                // RES-3993: `return <expr>;` as a block-in-expression-
                // position's trailing statement — most commonly a
                // block-bodied match arm (`Pat => { ...; return x; }`,
                // see `match_block_arms.rz`). `compile_expr` has no arm
                // for `ReturnStatement` (it isn't a value-producing
                // expression — it unconditionally exits the enclosing
                // function), so routing it through the `_` fallback
                // below previously hit `Unsupported("ReturnStatement")`.
                // Delegate to `compile_stmt_in_fn`, which already lowers
                // `return` to `<value>; Op::ReturnFromCall` exactly like
                // an ordinary function-body statement — the same
                // "always assume function context" convention this
                // function's `leading` statements already use.
                Node::ReturnStatement { .. } => compile_stmt_in_fn(
                    last_node,
                    chunk,
                    &mut block_locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    last_line,
                    &mut Vec::new(),
                ),
                _ => compile_expr(
                    last_node,
                    chunk,
                    &mut block_locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    last_line,
                ),
            }
        }
        _ => compile_expr(
            node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
    }
}

/// RES-3997: analog of [`compile_fn_body_stmts`] for the top-level `main`
/// chunk's statement list (see `compile`'s "Pass 3"). `compile_stmt` /
/// `compile_control_flow` differ from `compile_stmt_in_fn` /
/// `compile_control_flow_in_fn` (top-level globals vs. function-local
/// locals), so this mirrors the same "pop everything except a trailing
/// bare expression" split against the top-level statement compiler
/// rather than sharing one generic helper across both. A bare trailing
/// top-level expression-statement (no `let`/`return`/assignment) is the
/// program's implicit result value — `vm::run` returns it, and the
/// `--vm` CLI driver prints it when non-`Void` — exactly mirroring how a
/// function body's trailing bare expression is its implicit return
/// value.
#[allow(clippy::too_many_arguments)]
fn compile_top_level_stmts(
    stmts: &[(&Node, u32)],
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
) -> Result<(), CompileError> {
    if stmts.is_empty() {
        return Ok(());
    }
    let (leading, last) = stmts.split_at(stmts.len() - 1);
    for (node, line) in leading {
        compile_stmt(
            node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            *line,
            &mut Vec::new(),
        )?;
    }
    let (last_node, last_line) = last[0];
    match last_node {
        Node::ExpressionStatement { expr, .. } => compile_expr(
            expr,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            last_line,
        ),
        _ => compile_stmt(
            last_node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            last_line,
            &mut Vec::new(),
        ),
    }
}

/// RES-3997: compile a function body's top-level statement list, followed
/// unconditionally by `Op::ReturnFromCall` at every call site of this
/// helper — mirrors `compile_block_as_expr`'s leading/last split so the
/// existing "a trailing bare expression with no `return` keyword is the
/// function's implicit return value" convention survives the discard-pop
/// fix (see `compile_stmt_in_fn`'s `Node::ExpressionStatement` arm): every
/// statement is compiled normally (and popped if unused) *except* a
/// trailing `Node::ExpressionStatement`, whose value is left on the stack
/// instead of popped so the caller's `Op::ReturnFromCall` returns it.
/// Without this split, popping *every* discarded expression-statement
/// (correct for RES-3997's actual bug — a call used as a mid-body
/// statement) would also swallow the value of `fn f() -> int { x * y }`
/// style bodies, since those relied on the very same (buggy) unpopped
/// leak to carry their implicit return value to the trailing
/// `ReturnFromCall`.
#[allow(clippy::too_many_arguments)]
fn compile_fn_body_stmts(
    stmts: &[Node],
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    if stmts.is_empty() {
        return Ok(());
    }
    let (leading, last) = stmts.split_at(stmts.len() - 1);
    for stmt in leading {
        let stmt_line = node_line(stmt).unwrap_or(line);
        compile_stmt_in_fn(
            stmt,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            stmt_line,
            &mut Vec::new(),
        )?;
    }
    let last_node = &last[0];
    let last_line = node_line(last_node).unwrap_or(line);
    match last_node {
        Node::ExpressionStatement { expr, .. } => compile_expr(
            expr,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            last_line,
        ),
        _ => compile_stmt_in_fn(
            last_node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            last_line,
            &mut Vec::new(),
        ),
    }
}

/// Walk `node` collecting identifiers that are free in the expression (not
/// in `param_names`) and bound in `outer_locals`. Results go into `out` in
/// first-seen order; `seen` tracks which names we've already added.
fn collect_free_vars(
    node: &Node,
    param_names: &std::collections::HashSet<&str>,
    outer_locals: &HashMap<String, u16>,
    out: &mut Vec<(u16, String)>,
    seen: &mut std::collections::HashSet<String>,
) {
    match node {
        Node::Identifier { name, .. }
            if !param_names.contains(name.as_str())
                && !seen.contains(name)
                && outer_locals.contains_key(name) =>
        {
            let slot = outer_locals[name];
            seen.insert(name.clone());
            out.push((slot, name.clone()));
        }
        Node::Identifier { .. } => {}
        // Recurse into all child nodes.
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_free_vars(s, param_names, outer_locals, out, seen);
            }
        }
        Node::LetStatement { value, .. } => {
            collect_free_vars(value, param_names, outer_locals, out, seen);
        }
        Node::InfixExpression { left, right, .. } => {
            collect_free_vars(left, param_names, outer_locals, out, seen);
            collect_free_vars(right, param_names, outer_locals, out, seen);
        }
        Node::PrefixExpression { right, .. } => {
            collect_free_vars(right, param_names, outer_locals, out, seen);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_free_vars(function, param_names, outer_locals, out, seen);
            for a in arguments {
                collect_free_vars(a, param_names, outer_locals, out, seen);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_free_vars(condition, param_names, outer_locals, out, seen);
            collect_free_vars(consequence, param_names, outer_locals, out, seen);
            if let Some(alt) = alternative {
                collect_free_vars(alt, param_names, outer_locals, out, seen);
            }
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            collect_free_vars(v, param_names, outer_locals, out, seen);
        }
        Node::ReturnStatement { .. } => {}
        Node::ExpressionStatement { expr, .. } => {
            collect_free_vars(expr, param_names, outer_locals, out, seen);
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_free_vars(condition, param_names, outer_locals, out, seen);
            collect_free_vars(body, param_names, outer_locals, out, seen);
        }
        // RES-2506: walk all remaining expression-bearing node types so
        // that outer variables referenced inside them are captured.
        Node::ForInStatement { iterable, body, .. } => {
            collect_free_vars(iterable, param_names, outer_locals, out, seen);
            collect_free_vars(body, param_names, outer_locals, out, seen);
        }
        Node::Assignment { value, .. } => {
            collect_free_vars(value, param_names, outer_locals, out, seen);
        }
        Node::IndexExpression { target, index, .. } => {
            collect_free_vars(target, param_names, outer_locals, out, seen);
            collect_free_vars(index, param_names, outer_locals, out, seen);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            collect_free_vars(target, param_names, outer_locals, out, seen);
            collect_free_vars(index, param_names, outer_locals, out, seen);
            collect_free_vars(value, param_names, outer_locals, out, seen);
        }
        Node::ArrayLiteral { items, .. }
        | Node::SetLiteral { items, .. }
        | Node::TupleLiteral { items, .. } => {
            for item in items {
                collect_free_vars(item, param_names, outer_locals, out, seen);
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_free_vars(k, param_names, outer_locals, out, seen);
                collect_free_vars(v, param_names, outer_locals, out, seen);
            }
        }
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(expr) = part {
                    collect_free_vars(expr, param_names, outer_locals, out, seen);
                }
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_free_vars(scrutinee, param_names, outer_locals, out, seen);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    collect_free_vars(g, param_names, outer_locals, out, seen);
                }
                collect_free_vars(body, param_names, outer_locals, out, seen);
            }
        }
        Node::Slice { target, lo, hi, .. } => {
            collect_free_vars(target, param_names, outer_locals, out, seen);
            if let Some(lo) = lo {
                collect_free_vars(lo, param_names, outer_locals, out, seen);
            }
            if let Some(hi) = hi {
                collect_free_vars(hi, param_names, outer_locals, out, seen);
            }
        }
        Node::FieldAccess { target, .. } => {
            collect_free_vars(target, param_names, outer_locals, out, seen);
        }
        Node::FieldAssignment { target, value, .. } => {
            collect_free_vars(target, param_names, outer_locals, out, seen);
            collect_free_vars(value, param_names, outer_locals, out, seen);
        }
        Node::LetTupleDestructure { value, .. } => {
            collect_free_vars(value, param_names, outer_locals, out, seen);
        }
        Node::LetDestructureStruct { value, .. } => {
            collect_free_vars(value, param_names, outer_locals, out, seen);
        }
        Node::TupleIndex { tuple, .. } => {
            collect_free_vars(tuple, param_names, outer_locals, out, seen);
        }
        Node::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                collect_free_vars(v, param_names, outer_locals, out, seen);
            }
        }
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            collect_free_vars(condition, param_names, outer_locals, out, seen);
            if let Some(msg) = message {
                collect_free_vars(msg, param_names, outer_locals, out, seen);
            }
        }
        Node::TryExpression { expr, .. } => {
            collect_free_vars(expr, param_names, outer_locals, out, seen);
        }
        Node::NamedArg { value, .. } | Node::NewtypeConstruct { value, .. } => {
            collect_free_vars(value, param_names, outer_locals, out, seen);
        }
        Node::FunctionLiteral { body, .. } => {
            collect_free_vars(body, param_names, outer_locals, out, seen);
        }
        Node::OptionalChain { object, access, .. } => {
            collect_free_vars(object, param_names, outer_locals, out, seen);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    collect_free_vars(a, param_names, outer_locals, out, seen);
                }
            }
        }
        Node::Range { lo, hi, .. } => {
            collect_free_vars(lo, param_names, outer_locals, out, seen);
            collect_free_vars(hi, param_names, outer_locals, out, seen);
        }
        // RES-2512: TryCatch, LiveBlock, Quantifier, StaticLet were
        // falling through to `_ => {}`, silently missing free vars.
        Node::TryCatch { body, handlers, .. } => {
            for s in body {
                collect_free_vars(s, param_names, outer_locals, out, seen);
            }
            for (_, handler_body) in handlers {
                for s in handler_body {
                    collect_free_vars(s, param_names, outer_locals, out, seen);
                }
            }
        }
        Node::LiveBlock {
            body, invariants, ..
        } => {
            collect_free_vars(body, param_names, outer_locals, out, seen);
            for inv in invariants {
                collect_free_vars(inv, param_names, outer_locals, out, seen);
            }
        }
        Node::Quantifier { body, .. } => {
            collect_free_vars(body, param_names, outer_locals, out, seen);
        }
        Node::StaticLet { value, .. } => {
            collect_free_vars(value, param_names, outer_locals, out, seen);
        }
        // Leaf nodes (literals, break/continue, declarations with no
        // expression children) have no free vars.
        _ => {}
    }
}

// ── Match expression lowering ─────────────────────────────────────────────────

/// Compile a `match` expression. The scrutinee is evaluated once and
/// stored in a hidden temp local; each arm is compiled as a
/// pattern-check + optional-guard + body sequence with jump routing.
#[allow(clippy::too_many_arguments)]
fn compile_match_expr(
    scrutinee: &Node,
    arms: &[(crate::Pattern, Option<Node>, Node)],
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    compile_expr(
        scrutinee,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    if *next_local == u16::MAX {
        return Err(CompileError::TooManyLocals);
    }
    let scrutinee_slot = *next_local;
    *next_local += 1;
    chunk.emit(Op::StoreLocal(scrutinee_slot), line);

    // RES-1922: exact upper bound — every arm pushes one entry below
    // (its "jump past the whole match" patch). Skips the default
    // 0→4→8→16 grow chain for matches with ≥ 4 arms (common shape
    // for enum-exhaustive matches over Result / Option / custom sum
    // types). Same shape as RES-1800 / RES-1762 / RES-1796 pre-sizes.
    let mut after_match_patches: Vec<usize> = Vec::with_capacity(arms.len());

    for (pattern, guard, body) in arms {
        // Each arm gets its own mutable locals copy so bindings don't
        // leak across arms. The clone is cheap (typically ≤ 16 entries).
        let mut arm_locals = locals.clone();
        let next_local_snap = *next_local;

        let mut next_arm_patches: Vec<usize> = Vec::new();
        compile_pattern_check(
            pattern,
            scrutinee_slot,
            chunk,
            &mut arm_locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            &mut next_arm_patches,
        )?;

        if let Some(guard_expr) = guard {
            compile_expr(
                guard_expr,
                chunk,
                &mut arm_locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
        }

        compile_expr(
            body,
            chunk,
            &mut arm_locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;

        let after_p = chunk.emit(Op::Jump(0), line);
        after_match_patches.push(after_p);

        let next_arm_pc = chunk.code.len();
        for p in next_arm_patches {
            chunk.patch_jump(p, next_arm_pc)?;
        }

        // Reclaim temp slots used by this arm's bindings.
        *next_local = next_local_snap;
    }

    // Fallthrough (no arm matched) → Void.
    let void_idx = chunk.add_constant(Value::Void)?;
    chunk.emit(Op::Const(void_idx), line);

    let after_match_pc = chunk.code.len();
    for p in after_match_patches {
        chunk.patch_jump(p, after_match_pc)?;
    }
    Ok(())
}

/// RES-3993: `Match` used as a statement rather than an expression —
/// the lowering target for `if let` (RES-908) and `while let` (RES-914),
/// which the parser desugars to a bare `Node::Match` inside a `Block`
/// (see `parse_if_let_statement` / `parse_while_let_statement`). Neither
/// `compile_stmt` nor `compile_stmt_in_fn` had a `Match` arm before this,
/// so every if-let/while-let fell through to the generic
/// `Unsupported("Match")` catch-all.
///
/// Mirrors `compile_match_expr`'s pattern-check/guard machinery, but
/// compiles each arm body with `compile_stmt` (so `return`/`break`/
/// `continue`/assignment/nested-`let` inside an arm behave exactly like
/// any other statement block) instead of `compile_expr`, and does not
/// leave a fallthrough value on the operand stack — a statement-position
/// match's result is never consumed, unlike `compile_match_expr`'s
/// `Value::Void` fallback.
#[allow(clippy::too_many_arguments)]
fn compile_match_stmt(
    scrutinee: &Node,
    arms: &[(crate::Pattern, Option<Node>, Node)],
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_stack: &mut Vec<LoopState>,
) -> Result<(), CompileError> {
    compile_expr(
        scrutinee,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    if *next_local == u16::MAX {
        return Err(CompileError::TooManyLocals);
    }
    let scrutinee_slot = *next_local;
    *next_local += 1;
    chunk.emit(Op::StoreLocal(scrutinee_slot), line);

    let mut after_match_patches: Vec<usize> = Vec::with_capacity(arms.len());

    for (pattern, guard, body) in arms {
        let mut arm_locals = locals.clone();
        let next_local_snap = *next_local;

        let mut next_arm_patches: Vec<usize> = Vec::new();
        compile_pattern_check(
            pattern,
            scrutinee_slot,
            chunk,
            &mut arm_locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            &mut next_arm_patches,
        )?;

        if let Some(guard_expr) = guard {
            compile_expr(
                guard_expr,
                chunk,
                &mut arm_locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
        }

        compile_stmt(
            body,
            chunk,
            &mut arm_locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        )?;

        let after_p = chunk.emit(Op::Jump(0), line);
        after_match_patches.push(after_p);

        let next_arm_pc = chunk.code.len();
        for p in next_arm_patches {
            chunk.patch_jump(p, next_arm_pc)?;
        }

        // Reclaim temp slots used by this arm's bindings.
        *next_local = next_local_snap;
    }

    let after_match_pc = chunk.code.len();
    for p in after_match_patches {
        chunk.patch_jump(p, after_match_pc)?;
    }
    Ok(())
}

/// Same as [`compile_match_stmt`] but routes arm bodies through
/// `compile_stmt_in_fn` so `return` inside an arm emits `ReturnFromCall`
/// (matching how `compile_control_flow_in_fn` mirrors `compile_control_flow`
/// for `if`/`while`/`for`/block statements inside a function body).
#[allow(clippy::too_many_arguments)]
fn compile_match_stmt_in_fn(
    scrutinee: &Node,
    arms: &[(crate::Pattern, Option<Node>, Node)],
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_stack: &mut Vec<LoopState>,
) -> Result<(), CompileError> {
    compile_expr(
        scrutinee,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    if *next_local == u16::MAX {
        return Err(CompileError::TooManyLocals);
    }
    let scrutinee_slot = *next_local;
    *next_local += 1;
    chunk.emit(Op::StoreLocal(scrutinee_slot), line);

    let mut after_match_patches: Vec<usize> = Vec::with_capacity(arms.len());

    for (pattern, guard, body) in arms {
        let mut arm_locals = locals.clone();
        let next_local_snap = *next_local;

        let mut next_arm_patches: Vec<usize> = Vec::new();
        compile_pattern_check(
            pattern,
            scrutinee_slot,
            chunk,
            &mut arm_locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            &mut next_arm_patches,
        )?;

        if let Some(guard_expr) = guard {
            compile_expr(
                guard_expr,
                chunk,
                &mut arm_locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
        }

        compile_stmt_in_fn(
            body,
            chunk,
            &mut arm_locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_stack,
        )?;

        let after_p = chunk.emit(Op::Jump(0), line);
        after_match_patches.push(after_p);

        let next_arm_pc = chunk.code.len();
        for p in next_arm_patches {
            chunk.patch_jump(p, next_arm_pc)?;
        }

        // Reclaim temp slots used by this arm's bindings.
        *next_local = next_local_snap;
    }

    let after_match_pc = chunk.code.len();
    for p in after_match_patches {
        chunk.patch_jump(p, after_match_pc)?;
    }
    Ok(())
}

/// RES-3994: extract the single inner sub-pattern from an
/// `EnumPatternPayload` for `Option`/`Result` bridging in
/// `compile_pattern_check`'s `Pattern::EnumVariant` arm. `Option::Some(v)`
/// / `Result::Ok(v)` / `Result::Err(v)` parse with either a `Tuple`
/// payload (positional call syntax) or a `Named` payload (brace-field
/// syntax) depending on how the qualifier path was written, but both
/// carry exactly one sub-pattern for these built-in types. Returns
/// `None` for the payload-less `Option::None` case or any malformed
/// (non-1-arity) payload, which the caller falls back to `Wildcard`
/// for — a presence-only check, matching the interpreter's own bridge
/// (lib.rs `match_pattern`'s `Pattern::EnumVariant` arm).
fn enum_pattern_payload_inner(payload: &crate::EnumPatternPayload) -> Option<crate::Pattern> {
    match payload {
        crate::EnumPatternPayload::Tuple(pats) if pats.len() == 1 => Some(pats[0].clone()),
        crate::EnumPatternPayload::Named(fields) if fields.len() == 1 => {
            Some((*fields[0].1).clone())
        }
        _ => None,
    }
}

/// Emit code that checks whether the current scrutinee (in `scrutinee_slot`)
/// matches `pattern`. On failure, a `JumpIfFalse(0)` placeholder is appended
/// to `next_arm_patches` (caller patches it to the next arm). On success, any
/// name bindings are added to `locals`.
///
/// Supported: Wildcard, Literal, Identifier, Range, Or (literal branches),
/// Bind(name, inner). Complex structural patterns return `Unsupported`.
#[allow(clippy::too_many_arguments)]
fn compile_pattern_check(
    pattern: &crate::Pattern,
    scrutinee_slot: u16,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    next_arm_patches: &mut Vec<usize>,
) -> Result<(), CompileError> {
    use crate::Pattern;
    match pattern {
        Pattern::Wildcard => {
            // Always matches — no code.
        }
        Pattern::Literal(lit_node) => {
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            compile_expr(
                lit_node,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Eq, line);
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
        }
        Pattern::Identifier(name) => {
            // Bind the scrutinee value to `name`; always matches.
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let slot = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), slot);
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(Op::StoreLocal(slot), line);
        }
        Pattern::Range { lo, hi, inclusive } => {
            // lo <= scrutinee
            let lo_idx = chunk.add_constant(Value::Int(*lo))?;
            chunk.emit(Op::Const(lo_idx), line);
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(Op::Le, line);
            let p1 = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p1);
            // scrutinee <= hi  (or < hi)
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            let hi_idx = chunk.add_constant(Value::Int(*hi))?;
            chunk.emit(Op::Const(hi_idx), line);
            if *inclusive {
                chunk.emit(Op::Le, line);
            } else {
                chunk.emit(Op::Lt, line);
            }
            let p2 = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p2);
        }
        Pattern::Bind(name, inner) => {
            // Check inner pattern first; then bind `name` if it matched.
            compile_pattern_check(
                inner,
                scrutinee_slot,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                next_arm_patches,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let slot = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), slot);
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(Op::StoreLocal(slot), line);
        }
        Pattern::Or(branches) => {
            // Only support Or over Literal / Wildcard branches (no bindings).
            if branches.iter().any(pattern_has_bindings) {
                return Err(CompileError::Unsupported(
                    "Or pattern with identifier bindings",
                ));
            }
            // For each branch except the last: check; on match, jump to
            // or_matched. For the last: check; on fail, fall to next_arm.
            let mut or_matched_patches: Vec<usize> = Vec::new();
            for (i, branch) in branches.iter().enumerate() {
                let is_last = i == branches.len() - 1;
                if is_last {
                    // Last branch: normal "fail → next arm" check.
                    compile_pattern_check(
                        branch,
                        scrutinee_slot,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                        next_arm_patches,
                    )?;
                } else {
                    // Non-last: emit check that jumps to or_matched on
                    // success. We invert: collect a "fail" patch from the
                    // check, emit Jump(or_matched), then patch the fail
                    // to skip the jump (i.e., continue to the next branch).
                    let mut branch_fail: Vec<usize> = Vec::new();
                    compile_pattern_check(
                        branch,
                        scrutinee_slot,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                        &mut branch_fail,
                    )?;
                    // Branch matched if no JumpIfFalse was taken.
                    let matched_p = chunk.emit(Op::Jump(0), line);
                    or_matched_patches.push(matched_p);
                    // Patch branch_fail to here (next branch check).
                    let next_branch_pc = chunk.code.len();
                    for p in branch_fail {
                        chunk.patch_jump(p, next_branch_pc)?;
                    }
                }
            }
            // or_matched: all or_matched_patches land here.
            let or_matched_pc = chunk.code.len();
            for p in or_matched_patches {
                chunk.patch_jump(p, or_matched_pc)?;
            }
        }
        // RES-375: `None` — checks that scrutinee is an absent Option.
        Pattern::None => {
            let n = chunk.add_string_constant("is_none")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: n,
                    arity: 1,
                },
                line,
            );
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
        }
        // RES-375: `Some(inner)` — checks is_some, then extracts and matches inner.
        Pattern::Some(inner_pat) => {
            // 1. is_some(scrutinee) check.
            let is_some_n = chunk.add_string_constant("is_some")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: is_some_n,
                    arity: 1,
                },
                line,
            );
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            // 2. Extract inner: option_unwrap(scrutinee).
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let inner_slot = *next_local;
            *next_local += 1;
            let uw_n = chunk.add_string_constant("option_unwrap")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: uw_n,
                    arity: 1,
                },
                line,
            );
            chunk.emit(Op::StoreLocal(inner_slot), line);
            // 3. Check inner pattern.
            compile_pattern_check(
                inner_pat,
                inner_slot,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                next_arm_patches,
            )?;
        }
        // RES-923: `Ok(inner)` — checks is_ok, then extracts and matches inner.
        Pattern::Ok(inner_pat) => {
            let is_ok_n = chunk.add_string_constant("is_ok")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: is_ok_n,
                    arity: 1,
                },
                line,
            );
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let inner_slot = *next_local;
            *next_local += 1;
            let uw_n = chunk.add_string_constant("unwrap")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: uw_n,
                    arity: 1,
                },
                line,
            );
            chunk.emit(Op::StoreLocal(inner_slot), line);
            compile_pattern_check(
                inner_pat,
                inner_slot,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                next_arm_patches,
            )?;
        }
        // RES-923: `Err(inner)` — checks is_err, then extracts and matches inner.
        Pattern::Err(inner_pat) => {
            let is_err_n = chunk.add_string_constant("is_err")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: is_err_n,
                    arity: 1,
                },
                line,
            );
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let inner_slot = *next_local;
            *next_local += 1;
            let uwe_n = chunk.add_string_constant("unwrap_err")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: uwe_n,
                    arity: 1,
                },
                line,
            );
            chunk.emit(Op::StoreLocal(inner_slot), line);
            compile_pattern_check(
                inner_pat,
                inner_slot,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                next_arm_patches,
            )?;
        }
        // RES-932: `(p0, p1, ...)` — checks type, checks length, checks elements.
        Pattern::Tuple(sub_pats) => {
            // 1. Confirm the scrutinee is actually a Tuple (not an Array).
            let is_tup_n = chunk.add_string_constant("is_tuple")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: is_tup_n,
                    arity: 1,
                },
                line,
            );
            let p_type = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p_type);
            // 2. Check length via `len` builtin.
            let len_n = chunk.add_string_constant("len")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: len_n,
                    arity: 1,
                },
                line,
            );
            let expected_len = chunk.add_constant(Value::Int(sub_pats.len() as i64))?;
            chunk.emit(Op::Const(expected_len), line);
            chunk.emit(Op::Eq, line);
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            // Check each element.
            for (i, sub_pat) in sub_pats.iter().enumerate() {
                if *next_local == u16::MAX {
                    return Err(CompileError::TooManyLocals);
                }
                let elem_slot = *next_local;
                *next_local += 1;
                let i_idx = chunk.add_constant(Value::Int(i as i64))?;
                chunk.emit(Op::LoadLocal(scrutinee_slot), line);
                chunk.emit(Op::Const(i_idx), line);
                chunk.emit(Op::LoadIndex, line);
                chunk.emit(Op::StoreLocal(elem_slot), line);
                compile_pattern_check(
                    sub_pat,
                    elem_slot,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    next_arm_patches,
                )?;
            }
        }
        Pattern::Struct {
            struct_name,
            fields,
            ..
        } => {
            let sn_const = chunk.add_string_constant("struct_name")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: sn_const,
                    arity: 1,
                },
                line,
            );
            let expected = chunk.add_string_constant(struct_name)?;
            chunk.emit(Op::Const(expected), line);
            chunk.emit(Op::Eq, line);
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            for (fname, sub_pat) in fields {
                if *next_local == u16::MAX {
                    return Err(CompileError::TooManyLocals);
                }
                let field_slot = *next_local;
                *next_local += 1;
                let fname_idx = chunk.add_string_constant(fname)?;
                chunk.emit(Op::LoadLocal(scrutinee_slot), line);
                chunk.emit(
                    Op::GetField {
                        name_const: fname_idx,
                    },
                    line,
                );
                chunk.emit(Op::StoreLocal(field_slot), line);
                compile_pattern_check(
                    sub_pat,
                    field_slot,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    next_arm_patches,
                )?;
            }
        }
        Pattern::TupleStruct { name, fields } => {
            let sn_const = chunk.add_string_constant("struct_name")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: sn_const,
                    arity: 1,
                },
                line,
            );
            let expected = chunk.add_string_constant(name)?;
            chunk.emit(Op::Const(expected), line);
            chunk.emit(Op::Eq, line);
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            for (i, sub_pat) in fields.iter().enumerate() {
                if *next_local == u16::MAX {
                    return Err(CompileError::TooManyLocals);
                }
                let field_slot = *next_local;
                *next_local += 1;
                let fname_idx = chunk.add_string_constant(&i.to_string())?;
                chunk.emit(Op::LoadLocal(scrutinee_slot), line);
                chunk.emit(
                    Op::GetField {
                        name_const: fname_idx,
                    },
                    line,
                );
                chunk.emit(Op::StoreLocal(field_slot), line);
                compile_pattern_check(
                    sub_pat,
                    field_slot,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    next_arm_patches,
                )?;
            }
        }
        Pattern::EnumVariant {
            type_name,
            variant_name,
            payload,
        } => {
            // RES-3994: `Option::Some(v)` / `Option::None` /
            // `Result::Ok(v)` / `Result::Err(v)` written with the
            // explicit qualifier parse as a generic `Pattern::EnumVariant`
            // — only the bare `Some(v)` / `None` / `Ok(v)` / `Err(v)`
            // forms get the dedicated `Pattern::Some` / `Pattern::None`
            // / `Pattern::Ok` / `Pattern::Err` treatment (see
            // `parse_pattern_atom` in lib.rs). Their runtime values are
            // `Value::Option` / `Value::Result`, not `Value::EnumVariant`,
            // so the generic path below — which calls the `struct_name`
            // builtin, valid only for Struct/EnumVariant receivers —
            // always failed for them with "argument is not a struct or
            // enum variant" under `--vm`. Desugar to the dedicated
            // pattern and recurse, mirroring the interpreter's own
            // `Pattern::EnumVariant` bridge (lib.rs `match_pattern`).
            let bridged = match (type_name.as_deref(), variant_name.as_str()) {
                (Some("Option"), "Some") => Some(Pattern::Some(Box::new(
                    enum_pattern_payload_inner(payload).unwrap_or(Pattern::Wildcard),
                ))),
                (Some("Option"), "None") => Some(Pattern::None),
                (Some("Result"), "Ok") => Some(Pattern::Ok(Box::new(
                    enum_pattern_payload_inner(payload).unwrap_or(Pattern::Wildcard),
                ))),
                (Some("Result"), "Err") => Some(Pattern::Err(Box::new(
                    enum_pattern_payload_inner(payload).unwrap_or(Pattern::Wildcard),
                ))),
                _ => None,
            };
            if let Some(bridged_pat) = bridged {
                return compile_pattern_check(
                    &bridged_pat,
                    scrutinee_slot,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    next_arm_patches,
                );
            }
            let sn_const = chunk.add_string_constant("struct_name")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: sn_const,
                    arity: 1,
                },
                line,
            );
            let expected = chunk.add_string_constant(variant_name)?;
            chunk.emit(Op::Const(expected), line);
            chunk.emit(Op::Eq, line);
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            match payload {
                crate::EnumPatternPayload::None => {}
                crate::EnumPatternPayload::Named(fields) => {
                    for (fname, sub_pat) in fields {
                        if *next_local == u16::MAX {
                            return Err(CompileError::TooManyLocals);
                        }
                        let field_slot = *next_local;
                        *next_local += 1;
                        let fname_idx = chunk.add_string_constant(fname)?;
                        chunk.emit(Op::LoadLocal(scrutinee_slot), line);
                        chunk.emit(
                            Op::GetField {
                                name_const: fname_idx,
                            },
                            line,
                        );
                        chunk.emit(Op::StoreLocal(field_slot), line);
                        compile_pattern_check(
                            sub_pat,
                            field_slot,
                            chunk,
                            locals,
                            next_local,
                            fn_index,
                            ffi_index,
                            fns,
                            next_fn_idx,
                            line,
                            next_arm_patches,
                        )?;
                    }
                }
                crate::EnumPatternPayload::Tuple(sub_pats) => {
                    for (i, sub_pat) in sub_pats.iter().enumerate() {
                        if *next_local == u16::MAX {
                            return Err(CompileError::TooManyLocals);
                        }
                        let field_slot = *next_local;
                        *next_local += 1;
                        let fname_idx = chunk.add_string_constant(&i.to_string())?;
                        chunk.emit(Op::LoadLocal(scrutinee_slot), line);
                        chunk.emit(
                            Op::GetField {
                                name_const: fname_idx,
                            },
                            line,
                        );
                        chunk.emit(Op::StoreLocal(field_slot), line);
                        compile_pattern_check(
                            sub_pat,
                            field_slot,
                            chunk,
                            locals,
                            next_local,
                            fn_index,
                            ffi_index,
                            fns,
                            next_fn_idx,
                            line,
                            next_arm_patches,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Returns true if the pattern introduces any identifier bindings.
fn pattern_has_bindings(p: &crate::Pattern) -> bool {
    use crate::Pattern;
    match p {
        Pattern::Identifier(_) | Pattern::Bind(_, _) => true,
        Pattern::Or(branches) => branches.iter().any(pattern_has_bindings),
        Pattern::Wildcard | Pattern::Literal(_) | Pattern::Range { .. } | Pattern::None => false,
        Pattern::Struct { fields, .. } => fields.iter().any(|(_, p)| pattern_has_bindings(p)),
        Pattern::Tuple(ps) => ps.iter().any(pattern_has_bindings),
        Pattern::TupleStruct { fields, .. } => fields.iter().any(pattern_has_bindings),
        Pattern::Some(inner) | Pattern::Ok(inner) | Pattern::Err(inner) => {
            pattern_has_bindings(inner)
        }
        Pattern::EnumVariant { payload, .. } => match payload {
            crate::EnumPatternPayload::None => false,
            crate::EnumPatternPayload::Named(fields) => {
                fields.iter().any(|(_, p)| pattern_has_bindings(p))
            }
            crate::EnumPatternPayload::Tuple(ps) => ps.iter().any(pattern_has_bindings),
        },
    }
}

/// Static descriptor for a node kind, used in `Unsupported` errors.
/// RES-092: extract a 1-indexed source line from any Node variant
/// that carries a `Span`. Returns `None` for nodes whose span is
/// `Span::default()` (line 0 = synthetic) or for variants that
/// don't carry a span at all. Callers fall back to a parent-scope
/// line in those cases.
fn node_line(n: &Node) -> Option<u32> {
    let line: u32 = match n {
        // Statement variants (RES-079, RES-361).
        Node::LetStatement { span, .. }
        | Node::StaticLet { span, .. }
        | Node::Const { span, .. }
        | Node::Assignment { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::Break { span, .. }
        | Node::BreakWith { span, .. }
        | Node::Continue { span, .. }
        | Node::BreakLabel { span, .. }
        | Node::ContinueLabel { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::Slice { span, .. } => span.start.line as u32,

        // Block + ExpressionStatement (RES-087, tuple→struct).
        Node::Block { span, .. } | Node::ExpressionStatement { span, .. } => span.start.line as u32,

        // Leaves (RES-078).
        Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::StringInternLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::Identifier { span, .. } => span.start.line as u32,

        // Core expressions (RES-084) and index/field (RES-085).
        Node::PrefixExpression { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::IndexAssignment { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::FieldAssignment { span, .. } => span.start.line as u32,

        // Tuple-struct conversions (RES-086).
        Node::ArrayLiteral { span, .. }
        | Node::TryExpression { span, .. }
        | Node::OptionalChain { span, .. } => span.start.line as u32,

        // RES-148: map literal carries a span at its opening brace.
        Node::MapLiteral { span, .. } => span.start.line as u32,

        // RES-149: set literal span at its opening `#{`.
        Node::SetLiteral { span, .. } => span.start.line as u32,

        // RES-152: bytes literal span at its opening `b"`.
        Node::BytesLiteral { span, .. } => span.start.line as u32,

        // RES-2619: char literal span at its opening `'`.
        Node::CharLiteral { span, .. } => span.start.line as u32,

        // RES-155: struct destructure let carries the `let` keyword span.
        Node::LetDestructureStruct { span, .. } => span.start.line as u32,

        // Structural variants (RES-088).
        Node::Function { span, .. }
        | Node::Use { span, .. }
        | Node::Extern { span, .. }
        | Node::LiveBlock { span, .. }
        | Node::Assert { span, .. }
        | Node::Assume { span, .. }
        | Node::InvariantStatement { span, .. }
        | Node::Match { span, .. }
        | Node::StructDecl { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::ImplBlock { span, .. }
        | Node::TypeAlias { span, .. }
        | Node::RegionDecl { span, .. }
        | Node::Actor { span, .. }
        | Node::ActorDecl { span, .. }
        | Node::ClusterDecl { span, .. }
        | Node::FunctionLiteral { span, .. }
        | Node::TryCatch { span, .. }
        | Node::Quantifier { span, .. }
        | Node::SupervisorDecl { span, .. } => span.start.line as u32,

        // RES-291: integer range expression. Only emitted from the
        // tree-walker frontend today; bytecode lowering treats it as
        // unsupported.
        Node::Range { span, .. } => span.start.line as u32,

        // RES-142: duration literal carries the span of its integer
        // part; only emitted inside live-clause position so it
        // shouldn't round-trip through the compiler, but match it
        // anyway to keep the pattern exhaustive.
        Node::DurationLiteral { span, .. } => span.start.line as u32,

        // Program is wrapped in Spanned<Node> at the call site, not
        // inside the Node enum itself.
        Node::Program(_) => 0,

        // RES-325: NamedArg carries the span of its `name:` label.
        Node::NamedArg { span, .. } => span.start.line as u32,
        // RES-221: interpolated string carries the opening quote's span.
        Node::InterpolatedString { span, .. } => span.start.line as u32,

        // RES-324: module declaration; span at the `mod` keyword.
        Node::ModuleDecl { span, .. } => span.start.line as u32,

        // RES-319: newtype nodes carry a span.
        Node::NewtypeDecl { span, .. } => span.start.line as u32,
        Node::NewtypeConstruct { span, .. } => span.start.line as u32,
        // RES-401: tuples carry their own spans.
        Node::TupleLiteral { span, .. } => span.start.line as u32,
        Node::TupleIndex { span, .. } => span.start.line as u32,
        Node::LetTupleDestructure { span, .. } => span.start.line as u32,
        // RES-290: trait declarations carry a span.
        Node::TraitDecl { span, .. } => span.start.line as u32,
        // RES-400 PR 1: enum declarations carry a span.
        Node::EnumDecl { span, .. } => span.start.line as u32,
        // RES-406: unsafe block carries the keyword's span.
        Node::UnsafeBlock { span, .. } => span.start.line as u32,
        // RES-395: region type-param node — carries its declaration span.
        Node::RegionParam { span, .. } => span.start.line as u32,
        // RES-2552: blanket impl declaration — use its declaration span.
        // RES-2552: blanket impl — carries its declaration span.
        Node::BlanketImpl { span, .. } => span.start.line as u32,
        // RES-2660: static_assert — carries the keyword's span.
        Node::StaticAssert { span, .. } => span.start.line as u32,
        // RES-2579: defer statement — carries the keyword's span.
        Node::DeferStatement { span, .. } => span.start.line as u32,
        // RES-2613: bench block — carries the keyword's span.
        Node::BenchBlock { span, .. } => span.start.line as u32,
    };
    if line == 0 { None } else { Some(line) }
}

fn node_kind(n: &Node) -> &'static str {
    match n {
        Node::Program(_) => "Program",
        Node::Use { .. } => "Use",
        Node::Function { .. } => "Function",
        Node::LiveBlock { .. } => "LiveBlock",
        Node::Assert { .. } => "Assert",
        Node::Assume { .. } => "Assume",
        Node::InvariantStatement { .. } => "InvariantStatement",
        Node::Block { .. } => "Block",
        Node::LetStatement { .. } => "LetStatement",
        Node::StaticLet { .. } => "StaticLet",
        Node::Const { .. } => "Const",
        Node::Assignment { .. } => "Assignment",
        Node::ReturnStatement { .. } => "ReturnStatement",
        Node::IfStatement { .. } => "IfStatement",
        Node::WhileStatement { .. } => "WhileStatement",
        Node::ForInStatement { .. } => "ForInStatement",
        Node::ExpressionStatement { .. } => "ExpressionStatement",
        Node::Identifier { .. } => "Identifier",
        Node::IntegerLiteral { .. } => "IntegerLiteral",
        Node::FloatLiteral { .. } => "FloatLiteral",
        Node::StringLiteral { .. } => "StringLiteral",
        Node::StringInternLiteral { .. } => "StringInternLiteral",
        Node::BooleanLiteral { .. } => "BooleanLiteral",
        Node::PrefixExpression { .. } => "PrefixExpression",
        Node::InfixExpression { .. } => "InfixExpression",
        Node::CallExpression { .. } => "CallExpression",
        Node::ArrayLiteral { .. } => "ArrayLiteral",
        Node::IndexExpression { .. } => "IndexExpression",
        Node::IndexAssignment { .. } => "IndexAssignment",
        Node::RegionDecl { .. } => "RegionDecl",
        Node::StructLiteral { .. } => "StructLiteral",
        Node::FieldAccess { .. } => "FieldAccess",
        Node::FieldAssignment { .. } => "FieldAssignment",
        Node::BytesLiteral { .. } => "BytesLiteral",
        Node::CharLiteral { .. } => "CharLiteral",
        Node::Range { .. } => "Range",
        Node::Slice { .. } => "Slice",
        Node::LetTupleDestructure { .. } => "LetTupleDestructure",
        Node::LetDestructureStruct { .. } => "LetDestructureStruct",
        Node::TupleLiteral { .. } => "TupleLiteral",
        Node::TupleIndex { .. } => "TupleIndex",
        Node::MapLiteral { .. } => "MapLiteral",
        Node::SetLiteral { .. } => "SetLiteral",
        Node::Match { .. } => "Match",
        Node::FunctionLiteral { .. } => "FunctionLiteral",
        Node::InterpolatedString { .. } => "InterpolatedString",
        _ => "<other>",
    }
}

// ============================================================
// RES-384: tail-call rewriting pass
// ============================================================

/// Scan `chunk.code` for every adjacent `Call(fn_idx); ReturnFromCall`
/// pair where `fn_idx == own_fn_idx` (self-recursion, RES-384) or
/// `fn_idx` is in `mutual_targets` (RES-4017: another function in the
/// same `#[mutual_tail_call]` group) and replace the pair with a
/// single `TailCall(fn_idx)`. The removed `ReturnFromCall` leaves a
/// hole; rather than shifting the Vec (which would invalidate all
/// existing jump targets), we overwrite the second slot of each pair
/// with a `Jump(0)` sentinel pointing one step back so the dead op
/// can never be reached:
///
/// ```text
/// before:  [..., Call(i), ReturnFromCall, ...]
/// after:   [..., TailCall(i), (dead/unreachable), ...]
/// ```
///
/// Because `TailCall` does not fall through (it loops back to pc=0),
/// the instruction following it is dead. We leave it as a `Return`
/// no-op rather than a `Jump` to avoid confusing the disassembler;
/// the VM will never execute it.
///
/// Jump targets are NOT shifted — this transform only touches pairs
/// where the second op is `ReturnFromCall`, which nothing ever jumps
/// TO (no other op emits a forward-jump into `ReturnFromCall`; all
/// branch targets land on the instruction AFTER a block, not ON a
/// return). This invariant holds for the patterns the compiler emits.
///
/// `mutual_targets` is the set of fn-table indices this function may
/// tail-call into as part of a `#[mutual_tail_call]` group (empty
/// when the function isn't annotated — see
/// `mutual_tco::mutual_tail_call_indices`). `Call(idx); ReturnFromCall`
/// only ever arises when the call's result flows straight into the
/// return — i.e. genuine tail position — regardless of which `if`/
/// `match` arm it came from, so no separate AST-level tail analysis
/// is needed here.
fn rewrite_tail_calls(
    chunk: &mut crate::bytecode::Chunk,
    own_fn_idx: u16,
    mutual_targets: &std::collections::HashSet<u16>,
) {
    let len = chunk.code.len();
    if len < 2 {
        return;
    }
    // RES-1581: fuse the two-pass collect-then-rewrite into a single
    // walk. The intermediate `Vec<usize>` of positions was unnecessary
    // — rewriting in place doesn't break the next iteration's read
    // because the new `Op::Return` tombstone at `i+1` cannot itself
    // match a tail-call-eligible `Op::Call` at any later i. Drops the
    // Vec allocation and halves the linear scans.
    for i in 0..len - 1 {
        if let Op::Call(target) = chunk.code[i]
            && chunk.code[i + 1] == Op::ReturnFromCall
            && (target == own_fn_idx || mutual_targets.contains(&target))
        {
            // Replace the Call with TailCall; mark the ReturnFromCall
            // dead by overwriting with a no-op Return. The VM never
            // reaches it because TailCall resets pc, but leaving a
            // valid opcode keeps the chunk well-formed for the
            // disassembler and any future static analyses.
            chunk.code[i] = Op::TailCall(target);
            chunk.code[i + 1] = Op::Return; // unreachable tombstone
        }
    }
}

// ============================================================
// RES-170a: struct registry
// ============================================================
//
// The VM's eventual struct-ops lowering (RES-170c) needs to
// answer two questions at compile time without the runtime ever
// touching string names:
//
//   - "What `type_id` should `Op::MakeStruct` carry for this
//     struct literal?"
//   - "What `u8` field index corresponds to `p.x`?"
//
// This module builds the registry that answers both. Each
// `Node::StructDecl` in the program gets a unique `type_id`
// (assigned in source order so the indices are stable across
// compile invocations), and each field gets a `u8` slot index
// matching its declaration order. RES-170b will walk the AST
// threading local → struct-name info; RES-170c will consume the
// registry to emit MakeStruct / LoadField / StoreField.
//
// ## Why not reuse the JIT's RES-165a StructLayout?
//
// Different data. RES-165a computes byte offsets + cranelift
// `Type`s for the JIT's stack-allocated repr(C) layout. The VM
// uses a heap-allocated `Vec<Value>` indexed by field position —
// no byte offsets, no per-field types (each slot is a `Value`).
// The field-name-to-index map is the only shared piece, and
// each backend derives its own copy from the same
// `Node::StructDecl`. When cross-module type-id uniqueness
// lands (RES-170d), we may pull the registry into a common
// module and surface it to both backends; for today a
// compiler-local definition is simpler.

/// RES-170a: per-struct entry in the registry. `name` duplicates
/// the map key so callers can use an `&StructRegistryEntry` on
/// its own without lugging around the key. `fields` is sorted by
/// declaration position, so `fields[i]` is the name at slot `i`
/// and `field_index(name) -> Some(i as u8)` does the inverse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructRegistryEntry {
    /// Declared struct name (e.g. `"Point"`).
    pub name: String,
    /// Compile-time identifier for the struct. Unique within a
    /// `StructRegistry` build; assignment order matches the
    /// source order the decl appeared in the `Program`.
    pub type_id: u16,
    /// Field names in declaration order. Slot index inside a
    /// `Value::Struct { fields, .. }` matches this vector's
    /// indexing, so `LoadField { idx }` reads `fields[idx]`.
    pub fields: Vec<String>,
}

impl StructRegistryEntry {
    /// Return the `u8` slot index for `field_name`, or `None` if
    /// the struct has no such field. Linear scan — struct field
    /// counts are small and this is a compile-time lookup, not a
    /// per-instruction hot path.
    pub fn field_index(&self, field_name: &str) -> Option<u8> {
        self.fields
            .iter()
            .position(|f| f == field_name)
            .map(|i| i as u8)
    }
}

/// RES-170a: compile-time registry of every `Node::StructDecl`
/// in a `Program`. Built by `StructRegistry::from_program`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructRegistry {
    /// Keyed by declared name; each entry carries its `type_id`
    /// and field vector.
    entries: HashMap<String, StructRegistryEntry>,
}

impl StructRegistry {
    /// Walk every top-level `Node::StructDecl` in `program` and
    /// build a registry. Errors:
    ///
    ///   - `DuplicateStructName(name)` — two decls share `name`.
    ///   - `TooManyStructDecls`        — more than u16::MAX + 1 decls.
    ///   - `TooManyFields(name)`       — one decl has more than
    ///     u8::MAX + 1 fields (RES-170c's `LoadField { idx: u8 }`
    ///     is the hard cap).
    ///
    /// Nested declarations (inside `ImplBlock`s or other
    /// containers) are ignored for today; the parser only places
    /// `StructDecl`s at `Program` scope.
    pub fn from_program(program: &Node) -> Result<Self, CompileError> {
        let stmts = match program {
            Node::Program(s) => s,
            _ => {
                return Err(CompileError::Unsupported(
                    "struct registry requires a Program root",
                ));
            }
        };
        // RES-1579: pre-size `entries` to the StructDecl count. Same
        // shape as RES-1461 (fn_index), RES-1575 (locals), RES-1577
        // (ffi_index) — counting once is cheap, avoids the default
        // rehash chain as the registry grows.
        let struct_count = stmts
            .iter()
            .filter(|s| matches!(&s.node, Node::StructDecl { .. }))
            .count();
        let mut entries: HashMap<String, StructRegistryEntry> =
            HashMap::with_capacity(struct_count);
        let mut next_type_id: u32 = 0;
        for spanned in stmts {
            let Node::StructDecl { name, fields, .. } = &spanned.node else {
                continue;
            };
            if entries.contains_key(name) {
                return Err(CompileError::DuplicateStructName(name.clone()));
            }
            if fields.len() > u8::MAX as usize + 1 {
                return Err(CompileError::TooManyFields(name.clone()));
            }
            if next_type_id > u16::MAX as u32 {
                return Err(CompileError::TooManyStructDecls);
            }
            let field_names: Vec<String> =
                fields.iter().map(|(_ty, fname)| fname.clone()).collect();
            entries.insert(
                name.clone(),
                StructRegistryEntry {
                    name: name.clone(),
                    type_id: next_type_id as u16,
                    fields: field_names,
                },
            );
            next_type_id += 1;
        }
        Ok(Self { entries })
    }

    /// Number of registered struct decls.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if no struct decls were registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up a struct by name. Returns `None` if the program
    /// has no matching decl.
    pub fn get(&self, name: &str) -> Option<&StructRegistryEntry> {
        self.entries.get(name)
    }

    /// Convenience: resolve `(struct_name, field_name)` to the
    /// `(type_id, field_index)` pair RES-170c will encode into
    /// `MakeStruct` / `LoadField` operands. Returns `None` when
    /// the struct or the field doesn't exist.
    pub fn resolve(&self, struct_name: &str, field_name: &str) -> Option<(u16, u8)> {
        let entry = self.entries.get(struct_name)?;
        let idx = entry.field_index(field_name)?;
        Some((entry.type_id, idx))
    }
}

#[cfg(test)]
pub(crate) fn parse_and_compile(src: &str) -> Result<Program, String> {
    let (ast, errs) = crate::parse(src);
    if !errs.is_empty() {
        return Err(errs.join("; "));
    }
    compile(&ast).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Op;

    fn parse_one(src: &str) -> Node {
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        program
    }

    #[cfg(feature = "ffi")]
    #[test]
    fn extern_block_produces_foreign_sym_in_program() {
        let src = "fn main() { return 1; }\n";
        let prog = crate::compiler::parse_and_compile(src).expect("compiles");
        assert!(prog.foreign_syms.is_empty());
    }

    #[test]
    fn compile_int_literal_emits_const() {
        let p = parse_one("42;");
        let prog = compile(&p).unwrap();
        assert_eq!(prog.main.constants.len(), 1);
        assert!(matches!(prog.main.constants[0], Value::Int(42)));
        assert_eq!(prog.main.code.first(), Some(&Op::Const(0)));
        assert!(matches!(prog.main.code.last(), Some(Op::Return)));
        assert!(prog.functions.is_empty());
    }

    #[test]
    fn res3916_bare_zero_arg_enum_variant_ref_compiles_across_fn_boundary() {
        // Before RES-3916 this raised `UnknownIdentifier("E::A")`: the
        // variant was only registered as a local in the enum's declaring
        // (top-level) scope, invisible to `main`'s separate locals map.
        let p = parse_one("enum E { A, B }\nfn main() -> int { let x = E::A; return 0; }\nmain();");
        let prog = compile(&p).expect("bare zero-arg enum variant ref must compile");
        // The `main` function's chunk must contain a Const referencing the
        // `E::A` EnumVariant value.
        let main_fn = prog
            .functions
            .iter()
            .find(|f| f.name == "main")
            .expect("main fn present");
        let has_variant_const = main_fn.chunk.constants.iter().any(|c| {
            matches!(
                c,
                Value::EnumVariant { type_name, variant, .. }
                    if type_name == "E" && variant == "A"
            )
        });
        assert!(
            has_variant_const,
            "main chunk should intern the E::A variant constant: {:?}",
            main_fn.chunk.constants
        );
    }

    #[test]
    fn res3916_unknown_qualified_name_still_errors() {
        // A `::`-qualified name that isn't a known zero-arg variant must
        // still raise UnknownIdentifier — the fix must not swallow real
        // resolution failures.
        let p = parse_one("fn main() -> int { let x = NotAnEnum::Nope; return 0; }\nmain();");
        let err = compile(&p).expect_err("unknown qualified name must still error");
        assert!(
            matches!(err, CompileError::UnknownIdentifier(ref n) if n == "NotAnEnum::Nope"),
            "expected UnknownIdentifier, got {err:?}"
        );
    }

    #[test]
    fn compile_arith_respects_precedence() {
        let p = parse_one("2 + 3 * 4;");
        let prog = compile(&p).unwrap();
        let body: Vec<&Op> = prog
            .main
            .code
            .iter()
            .filter(|op| !matches!(op, Op::Return))
            .collect();
        assert_eq!(body.len(), 5, "got {:?}", body);
        assert!(matches!(body[3], Op::Mul));
        assert!(matches!(body[4], Op::Add));
    }

    #[test]
    fn compile_let_emits_store_local() {
        let p = parse_one("let x = 7;");
        let prog = compile(&p).unwrap();
        assert!(
            prog.main
                .code
                .iter()
                .any(|op| matches!(op, Op::StoreLocal(0)))
        );
    }

    #[test]
    fn compile_unknown_identifier_errors_cleanly() {
        let p = parse_one("y;");
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::UnknownIdentifier(_)));
    }

    #[test]
    fn compile_struct_match_pattern_compiles() {
        // RES-2540: struct match patterns now compile successfully.
        let p = parse_one(
            r#"struct Point { int x, int y }
            fn classify(Point p) -> int {
                return match p {
                    Point { x: 0, y: 0 } => 1,
                    _ => 0,
                };
            }"#,
        );
        assert!(compile(&p).is_ok());
    }

    // ---------- RES-334: for-in lowering ----------

    /// `for x in arr { ... }` no longer reports `Unsupported`. The
    /// chunk should compile cleanly and the loop variable's slot
    /// should be readable inside the body.
    #[test]
    fn res334_for_in_array_compiles() {
        let p = parse_one(
            r#"
                let arr = [1, 2, 3];
                let total = 0;
                for x in arr {
                    total = total + x;
                }
            "#,
        );
        let prog = compile(&p).expect("for-in must compile");
        // Loop body must read the loop variable: `LoadIndex` produces
        // it and `StoreLocal` commits it; then a `LoadLocal` of that
        // same slot must follow inside the body.
        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::LoadIndex)),
            "expected LoadIndex in for-in body: {:?}",
            prog.main.code
        );
    }

    /// The lowered shape includes a `len` builtin call to compute
    /// the iteration bound. Verify the constant pool carries the
    /// builtin name and the chunk emits `CallBuiltin`.
    #[test]
    fn res334_for_in_uses_len_builtin() {
        let p = parse_one(
            r#"
                let arr = [10, 20];
                for x in arr { let y = x; }
            "#,
        );
        let prog = compile(&p).expect("for-in compiles");
        let mut saw_len = false;
        for op in &prog.main.code {
            if let Op::CallBuiltin { name_const, arity } = op {
                let s = match prog.main.constants.get(*name_const as usize) {
                    Some(Value::String(s)) => s.clone(),
                    _ => continue,
                };
                if s == "len" {
                    assert_eq!(*arity, 1, "len call must have arity 1");
                    saw_len = true;
                }
            }
        }
        assert!(
            saw_len,
            "expected a CallBuiltin(len, 1) for the iteration bound"
        );
    }

    /// for-in must include a back-edge `Jump` to the loop test and a
    /// forward `JumpIfFalse` exiting the loop, mirroring `while`.
    #[test]
    fn res334_for_in_emits_back_edge_and_exit_jump() {
        let p = parse_one(
            r#"
                let arr = [1];
                for x in arr { let y = x; }
            "#,
        );
        let prog = compile(&p).expect("for-in compiles");
        let has_back_edge = prog
            .main
            .code
            .iter()
            .any(|op| matches!(op, Op::Jump(off) if *off < 0));
        let has_exit = prog
            .main
            .code
            .iter()
            .any(|op| matches!(op, Op::JumpIfFalse(off) if *off > 0));
        assert!(
            has_back_edge,
            "expected a negative-offset Jump (back-edge): {:?}",
            prog.main.code
        );
        assert!(
            has_exit,
            "expected a positive-offset JumpIfFalse (exit): {:?}",
            prog.main.code
        );
    }

    /// for-in inside a function body must compile through the
    /// `compile_stmt_in_fn` dispatcher so a `return` in the body
    /// emits `ReturnFromCall`, not `Return`.
    #[test]
    fn res334_for_in_in_fn_body_compiles_with_return_from_call() {
        let p = parse_one(
            r#"
                fn first(int dummy) -> int {
                    let xs = [1, 2, 3];
                    for x in xs {
                        return x;
                    }
                    return -1;
                }
            "#,
        );
        let prog = compile(&p).expect("for-in inside fn compiles");
        let f = &prog.functions[0];
        assert!(
            f.chunk
                .code
                .iter()
                .any(|op| matches!(op, Op::ReturnFromCall)),
            "expected ReturnFromCall inside fn body: {:?}",
            f.chunk.code
        );
        // No bare `Op::Return` should appear in a fn body.
        assert!(
            !f.chunk.code.iter().any(|op| matches!(op, Op::Return)),
            "fn body must not emit Op::Return (halts VM); got {:?}",
            f.chunk.code
        );
    }

    /// Nested for-in must allocate non-overlapping iteration-state
    /// slots so the outer loop's index isn't clobbered by the
    /// inner loop.
    #[test]
    fn res334_nested_for_in_compiles() {
        let p = parse_one(
            r#"
                let outer = [[1, 2], [3]];
                let total = 0;
                for row in outer {
                    for x in row {
                        total = total + x;
                    }
                }
            "#,
        );
        let prog = compile(&p).expect("nested for-in compiles");
        // Two distinct StoreLocal targets must be initialised to 0
        // (the inner and outer index slots). The pattern looks for
        // `Const(<int 0>); StoreLocal(s)` pairs.
        let mut zero_init_slots: Vec<u16> = Vec::new();
        let mut prev: Option<&Op> = None;
        for op in &prog.main.code {
            if let Some(Op::Const(c)) = prev
                && let Op::StoreLocal(slot) = op
                && matches!(prog.main.constants.get(*c as usize), Some(Value::Int(0)))
            {
                zero_init_slots.push(*slot);
            }
            prev = Some(op);
        }
        assert!(
            zero_init_slots.len() >= 2,
            "expected at least two zero-initialised index slots in nested for-in: got {:?}",
            zero_init_slots
        );
    }

    // ---------- RES-334b: string + range iteration ----------

    #[test]
    fn res334b_for_in_string_compiles() {
        // `for c in "hi"` must compile without errors.
        let p = parse_one(r#"let s = "hi"; let n = 0; for c in s { n = n + 1; } return n;"#);
        compile(&p).expect("for-in over string compiles");
    }

    #[test]
    fn res334b_for_in_range_compiles() {
        // `for i in 0..3` must compile — the range is lowered to
        // `__range(0, 3, false)` by compile_expr (RES-4000), then
        // `IterPrepare` materializes it into an array for the loop.
        let p = parse_one("let n = 0; for i in 0..3 { n = n + i; } return n;");
        compile(&p).expect("for-in over range compiles");
    }

    // ---------- RES-081 tests ----------

    #[test]
    fn compile_fn_decl_populates_functions_table() {
        let p = parse_one("fn zero() { return 0; }");
        let prog = compile(&p).unwrap();
        assert_eq!(prog.functions.len(), 1);
        assert_eq!(prog.functions[0].name, "zero");
        assert_eq!(prog.functions[0].arity, 0);
    }

    #[test]
    fn compile_call_emits_call_op() {
        let p = parse_one("fn zero() { return 0; } zero();");
        let prog = compile(&p).unwrap();
        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::Call(0))),
            "expected Call(0) in main.code: {:?}",
            prog.main.code
        );
    }

    #[test]
    fn compile_unknown_function_call_errors() {
        let p = parse_one("nope();");
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::UnknownFunction(_)), "{:?}", err);
    }

    /// RES-VM (issue #266): `println("hi")` lowers to a `CallBuiltin`
    /// op (not `Call`, which is for user functions). The constant pool
    /// holds the builtin's name as a `Value::String`; arity is the
    /// argument count.
    #[test]
    fn compile_println_emits_call_builtin() {
        let p = parse_one("println(\"hi\");");
        let prog = compile(&p).unwrap();
        // Find the CallBuiltin op and verify its constant resolves
        // to the builtin name.
        let mut found = false;
        for op in &prog.main.code {
            if let Op::CallBuiltin { name_const, arity } = op {
                let name = match prog.main.constants.get(*name_const as usize) {
                    Some(Value::String(s)) => s.clone(),
                    other => panic!("expected Value::String at name_const, got {:?}", other),
                };
                assert_eq!(name, "println");
                assert_eq!(*arity, 1);
                found = true;
            }
        }
        assert!(
            found,
            "expected a CallBuiltin op in main.code: {:?}",
            prog.main.code
        );
    }

    /// RES-VM (issue #266): a user-defined function with the same
    /// name as a builtin shadows the builtin. Compile path picks the
    /// user fn (Call), not CallBuiltin — mirrors the tree walker's
    /// lookup order where the user binding wins.
    #[test]
    fn compile_user_fn_shadows_builtin() {
        let p = parse_one("fn println() { return 1; } println();");
        let prog = compile(&p).unwrap();
        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::Call(_))),
            "expected Call (user fn) in main.code: {:?}",
            prog.main.code
        );
        assert!(
            !prog
                .main
                .code
                .iter()
                .any(|op| matches!(op, Op::CallBuiltin { .. })),
            "user fn must shadow builtin; got: {:?}",
            prog.main.code
        );
    }

    #[test]
    fn compile_fn_with_params_maps_them_to_first_locals() {
        let p = parse_one("fn sq(int n) { return n * n; }");
        let prog = compile(&p).unwrap();
        let f = &prog.functions[0];
        assert_eq!(f.arity, 1);
        // Inside the body, `n` is local 0. The emitted code should
        // LoadLocal(0) twice before Mul.
        let load_count = f
            .chunk
            .code
            .iter()
            .filter(|op| matches!(op, Op::LoadLocal(0)))
            .count();
        assert_eq!(
            load_count, 2,
            "expected two LoadLocal(0) for n*n: {:?}",
            f.chunk.code
        );
    }

    #[test]
    fn compile_too_many_params_errors() {
        // 256 params — over the u8 limit.
        let params: Vec<String> = (0..256).map(|i| format!("int p{}", i)).collect();
        let src = format!("fn big({}) {{ return 0; }}", params.join(", "));
        let p = parse_one(&src);
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::Unsupported(_)), "{:?}", err);
    }

    // ---------- RES-170a: struct registry ----------

    #[test]
    fn res170a_empty_program_has_empty_registry() {
        let p = parse_one("return 1;");
        let reg = StructRegistry::from_program(&p).unwrap();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn res170a_single_struct_registers_with_type_id_zero() {
        let p = parse_one(
            r#"
            struct Point {
                int x,
                int y,
            }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        let pt = reg.get("Point").expect("Point should be registered");
        assert_eq!(pt.name, "Point");
        assert_eq!(pt.type_id, 0);
        assert_eq!(pt.fields, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn res170a_field_names_preserve_declaration_order() {
        let p = parse_one(
            r#"
            struct Rec {
                int c,
                int a,
                int b,
            }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        let r = reg.get("Rec").unwrap();
        // Source order is c, a, b — NOT alphabetical.
        assert_eq!(
            r.fields,
            vec!["c".to_string(), "a".to_string(), "b".to_string()]
        );
        assert_eq!(r.field_index("c"), Some(0));
        assert_eq!(r.field_index("a"), Some(1));
        assert_eq!(r.field_index("b"), Some(2));
    }

    #[test]
    fn res170a_field_index_missing_returns_none() {
        let p = parse_one(r#"struct S { int x, }"#);
        let reg = StructRegistry::from_program(&p).unwrap();
        let s = reg.get("S").unwrap();
        assert_eq!(s.field_index("x"), Some(0));
        assert!(s.field_index("nope").is_none());
    }

    #[test]
    fn res170a_multiple_structs_get_sequential_type_ids() {
        let p = parse_one(
            r#"
            struct A { int x, }
            struct B { int y, }
            struct C { int z, }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        assert_eq!(reg.get("A").unwrap().type_id, 0);
        assert_eq!(reg.get("B").unwrap().type_id, 1);
        assert_eq!(reg.get("C").unwrap().type_id, 2);
        assert_eq!(reg.len(), 3);
    }

    #[test]
    fn res170a_duplicate_struct_name_errors() {
        let p = parse_one(
            r#"
            struct Dup { int x, }
            struct Dup { int y, }
        "#,
        );
        let err = StructRegistry::from_program(&p).unwrap_err();
        match err {
            CompileError::DuplicateStructName(n) => assert_eq!(n, "Dup"),
            other => panic!("expected DuplicateStructName, got {:?}", other),
        }
    }

    #[test]
    fn res170a_unknown_struct_lookup_is_none() {
        let p = parse_one(r#"struct P { int x, }"#);
        let reg = StructRegistry::from_program(&p).unwrap();
        assert!(reg.get("Q").is_none());
    }

    #[test]
    fn res170a_resolve_roundtrips_type_id_and_field_index() {
        let p = parse_one(
            r#"
            struct First  { int a, }
            struct Second { int x, bool y, int z, }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        assert_eq!(reg.resolve("First", "a"), Some((0, 0)));
        assert_eq!(reg.resolve("Second", "x"), Some((1, 0)));
        assert_eq!(reg.resolve("Second", "y"), Some((1, 1)));
        assert_eq!(reg.resolve("Second", "z"), Some((1, 2)));
        // Unknown struct / unknown field → None.
        assert!(reg.resolve("Nope", "a").is_none());
        assert!(reg.resolve("Second", "nope").is_none());
    }

    #[test]
    fn res170a_registry_coexists_with_let_and_fn_decls() {
        // Realistic program: mixed struct / fn / let statements at
        // top level. The registry must pick up only the structs.
        let p = parse_one(
            r#"
            let start = 0;
            struct P { int x, int y, }
            fn add(int a, int b) -> int { return a + b; }
            struct Q { bool flag, }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.get("P").unwrap().type_id, 0);
        assert_eq!(reg.get("Q").unwrap().type_id, 1);
    }

    #[test]
    fn res170a_empty_struct_gets_empty_field_vec() {
        let p = parse_one(r#"struct Empty { }"#);
        let reg = StructRegistry::from_program(&p).unwrap();
        let e = reg.get("Empty").unwrap();
        assert!(e.fields.is_empty());
        assert!(e.field_index("anything").is_none());
    }

    #[test]
    fn res170a_non_program_root_errors() {
        // The registry requires a Program root — fed a bare node,
        // it should reject rather than silently produce an empty
        // registry.
        let just_int = Node::IntegerLiteral {
            value: 42,
            span: crate::span::Span::default(),
        };
        let err = StructRegistry::from_program(&just_int).unwrap_err();
        assert!(matches!(err, CompileError::Unsupported(_)), "got {:?}", err);
    }

    // ---------- RES-407: bounds-check elision ----------

    use std::sync::Mutex;
    static BOUNDS_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Walk every chunk in `prog` (main + all user fns) and count
    /// occurrences of `LoadIndex` and `LoadIndexUnchecked`.
    fn count_load_index_ops(prog: &Program) -> (usize, usize) {
        let chunks = std::iter::once(&prog.main).chain(prog.functions.iter().map(|f| &f.chunk));
        let mut checked = 0usize;
        let mut unchecked = 0usize;
        for c in chunks {
            for op in &c.code {
                match op {
                    Op::LoadIndex => checked += 1,
                    Op::LoadIndexUnchecked => unchecked += 1,
                    _ => {}
                }
            }
        }
        (checked, unchecked)
    }

    #[test]
    fn res407_proven_literal_index_emits_unchecked_load() {
        // `lock()` may poison if a sibling test panicked; recover the
        // guard so this test doesn't transitively fail.
        let _g = BOUNDS_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let src = r#"
fn main() {
    let xs = [10, 20, 30];
    let y = xs[1];
}
main();
"#;
        let program = parse_one(src);
        // Pass needs to run before compile so the proven-sites set is
        // populated. The compiler reads it via thread-local.
        crate::bounds_check::check_array_bounds(&program, "<test>").unwrap();
        let prog = compile(&program).expect("compiles");
        let (checked, unchecked) = count_load_index_ops(&prog);
        assert_eq!(
            unchecked, 1,
            "expected one LoadIndexUnchecked for proven xs[1] (checked={})",
            checked
        );
        assert_eq!(
            checked, 0,
            "expected no checked LoadIndex (unchecked={})",
            unchecked
        );
    }

    #[test]
    fn res407_unprovable_index_keeps_checked_load() {
        let _g = BOUNDS_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // `i` is a free parameter — bounds_check can't prove it.
        let src = r#"
fn get(int i) -> int {
    let xs = [1, 2, 3];
    return xs[i];
}
"#;
        let program = parse_one(src);
        crate::bounds_check::check_array_bounds(&program, "<test>").unwrap();
        let prog = compile(&program).expect("compiles");
        let (checked, unchecked) = count_load_index_ops(&prog);
        assert_eq!(
            unchecked, 0,
            "expected no LoadIndexUnchecked for dynamic xs[i] (checked={})",
            checked
        );
        assert!(
            checked >= 1,
            "expected at least one checked LoadIndex for dynamic xs[i]"
        );
    }

    // ── RES-break-continue: break / continue / assert compilation ──

    fn vm_run(src: &str) -> crate::vm::VmError {
        let prog = parse_one(src);
        match compile(&prog) {
            Err(e) => panic!("compile error: {:?}", e),
            Ok(p) => match crate::vm::run(&p) {
                Ok(v) => panic!("expected error, got {:?}", v),
                Err(e) => e,
            },
        }
    }

    fn vm_ok(src: &str) -> Value {
        let prog = parse_one(src);
        let p = compile(&prog).expect("compiles");
        crate::vm::run(&p).expect("runs")
    }

    #[test]
    fn break_exits_while_loop() {
        // Loop would run forever without break; it exits after 3 iterations.
        let src = r#"
let i = 0;
while true {
    i = i + 1;
    if i == 3 {
        break;
    }
}
i;
"#;
        match vm_ok(src) {
            Value::Int(3) => {}
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    #[test]
    fn continue_skips_body_tail_in_while_loop() {
        // Accumulate only even numbers 0..10.
        let src = r#"
let i = 0;
let sum = 0;
while i < 10 {
    i = i + 1;
    if i % 2 != 0 {
        continue;
    }
    sum = sum + i;
}
sum;
"#;
        // Even numbers 2+4+6+8+10 = 30
        match vm_ok(src) {
            Value::Int(30) => {}
            other => panic!("expected Int(30), got {:?}", other),
        }
    }

    #[test]
    fn break_in_fn_while_loop() {
        let src = r#"
fn first_ge(int target) -> int {
    let i = 0;
    while true {
        i = i + 1;
        if i >= target {
            break;
        }
    }
    return i;
}
first_ge(5);
"#;
        match vm_ok(src) {
            Value::Int(5) => {}
            other => panic!("expected Int(5), got {:?}", other),
        }
    }

    #[test]
    fn break_in_for_in_loop() {
        let src = r#"
let arr = [10, 20, 30, 40, 50];
let found = 0;
for x in arr {
    if x == 30 {
        found = x;
        break;
    }
}
found;
"#;
        match vm_ok(src) {
            Value::Int(30) => {}
            other => panic!("expected Int(30), got {:?}", other),
        }
    }

    #[test]
    fn continue_in_for_in_loop_skips_element() {
        let src = r#"
let arr = [1, 2, 3, 4, 5];
let sum = 0;
for x in arr {
    if x == 3 {
        continue;
    }
    sum = sum + x;
}
sum;
"#;
        // 1+2+4+5 = 12 (skipped 3)
        match vm_ok(src) {
            Value::Int(12) => {}
            other => panic!("expected Int(12), got {:?}", other),
        }
    }

    // ── RES-3993: `if let` / `while let` desugar to a statement-position
    // `Match` — see `compile_match_stmt`'s doc comment. Before this fix,
    // every one of these compiled to `CompileError::Unsupported("Match")`.

    #[test]
    fn res3993_if_let_top_level_binds_pattern_and_runs_matching_arm() {
        let src = r#"
let x = 0;
if let 0 = x {
    x = 100;
} else {
    x = 1;
}
x;
"#;
        match vm_ok(src) {
            Value::Int(100) => {}
            other => panic!("expected Int(100), got {:?}", other),
        }
    }

    #[test]
    fn res3993_if_let_top_level_falls_through_to_else_on_no_match() {
        let src = r#"
let x = 7;
if let 0 = x {
    x = 100;
} else {
    x = 1;
}
x;
"#;
        match vm_ok(src) {
            Value::Int(1) => {}
            other => panic!("expected Int(1), got {:?}", other),
        }
    }

    #[test]
    fn res3993_if_let_in_fn_body_returns_from_matching_arm() {
        let src = r#"
fn classify(int n) -> string {
    if let 0 = n {
        return "zero";
    }
    return "other";
}
classify(0);
"#;
        match vm_ok(src) {
            Value::String(s) => assert_eq!(s, "zero"),
            other => panic!("expected String(\"zero\"), got {:?}", other),
        }
    }

    #[test]
    fn res3993_while_let_top_level_drains_matching_pattern_then_breaks() {
        // Mirrors examples/while_let.rz's "literal-pattern drain" case:
        // the loop body only runs while the scrutinee equals the
        // literal pattern; the wildcard fallthrough arm breaks.
        let src = r#"
let counter = 3;
let runs = 0;
while let 3 = counter {
    runs = runs + 1;
    counter = counter - 1;
}
runs;
"#;
        match vm_ok(src) {
            Value::Int(1) => {}
            other => panic!("expected Int(1), got {:?}", other),
        }
    }

    #[test]
    fn res3993_while_let_in_fn_body_with_identifier_pattern_and_break() {
        let src = r#"
fn drain_to(int limit) -> int {
    let i = 0;
    let last = 0;
    while let n = i {
        last = n;
        if n >= limit {
            break;
        }
        i = i + 1;
    }
    return last;
}
drain_to(4);
"#;
        match vm_ok(src) {
            Value::Int(4) => {}
            other => panic!("expected Int(4), got {:?}", other),
        }
    }

    #[test]
    fn assert_passes_when_condition_true() {
        let src = r#"
let x = 5;
assert(x > 0);
x;
"#;
        match vm_ok(src) {
            Value::Int(5) => {}
            other => panic!("expected Int(5), got {:?}", other),
        }
    }

    #[test]
    fn assert_fails_when_condition_false() {
        let src = r#"
let x = -1;
assert(x > 0);
x;
"#;
        let err = vm_run(src);
        assert!(
            matches!(err.kind(), crate::vm::VmError::AssertionFailed(_)),
            "expected AssertionFailed, got {:?}",
            err
        );
    }

    #[test]
    fn assert_with_custom_message() {
        let src = r#"
assert(false, "custom failure message");
"#;
        let err = vm_run(src);
        match err.kind() {
            crate::vm::VmError::AssertionFailed(msg) => {
                assert!(
                    msg.contains("custom failure message"),
                    "expected custom message in {:?}",
                    msg
                );
            }
            other => panic!("expected AssertionFailed, got {:?}", other),
        }
    }

    #[test]
    fn assert_in_fn_body_passes() {
        let src = r#"
fn check(int n) -> int {
    assert(n >= 0);
    return n * 2;
}
check(7);
"#;
        match vm_ok(src) {
            Value::Int(14) => {}
            other => panic!("expected Int(14), got {:?}", other),
        }
    }

    #[test]
    fn assert_in_fn_body_fails() {
        let src = r#"
fn check(int n) -> int {
    assert(n >= 0, "n must be non-negative");
    return n;
}
check(-1);
"#;
        let err = vm_run(src);
        assert!(
            matches!(err.kind(), crate::vm::VmError::AssertionFailed(_)),
            "expected AssertionFailed, got {:?}",
            err
        );
    }

    #[test]
    fn assert_dynamic_message_variable() {
        let src = r#"
let reason = "too small";
assert(false, reason);
"#;
        let err = vm_run(src);
        match err.kind() {
            crate::vm::VmError::AssertionFailed(msg) => {
                assert!(
                    msg.contains("too small"),
                    "expected dynamic message in {:?}",
                    msg
                );
            }
            other => panic!("expected AssertionFailed, got {:?}", other),
        }
    }

    #[test]
    fn assert_dynamic_message_concatenation() {
        let src = r#"
assert(false, "x was " + to_string(42));
"#;
        let err = vm_run(src);
        match err.kind() {
            crate::vm::VmError::AssertionFailed(msg) => {
                assert!(msg.contains("x was 42"), "expected 'x was 42' in {:?}", msg);
            }
            other => panic!("expected AssertionFailed, got {:?}", other),
        }
    }

    #[test]
    fn assert_dynamic_message_in_function() {
        let src = r#"
fn check(int n) -> int {
    assert(n > 0, "got " + to_string(n));
    return n;
}
check(-5);
"#;
        let err = vm_run(src);
        match err.kind() {
            crate::vm::VmError::AssertionFailed(msg) => {
                assert!(msg.contains("got -5"), "expected 'got -5' in {:?}", msg);
            }
            other => panic!("expected AssertionFailed, got {:?}", other),
        }
    }

    #[test]
    fn closure_captures_var_in_assert_message() {
        let src = r#"
let label = "test";
let f = fn() -> int {
    assert(true, label);
    return 1;
};
f();
"#;
        match vm_ok(src) {
            Value::Int(1) => {}
            other => panic!("expected Int(1), got {:?}", other),
        }
    }

    #[test]
    fn break_outside_loop_is_compile_error() {
        let src = "break;";
        let prog = parse_one(src);
        assert!(
            compile(&prog).is_err(),
            "expected compile error for break outside loop"
        );
    }

    #[test]
    fn continue_outside_loop_is_compile_error() {
        let src = "continue;";
        let prog = parse_one(src);
        assert!(
            compile(&prog).is_err(),
            "expected compile error for continue outside loop"
        );
    }

    #[test]
    fn nested_break_targets_inner_loop() {
        // The inner break should exit only the inner while; outer loop counts to 3.
        let src = r#"
let outer = 0;
while outer < 3 {
    let inner = 0;
    while true {
        inner = inner + 1;
        if inner == 2 {
            break;
        }
    }
    outer = outer + 1;
}
outer;
"#;
        match vm_ok(src) {
            Value::Int(3) => {}
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    // ---------- RES-384b / RES-291 / RES-921 / RES-152: new compile coverage ----------

    #[test]
    fn static_let_compiles_as_local() {
        let src = "static let x = 42; x;";
        match vm_ok(src) {
            Value::Int(42) => {}
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    #[test]
    fn res4045_return_after_if_ending_in_assert_fail() {
        // RES-4045: `return EXPR;` immediately following an `if COND {
        // ...; assert(false, msg); }` (if-branch unconditionally fails,
        // no `else`) used to compile to bytecode that dropped the
        // `LoadLocal` for `EXPR` — `ReturnFromCall` popped an empty
        // operand stack and silently returned `Value::Void`. Root
        // cause: the peephole optimizer's `Not; JumpIfFalse` ->
        // `JumpIfTrue` fusion (and its 3 siblings) recorded the wrong
        // "original PC" for the jump-relink pass, leaving a stale
        // pre-fold offset in place whenever anything shifted between
        // the jump and its target.
        let src = r#"
fn read_temp_sensor(int nominal) -> int {
    return nominal;
}
fn is_plausible(int x) -> bool {
    return x != 9999;
}
fn safe_read(int nominal) -> int {
    let reading = read_temp_sensor(nominal);
    let ok = is_plausible(reading);
    if !ok {
        assert(false, "implausible");
    }
    return reading;
}
safe_read(500);
"#;
        match vm_ok(src) {
            Value::Int(500) => {}
            other => panic!("expected Int(500), got {:?}", other),
        }
    }

    #[test]
    fn res4046_function_scoped_static_let_persists_across_calls() {
        // RES-4046: a function-scoped `static let` used to compile as
        // an ordinary local — reset to its initializer on every call —
        // instead of persisting like the tree-walking interpreter's
        // `self.statics` (and like top-level `static let`, which is
        // trivially "persistent" since top-level code runs once).
        let src = r#"
fn read_random() {
    static let toggle = false;
    toggle = !toggle;
    if toggle {
        return 0.25;
    } else {
        return 0.75;
    }
}
fn three_calls() {
    let a = read_random();
    let b = read_random();
    let c = read_random();
    return [a, b, c];
}
three_calls();
"#;
        match vm_ok(src) {
            Value::Array(v) => {
                assert_eq!(v.len(), 3, "expected 3 elements, got {:?}", v);
                assert!(
                    matches!(v[0], Value::Float(f) if f == 0.25)
                        && matches!(v[1], Value::Float(f) if f == 0.75)
                        && matches!(v[2], Value::Float(f) if f == 0.25),
                    "static let must persist (toggle) across separate calls, got {:?}",
                    v
                );
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn const_decl_is_noop_in_vm() {
        // `const` is pre-evaluated; the bytecode should compile cleanly.
        let p = parse_one("const LIMIT = 10;");
        assert!(compile(&p).is_ok(), "const decl must compile");
    }

    #[test]
    fn live_block_body_executes() {
        // `live { ... }` compiles as a plain block in the VM.
        let src = r#"
let x = 0;
live {
    x = 5;
}
x;
"#;
        match vm_ok(src) {
            Value::Int(5) => {}
            other => panic!("expected Int(5), got {:?}", other),
        }
    }

    #[test]
    fn assume_and_invariant_are_noops() {
        // Verification-only constructs compile to no ops — program still runs.
        let src = r#"
let x = 3;
assume(x > 0, "x must be positive");
x;
"#;
        match vm_ok(src) {
            Value::Int(3) => {}
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    #[test]
    fn bytes_literal_compiles_to_bytes_value() {
        let p = parse_one(r#"b"hello";"#);
        let prog = compile(&p).expect("bytes literal must compile");
        assert!(
            prog.main
                .constants
                .iter()
                .any(|c| matches!(c, Value::Bytes(_))),
            "constant pool must contain a Bytes constant"
        );
    }

    // RES-4000 (test change): these two tests previously asserted that
    // `let r = <range>; r;` produced a `Value::Array` under `--vm` —
    // that was the bug (`type_of(1..5)` reported `"array"` under `--vm`
    // vs `"range"` on the interpreter). The fix lowers `Node::Range` to
    // a first-class `Value::Range` (mirroring the interpreter's
    // `eval_range_value`) instead of eagerly calling `array_range`, so
    // the correct assertion is `Value::Range`, not `Value::Array`.
    #[test]
    fn range_expr_exclusive_produces_range_value() {
        let src = "let r = 0..3; r;";
        match vm_ok(src) {
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                assert_eq!(start, 0);
                assert_eq!(end, 3);
                assert!(!inclusive);
            }
            other => panic!("expected Range(0..3), got {:?}", other),
        }
    }

    #[test]
    fn range_expr_inclusive_produces_range_value() {
        let src = "let r = 1..=3; r;";
        match vm_ok(src) {
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                assert_eq!(start, 1);
                assert_eq!(end, 3);
                assert!(inclusive);
            }
            other => panic!("expected Range(1..=3), got {:?}", other),
        }
    }

    // RES-4000: `for` iteration over a range literal must still yield
    // the same element sequence it did before — `Op::IterPrepare`
    // materializes the `Value::Range` into an array internally, but
    // that's an implementation detail of loop compilation, not a
    // change in `for`-loop semantics.
    #[test]
    fn range_expr_for_in_exclusive_iterates_correctly() {
        let src = "let sum = 0; for i in 0..3 { sum = sum + i; } sum;";
        assert!(matches!(vm_ok(src), Value::Int(3))); // 0+1+2
    }

    #[test]
    fn range_expr_for_in_inclusive_iterates_correctly() {
        let src = "let sum = 0; for i in 1..=3 { sum = sum + i; } sum;";
        assert!(matches!(vm_ok(src), Value::Int(6))); // 1+2+3
    }

    // ── RES-4060: `Node::Quantifier` (forall/exists) VM lowering ──

    #[test]
    fn quantifier_forall_range_true() {
        let src = "forall i in 0..5: i >= 0;";
        assert!(matches!(vm_ok(src), Value::Bool(true)));
    }

    #[test]
    fn quantifier_forall_range_false_short_circuits() {
        let src = "forall i in 0..5: i < 3;";
        assert!(matches!(vm_ok(src), Value::Bool(false)));
    }

    #[test]
    fn quantifier_forall_vacuous_range_is_true() {
        let src = "forall i in 5..5: false;";
        assert!(matches!(vm_ok(src), Value::Bool(true)));
    }

    #[test]
    fn quantifier_exists_range_true() {
        let src = "exists i in 0..5: i == 2;";
        assert!(matches!(vm_ok(src), Value::Bool(true)));
    }

    #[test]
    fn quantifier_exists_range_false() {
        let src = "exists i in 0..5: i < 0;";
        assert!(matches!(vm_ok(src), Value::Bool(false)));
    }

    #[test]
    fn quantifier_exists_over_array_literal() {
        let src = "exists x in [1, 5, 9]: x > 4;";
        assert!(matches!(vm_ok(src), Value::Bool(true)));
    }

    #[test]
    fn quantifier_forall_over_array_referencing_indices() {
        let src = "let a = [10, 20, 30]; forall i in 0..len(a): a[i] > 0;";
        assert!(matches!(vm_ok(src), Value::Bool(true)));
    }

    #[test]
    fn quantifier_var_does_not_leak_outer_scope() {
        // The quantified variable is scoped to the quantifier body only —
        // an outer `i` of the same name must survive unchanged.
        let src = "let i = 99; let r = forall i in 0..3: i < 10; i;";
        assert!(matches!(vm_ok(src), Value::Int(99)));
    }

    #[test]
    fn quantifier_inside_assert_true_does_not_fail() {
        let src = r#"assert(forall i in 0..10: i + 1 > 0); "ok";"#;
        match vm_ok(src) {
            Value::String(s) => assert_eq!(s, "ok"),
            other => panic!("expected String(\"ok\"), got {:?}", other),
        }
    }

    #[test]
    fn quantifier_inside_assert_false_raises_assertion_failed() {
        let src = "assert(forall i in 0..5: i < 3);";
        match vm_run(src) {
            crate::vm::VmError::AtLine { kind, .. } => {
                assert!(matches!(*kind, crate::vm::VmError::AssertionFailed(_)));
            }
            other => panic!("expected AssertionFailed, got {:?}", other),
        }
    }

    // ── RES-4119: `Node::DeferStatement` VM lowering ──

    #[test]
    fn defer_emits_defer_push_and_thunk_function() {
        let src = r#"fn f() { defer 1 + 1; } f();"#;
        let prog = parse_one(src);
        let p = compile(&prog).expect("compiles");
        let f = p.functions.iter().find(|f| f.name == "f").expect("f");
        assert!(
            f.chunk.code.iter().any(|op| matches!(op, Op::DeferPush(_))),
            "f's chunk must contain Op::DeferPush: {:?}",
            f.chunk.code
        );
        assert!(
            p.functions.iter().any(|f| f.name == "$defer"),
            "program must contain a synthesized $defer thunk"
        );
    }

    #[test]
    fn defer_does_not_alter_return_value() {
        let src = "fn f() -> int { defer 1 + 1; return 7; } f();";
        assert!(matches!(vm_ok(src), Value::Int(7)));
    }

    #[test]
    fn defer_runs_on_implicit_end_of_body() {
        // The deferred `1 / 0` fires on the implicit fall-off-the-end
        // return path — the call errors even though the body has no
        // explicit `return`.
        let src = "fn f() { defer 1 / 0; } f();";
        let err = format!("{:?}", vm_run(src));
        assert!(err.contains("DivideByZero"), "got: {err}");
    }

    #[test]
    fn defer_runs_on_early_return() {
        let src = "fn f() -> int { defer 1 / 0; return 1; } f();";
        let err = format!("{:?}", vm_run(src));
        assert!(err.contains("DivideByZero"), "got: {err}");
    }

    #[test]
    fn defer_lifo_order_last_registered_fires_first() {
        // Both defers error with distinguishable errors; LIFO means the
        // second-registered one runs (and fails) first, so its error —
        // ArrayIndexOutOfBounds, not DivisionByZero — is the one kept.
        let src = "fn f() { defer 1 / 0; defer [1][5]; } f();";
        let err = format!("{:?}", vm_run(src));
        assert!(
            err.contains("ArrayIndexOutOfBounds"),
            "LIFO: expected the later defer's error first, got: {err}"
        );
    }

    #[test]
    fn defer_sees_reassignment_after_registration() {
        // Matches the tree-walker: its captured Environment is
        // Rc<RefCell>-shared with the live one, so a reassignment
        // between `defer` and function exit is visible to the deferred
        // expr. `1 / x` only succeeds with the NEW value (1), so a
        // stale defer-time snapshot (x = 0) would error here.
        let src = "fn f() -> int { let x = 0; defer 1 / x; x = 1; return x; } f();";
        assert!(matches!(vm_ok(src), Value::Int(1)));
    }

    #[test]
    fn defer_body_error_takes_precedence_over_defer_error() {
        // Body fails (array index out of bounds) before any return path
        // is reached — the reported error is the body's, never the
        // deferred `1 / 0`'s, observably matching the interpreter
        // (which drains the defer but discards its error in favor of
        // the body's).
        let src = "fn f() -> int { defer 1 / 0; return [1][5]; } f();";
        let err = format!("{:?}", vm_run(src));
        assert!(err.contains("ArrayIndexOutOfBounds"), "got: {err}");
    }

    #[test]
    fn defer_in_nested_calls_drains_per_frame() {
        // Each frame drains its own stack: inner's defer divides by
        // `y - 4` (only sound against inner's own local), outer's
        // return value is untouched by either drain.
        let src = r#"
fn inner() -> int { let y = 5; defer 1 / (y - 4); return y; }
fn outer() -> int { defer 1 + 1; let v = inner(); return v + 1; }
outer();
"#;
        assert!(matches!(vm_ok(src), Value::Int(6)));
    }

    #[test]
    fn slice_expr_basic() {
        let src = "let a = [10, 20, 30, 40]; a[1..3];";
        match vm_ok(src) {
            Value::Array(v) => {
                assert_eq!(v.len(), 2);
                assert!(matches!(v[0], Value::Int(20)));
                assert!(matches!(v[1], Value::Int(30)));
            }
            other => panic!("expected [20,30], got {:?}", other),
        }
    }

    #[test]
    fn slice_expr_inclusive() {
        let src = "let a = [10, 20, 30, 40]; a[1..=2];";
        match vm_ok(src) {
            Value::Array(v) => {
                assert_eq!(v.len(), 2);
                assert!(matches!(v[0], Value::Int(20)));
                assert!(matches!(v[1], Value::Int(30)));
            }
            other => panic!("expected [20,30], got {:?}", other),
        }
    }

    #[test]
    fn slice_negative_hi_not_confused_with_sentinel() {
        let src = "let a = [10, 20, 30, 40, 50]; a[0..-1];";
        match vm_ok(src) {
            Value::Array(v) => {
                assert_eq!(v.len(), 4, "xs[0..-1] must exclude the last element");
                assert!(matches!(v[0], Value::Int(10)));
                assert!(matches!(v[3], Value::Int(40)));
            }
            other => panic!("expected [10,20,30,40], got {:?}", other),
        }
    }

    #[test]
    fn slice_no_hi_gives_full_array() {
        let src = "let a = [10, 20, 30]; a[0..];";
        match vm_ok(src) {
            Value::Array(v) => assert_eq!(v.len(), 3),
            other => panic!("expected 3-element array, got {:?}", other),
        }
    }

    // ── DurationLiteral ──────────────────────────────────────────────────────

    #[test]
    fn duration_literal_compiles_in_live_block() {
        // DurationLiteral appears as the `deadline` of a `live within`
        // block. The bytecode compiler ignores the deadline and compiles
        // the body; the live block should run without Unsupported errors.
        match vm_ok("live within 100ms { 42; } 42;") {
            Value::Int(42) => {}
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    // ── NewtypeConstruct ─────────────────────────────────────────────────────

    #[test]
    fn newtype_construct_wraps_value() {
        // lower_program rewrites Meters(42) → NewtypeConstruct; the bytecode
        // compiler must not return Unsupported. Result is a Struct.
        let mut p = parse_one("newtype Meters = Int; let x = Meters(42); x;");
        crate::newtypes::lower_program(&mut p);
        let prog = compile(&p).expect("NewtypeConstruct must compile");
        let v = crate::vm::run(&prog).expect("NewtypeConstruct must run");
        assert!(
            matches!(v, Value::Struct { .. }),
            "expected Struct from newtype constructor, got {:?}",
            v
        );
    }

    // ── TryExpression (bytecode VM path) ─────────────────────────────────────

    #[test]
    fn try_unwrap_ok_result_via_vm() {
        // Build a tiny program directly in bytecode: push `Result{ok:true,
        // payload:Int(42)}`, emit TryUnwrap, emit Return. The VM must
        // leave Int(42) on the stack.
        use crate::bytecode::{Chunk, Op, Program};
        let mut main = Chunk::new();
        // Const 0 → Value::Result { ok: true, payload: Box(Int(42)) }
        main.constants.push(Value::Result {
            ok: true,
            payload: Box::new(Value::Int(42)),
        });
        main.code.push(Op::Const(0));
        main.line_info.push(1);
        main.code.push(Op::TryUnwrap);
        main.line_info.push(1);
        main.code.push(Op::Return);
        main.line_info.push(1);
        let prog = Program {
            main,
            functions: Vec::new(),
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        match crate::vm::run(&prog).expect("must run") {
            Value::Int(42) => {}
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    #[test]
    fn try_unwrap_some_option_via_vm() {
        use crate::bytecode::{Chunk, Op, Program};
        let mut main = Chunk::new();
        main.constants
            .push(Value::Option(Some(Box::new(Value::Int(7)))));
        main.code.push(Op::Const(0));
        main.line_info.push(1);
        main.code.push(Op::TryUnwrap);
        main.line_info.push(1);
        main.code.push(Op::Return);
        main.line_info.push(1);
        let prog = Program {
            main,
            functions: Vec::new(),
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        match crate::vm::run(&prog).expect("must run") {
            Value::Int(7) => {}
            other => panic!("expected Int(7), got {:?}", other),
        }
    }

    #[test]
    fn try_unwrap_err_result_early_returns() {
        // TryUnwrap on Err early-returns to the caller. When in main
        // (frames.len()==1 after pop), the VM halts with the Err value.
        use crate::bytecode::{Chunk, Op, Program};
        let mut main = Chunk::new();
        main.constants.push(Value::Result {
            ok: false,
            payload: Box::new(Value::Int(99)),
        });
        main.code.push(Op::Const(0));
        main.line_info.push(1);
        main.code.push(Op::TryUnwrap);
        main.line_info.push(1);
        // This Return is unreachable; TryUnwrap halts via early-return.
        main.code.push(Op::Return);
        main.line_info.push(1);
        let prog = Program {
            main,
            functions: Vec::new(),
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        match crate::vm::run(&prog).expect("must run") {
            Value::Result { ok: false, payload } => {
                assert!(
                    matches!(*payload, Value::Int(99)),
                    "expected Int(99) payload, got {:?}",
                    payload
                );
            }
            other => panic!("expected Err(99), got {:?}", other),
        }
    }

    // ── NamedArg ─────────────────────────────────────────────────────────────

    #[test]
    fn named_arg_compiles_without_unsupported() {
        // NamedArg nodes appear at call sites with labelled arguments.
        // The bytecode compiler must not return Unsupported for them.
        // Compile `add(a: 3, b: 4)` — only the values matter.
        let p = parse_one("fn add(int a, int b) -> int { return a + b; } add(a: 3, b: 4);");
        let prog = compile(&p).expect("NamedArg must compile");
        match crate::vm::run(&prog).expect("must run") {
            Value::Int(7) => {}
            other => panic!("expected Int(7), got {:?}", other),
        }
    }

    // ── RES-2502: labeled break/continue ────────────────────────────────────

    fn compile_run(src: &str) -> Result<Value, String> {
        let prog = parse_and_compile(src)?;
        crate::vm::run(&prog).map_err(|e| format!("{e:?}"))
    }

    fn assert_int(v: Value, expected: i64) {
        match v {
            Value::Int(n) => assert_eq!(n, expected),
            other => panic!("expected Int({expected}), got {other:?}"),
        }
    }

    #[test]
    fn labeled_break_exits_outer_for() {
        let v = compile_run(
            r#"let found = 0;
outer: for i in [0, 1, 2, 3, 4] {
    for j in [0, 1, 2, 3, 4] {
        if i == 2 && j == 3 {
            found = i * 10 + j;
            break outer;
        }
    }
}
found;"#,
        )
        .unwrap();
        assert_int(v, 23);
    }

    #[test]
    fn labeled_break_exits_outer_while() {
        let v = compile_run(
            r#"let i = 0;
let found = 0;
outer: while i < 5 {
    let j = 0;
    while j < 5 {
        if i == 1 && j == 2 {
            found = i * 10 + j;
            break outer;
        }
        j = j + 1;
    }
    i = i + 1;
}
found;"#,
        )
        .unwrap();
        assert_int(v, 12);
    }

    #[test]
    fn labeled_continue_skips_outer_iteration() {
        let v = compile_run(
            r#"let sum = 0;
outer: for i in [0, 1, 2, 3] {
    for j in [0, 1, 2] {
        if j == 1 {
            continue outer;
        }
        sum = sum + 1;
    }
}
sum;"#,
        )
        .unwrap();
        assert_int(v, 4);
    }

    #[test]
    fn unlabeled_break_inside_labeled_loop() {
        let v = compile_run(
            r#"let count = 0;
outer: for i in [0, 1, 2] {
    for j in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
        if j == 2 { break; }
        count = count + 1;
    }
}
count;"#,
        )
        .unwrap();
        assert_int(v, 6);
    }

    #[test]
    fn labeled_break_inner_named_loop() {
        let v = compile_run(
            r#"let x = 0;
outer: for i in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
    inner: for j in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
        if j == 5 { break inner; }
        x = x + 1;
    }
    if i == 2 { break outer; }
}
x;"#,
        )
        .unwrap();
        assert_int(v, 15);
    }

    #[test]
    fn labeled_break_in_fn_body() {
        let v = compile_run(
            r#"fn search(IntArr xs, IntArr ys) -> int {
    outer: for i in xs {
        for j in ys {
            if i == 2 && j == 3 {
                return i * 10 + j;
            }
        }
    }
    return 0;
}
search([0, 1, 2, 3], [0, 1, 2, 3]);"#,
        )
        .unwrap();
        assert_int(v, 23);
    }

    #[test]
    fn closure_capture_with_body_local() {
        let v = compile_run(
            "let outer = 10;\nlet f = fn(Int x) { let local = x + 1; local + outer; };\nf(5);",
        )
        .unwrap();
        assert_int(v, 16);
    }

    #[test]
    fn closure_capture_two_vars_with_body_local() {
        let v = compile_run(
            "let a = 3;\nlet b = 7;\nlet f = fn(Int x) { let c = x * 2; c + a + b; };\nf(4);",
        )
        .unwrap();
        assert_int(v, 18);
    }

    #[test]
    fn closure_capture_mutation_visible() {
        let v = compile_run("let outer = 10;\nlet f = fn() { outer = 99; outer; };\nf();").unwrap();
        assert_int(v, 99);
    }

    #[test]
    fn closure_no_capture_with_locals() {
        let v =
            compile_run("let f = fn(Int x) { let a = x + 1; let b = a * 2; b; };\nf(5);").unwrap();
        assert_int(v, 12);
    }

    #[test]
    fn closure_capture_only_no_body_locals() {
        let v = compile_run("let outer = 42;\nlet f = fn() { outer; };\nf();").unwrap();
        assert_int(v, 42);
    }

    // RES-2506: tests for collect_free_vars coverage — each test
    // exercises a node type that was previously missed by the catch-all.

    #[test]
    fn closure_capture_in_for_in() {
        let v = compile_run(
            "let arr = [10, 20, 30];\nlet f = fn() {\n  let sum = 0;\n  for x in arr { sum = sum + x; }\n  sum;\n};\nf();",
        ).unwrap();
        assert_int(v, 60);
    }

    #[test]
    fn closure_capture_in_index_expr() {
        let v =
            compile_run("let arr = [10, 20, 30];\nlet f = fn(Int i) { arr[i]; };\nf(2);").unwrap();
        assert_int(v, 30);
    }

    #[test]
    fn closure_capture_in_array_literal() {
        let v = compile_run(
            "let a = 1;\nlet b = 2;\nlet f = fn() { let arr = [a, b, 3]; arr[0] + arr[1] + arr[2]; };\nf();",
        ).unwrap();
        assert_int(v, 6);
    }

    #[test]
    fn closure_capture_in_assignment() {
        let v = compile_run("let outer = 10;\nlet f = fn(Int x) { outer = x; outer; };\nf(77);")
            .unwrap();
        assert_int(v, 77);
    }

    #[test]
    fn closure_capture_in_interpolated_string() {
        let v = compile_run(
            r#"let name = "world";
let f = fn() { "hello {name}"; };
f();"#,
        )
        .unwrap();
        match v {
            Value::String(s) => assert_eq!(s, "hello world"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn closure_capture_in_match_expr() {
        let v = compile_run(
            "let outer = 100;\nlet f = fn(Int x) { match x { 1 => outer, _ => 0 }; };\nf(1);",
        )
        .unwrap();
        assert_int(v, 100);
    }

    #[test]
    fn closure_capture_in_field_access() {
        let v = compile_run(
            "struct Point { int x, int y }\nlet p = new Point { x: 5, y: 10 };\nlet f = fn() { p.x + p.y; };\nf();",
        )
        .unwrap();
        assert_int(v, 15);
    }

    #[test]
    fn closure_capture_in_struct_literal() {
        let v = compile_run(
            "struct Pair { int a, int b }\nlet x = 3;\nlet y = 7;\nlet f = fn() { let p = new Pair { a: x, b: y }; p.a + p.b; };\nf();",
        )
        .unwrap();
        assert_int(v, 10);
    }

    // RES-2512: tests for the 4 node types that were still missing
    // from collect_free_vars after RES-2506.

    #[test]
    fn closure_capture_in_static_let() {
        let v = compile_run("let outer = 42;\nlet f = fn() { static let x = outer; x; };\nf();")
            .unwrap();
        assert_int(v, 42);
    }

    #[test]
    fn labeled_continue_in_fn_body() {
        let v = compile_run(
            r#"fn count_first_cols(IntArr rows, IntArr cols) -> int {
    let sum = 0;
    outer: for i in rows {
        for j in cols {
            if j == 1 { continue outer; }
            sum = sum + 1;
        }
    }
    return sum;
}
count_first_cols([0, 1, 2, 3], [0, 1, 2]);"#,
        )
        .unwrap();
        assert_int(v, 4);
    }

    #[test]
    fn if_expression_in_let() {
        let v = compile_run(
            r#"
fn choose(bool b) -> int {
    let x = if b { 1 } else { 2 };
    return x;
}
choose(true);"#,
        )
        .unwrap();
        assert_int(v, 1);

        let v2 = compile_run(
            r#"
fn choose(bool b) -> int {
    let x = if b { 1 } else { 2 };
    return x;
}
choose(false);"#,
        )
        .unwrap();
        assert_int(v2, 2);
    }

    #[test]
    fn if_expression_nested() {
        let v = compile_run(
            r#"
fn nested(bool a, bool b) -> int {
    let x = if a { if b { 10 } else { 20 } } else { 30 };
    return x;
}
nested(true, false);"#,
        )
        .unwrap();
        assert_int(v, 20);
    }

    #[test]
    fn if_expression_multi_stmt_block() {
        let v = compile_run(
            r#"
fn compute(bool b) -> int {
    let x = if b {
        let tmp = 10;
        let result = tmp + 5;
        result
    } else {
        let tmp = 20;
        tmp
    };
    return x;
}
compute(true);"#,
        )
        .unwrap();
        assert_int(v, 15);
    }

    #[test]
    fn if_expression_in_return() {
        let v = compile_run(
            r#"
fn abs_val(int n) -> int {
    return if n < 0 { 0 - n } else { n };
}
abs_val(-7);"#,
        )
        .unwrap();
        assert_int(v, 7);
    }

    #[test]
    fn bare_none_compiles_as_builtin() {
        let v = compile_run("None;").unwrap();
        assert!(
            matches!(v, Value::Option(None)),
            "expected None, got {:?}",
            v
        );
    }

    #[test]
    fn none_in_function_return() {
        let v = compile_run(
            r#"
fn maybe(int x) {
    if x > 0 { return Some(x); }
    return None;
}
maybe(-1);"#,
        )
        .unwrap();
        assert!(
            matches!(v, Value::Option(None)),
            "expected None, got {:?}",
            v
        );
    }

    #[test]
    fn none_vs_some_round_trip() {
        let v = compile_run(
            r#"
fn maybe(int x) {
    if x > 0 { return Some(x); }
    return None;
}
maybe(5);"#,
        )
        .unwrap();
        match v {
            Value::Option(Some(inner)) => assert_int(*inner, 5),
            other => panic!("expected Some(5), got {:?}", other),
        }
    }
}
