// region_inference.rs
//
// RES-394 PR 1: region-variable machinery + unification table.
// RES-394 PR 2: inference pass — assigns region vars to unlabeled
//               reference parameters and walks the call graph.
#![allow(dead_code)]

use std::collections::HashMap;

// ============================================================
// Region vocabulary
// ============================================================

/// An inference variable assigned to an unlabeled reference parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionVar(pub u32);

/// A region is either a concrete user-declared label or an inference variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Region {
    /// A user-declared region label, e.g. from `region A;`.
    Named(String),
    /// An unresolved inference variable.
    Var(RegionVar),
}

impl Region {
    /// Convenience constructor.
    pub fn named(label: impl Into<String>) -> Self {
        Region::Named(label.into())
    }
}

// ============================================================
// Union-find table
// ============================================================

/// Maps region variables to their canonical `Region` representative.
///
/// Implements a simple union-find (without path compression): each variable
/// either points to another `Region` (its representative) or is free.
pub struct RegionTable {
    next_id: u32,
    parent: HashMap<u32, Region>,
}

impl RegionTable {
    pub fn new() -> Self {
        RegionTable {
            next_id: 0,
            parent: HashMap::new(),
        }
    }

    /// Allocate a fresh region variable.
    pub fn fresh(&mut self) -> RegionVar {
        let id = self.next_id;
        self.next_id += 1;
        RegionVar(id)
    }

    /// Resolve a `Region` to its canonical representative.
    ///
    /// Follows variable chains until a `Region::Named` or an unbound
    /// `Region::Var` is reached.
    pub fn resolve(&self, mut r: Region) -> Region {
        loop {
            match &r {
                Region::Var(v) => match self.parent.get(&v.0) {
                    Some(parent) => r = parent.clone(),
                    None => return r,
                },
                Region::Named(_) => return r,
            }
        }
    }

    /// Unify two regions — constrain them to refer to the same memory area.
    ///
    /// Returns `Err` if both regions resolve to different concrete labels
    /// (i.e. the user labeled them differently and they truly cannot alias).
    pub fn unify(&mut self, a: Region, b: Region) -> Result<(), String> {
        let ra = self.resolve(a);
        let rb = self.resolve(b);

        if ra == rb {
            return Ok(());
        }

        match (ra, rb) {
            // Variable unified with a concrete label or another variable.
            (Region::Var(va), rhs) => {
                self.parent.insert(va.0, rhs);
                Ok(())
            }
            // Concrete label unified with a variable.
            (lhs, Region::Var(vb)) => {
                self.parent.insert(vb.0, lhs);
                Ok(())
            }
            // Two different concrete labels — genuine conflict.
            (Region::Named(a), Region::Named(b)) => Err(format!(
                "region conflict: label `{}` cannot unify with label `{}`",
                a, b
            )),
        }
    }

    /// Return the number of variables allocated so far.
    pub fn var_count(&self) -> u32 {
        self.next_id
    }
}

impl Default for RegionTable {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================
// Region map — per-function parameter→region mapping
// ============================================================

/// Identifies a specific function parameter by function name and
/// zero-based index within the parameter list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParamKey {
    pub fn_name: String,
    pub param_idx: usize,
}

/// Identifies a local variable by function name and variable name.
/// RES-773: extended region tracking to locals (not just parameters).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalKey {
    pub fn_name: String,
    pub var_name: String,
}

/// Associates each reference parameter and local variable with an inferred `Region`.
pub struct RegionMap {
    pub table: RegionTable,
    /// Mapping from `(fn_name, param_idx)` → `Region`.
    pub entries: HashMap<ParamKey, Region>,
    /// RES-773: mapping from `(fn_name, var_name)` → `Region` for local variables.
    pub local_entries: HashMap<LocalKey, Region>,
}

impl RegionMap {
    fn new() -> Self {
        RegionMap {
            table: RegionTable::new(),
            entries: HashMap::new(),
            local_entries: HashMap::new(),
        }
    }

    /// Look up the region for a parameter, resolving any inference
    /// variable to its canonical representative.
    pub fn get_resolved(&self, key: &ParamKey) -> Option<Region> {
        self.entries.get(key).map(|r| self.table.resolve(r.clone()))
    }

    /// RES-773: look up the region for a local variable, resolving any
    /// inference variable to its canonical representative.
    pub fn get_local_resolved(&self, key: &LocalKey) -> Option<Region> {
        self.local_entries
            .get(key)
            .map(|r| self.table.resolve(r.clone()))
    }
}

// ============================================================
// Inference pass (RES-394 PR 2)
// ============================================================

/// Parse the region label from an encoded parameter type string.
///
/// Replicates the logic in `crate::parse_ref_type` without needing
/// to import it (keeping this module self-contained).
fn region_from_type_str(ty: &str) -> Option<(bool, Option<String>)> {
    let rest = ty.strip_prefix('&')?;
    let (is_mut, rest) = if let Some(r) = rest.strip_prefix("mut") {
        (true, r)
    } else {
        (false, rest)
    };
    let rest = rest.trim_start();
    if let Some(after_bracket) = rest.strip_prefix('[') {
        let close = after_bracket.find(']')?;
        let label = after_bracket[..close].trim().to_string();
        if label.is_empty() {
            return Some((is_mut, None));
        }
        Some((is_mut, Some(label)))
    } else {
        Some((is_mut, None))
    }
}

/// Walk a node tree collecting all `Node::LetStatement` nodes to extract
/// local variable bindings with their type annotations.
fn collect_local_bindings(node: &crate::Node, locals: &mut Vec<(String, Option<String>)>) {
    match node {
        crate::Node::LetStatement {
            name,
            type_annot: Some(ty),
            ..
        } => {
            locals.push((name.clone(), Some(ty.clone())));
        }
        crate::Node::LetStatement { .. } => {} // Ignore untyped locals
        crate::Node::Block { stmts, .. } => {
            for s in stmts {
                collect_local_bindings(s, locals);
            }
        }
        crate::Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_local_bindings(condition, locals);
            collect_local_bindings(consequence, locals);
            if let Some(alt) = alternative {
                collect_local_bindings(alt, locals);
            }
        }
        crate::Node::WhileStatement {
            condition, body, ..
        } => {
            collect_local_bindings(condition, locals);
            collect_local_bindings(body, locals);
        }
        crate::Node::ForInStatement { body, .. } => collect_local_bindings(body, locals),
        _ => {}
    }
}

/// RES-394 PR 2: walk the program AST and build a `RegionMap` by
/// assigning region variables to unlabeled reference parameters.
/// RES-773: extended to also collect local variable bindings.
///
/// Labeled parameters/locals (`&[A] T`) keep their concrete `Region::Named`
/// label; unlabeled ones (`&T` / `&mut T`) receive a fresh `RegionVar`.
pub fn build_region_map(program: &crate::Node) -> RegionMap {
    let mut map = RegionMap::new();
    let stmts = match program {
        crate::Node::Program(s) => s,
        _ => return map,
    };
    for spanned in stmts {
        if let crate::Node::Function {
            name: fn_name,
            parameters,
            body,
            ..
        } = &spanned.node
        {
            for (idx, (ty, _pname)) in parameters.iter().enumerate() {
                if let Some((_is_mut, label)) = region_from_type_str(ty) {
                    let region = match label {
                        Some(l) => Region::named(l),
                        None => Region::Var(map.table.fresh()),
                    };
                    map.entries.insert(
                        ParamKey {
                            fn_name: fn_name.clone(),
                            param_idx: idx,
                        },
                        region,
                    );
                }
            }

            // RES-773: collect local variable bindings in the function body.
            let mut locals: Vec<(String, Option<String>)> = Vec::new();
            collect_local_bindings(body, &mut locals);
            for (var_name, type_annot) in locals {
                if let Some(ty) = type_annot
                    && let Some((_is_mut, label)) = region_from_type_str(&ty)
                {
                    let region = match label {
                        Some(l) => Region::named(l),
                        None => Region::Var(map.table.fresh()),
                    };
                    map.local_entries.insert(
                        LocalKey {
                            fn_name: fn_name.clone(),
                            var_name,
                        },
                        region,
                    );
                }
            }
        }
    }
    map
}

/// A-E5: region/lifetime inference entry point for UNANNOTATED code.
///
/// RES-1202 / RES-1611 history: this used to be a no-op stub — the
/// call-site region-label substitution check
/// (`check_call_site_region_aliasing`) only ever covers
/// region-*polymorphic* callees (`fn f<R, S>(...)`), because it needs a
/// declared/inferred region *label* on the caller's argument to build a
/// substitution. A plain (non-generic) function whose `&mut` parameters
/// carry no `[LABEL]` at all — the common case for code that hasn't
/// opted into the region system — was never checked at call sites: two
/// `&mut` parameters on the same function were only compared at the
/// *declaration* (`check_region_aliasing`'s pairwise loop), where two
/// unlabeled params always get distinct fresh `RegionVar`s and are
/// therefore always accepted (RES-394 D5) — there is no dataflow-driven
/// unification that would ever force them to collide. So passing the
/// *same* local variable into two `&mut` parameters of a plain function
/// compiled silently.
///
/// This pass closes that gap with a check that needs no label inference
/// at all: within a single call expression, if the same plain
/// identifier appears as the argument for two (or more) parameter
/// slots and at least one of those slots is a reference type
/// (`&`/`&mut`) with at least one of them `&mut`, the two references
/// are provably the same runtime binding — aliasing isn't a matter of
/// inference, it's syntactic identity at one evaluation point. That
/// makes this check unconditionally sound: no false positive is
/// possible, because two occurrences of the same identifier in the same
/// argument list *are* the same binding, full stop.
///
/// Deliberately conservative / deferred (tracked in a follow-up issue,
/// see the PR body for the number):
/// - Region-polymorphic callees (non-empty `type_params`) are skipped
///   here — `check_call_site_region_aliasing` already covers them via
///   region-label substitution, and skipping avoids double-reporting.
/// - No cross-statement / conditional-path aliasing (e.g. an `if` that
///   sometimes passes the same variable twice) — only literal syntactic
///   repetition within one call's argument list.
/// - No use-after-move detection: the language has no Copy/Move type
///   distinction outside `linear T` (see `linear.rs`), so there is no
///   sound way yet to tell whether re-reading a plain local after
///   passing it somewhere is a genuine violation or an ordinary Copy.
///   Enforcing that now would risk false positives on every ordinary
///   value type in the corpus.
/// - No whole-program / interprocedural analysis — call sites are
///   checked against the literal argument identifiers visible at that
///   call, not through further indirection (struct fields, arrays,
///   closures).
pub fn infer(program: &crate::Node, source_path: &str) -> Result<(), String> {
    let errors = check_unannotated_mut_alias(program, source_path);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Walk a node tree collecting all `Node::CallExpression` nodes whose
/// function slot is a plain `Node::Identifier`, alongside the call's
/// own span. Same traversal shape as `collect_calls`, extended with the
/// span so diagnostics can point at the offending call site rather than
/// falling back to the enclosing function's span.
fn collect_calls_with_span<'a>(
    node: &'a crate::Node,
    calls: &mut Vec<(&'a str, &'a [crate::Node], crate::span::Span)>,
) {
    match node {
        crate::Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            if let crate::Node::Identifier { name, .. } = function.as_ref() {
                calls.push((name.as_str(), arguments.as_slice(), *span));
            }
            for arg in arguments {
                collect_calls_with_span(arg, calls);
            }
        }
        crate::Node::Block { stmts, .. } => {
            for s in stmts {
                collect_calls_with_span(s, calls);
            }
        }
        crate::Node::LetStatement { value, .. } => collect_calls_with_span(value, calls),
        crate::Node::Assignment { value, .. } => collect_calls_with_span(value, calls),
        crate::Node::ReturnStatement { value: Some(v), .. } => collect_calls_with_span(v, calls),
        crate::Node::ReturnStatement { value: None, .. } => {}
        crate::Node::ExpressionStatement { expr, .. } => {
            collect_calls_with_span(expr, calls);
        }
        crate::Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_calls_with_span(condition, calls);
            collect_calls_with_span(consequence, calls);
            if let Some(alt) = alternative {
                collect_calls_with_span(alt, calls);
            }
        }
        // RES-4070: `match` arms were never walked here, so a `&mut`-alias
        // call hidden inside an arm body (or a guard) compiled silently —
        // the A-E5 straight-line check only ever saw `IfStatement`/`Block`/
        // etc. Each arm's guard and body run on the same footing as an
        // `if` branch: whichever arm actually matches, that arm's body is
        // the one that executes, so flagging a literal same-identifier
        // `&mut` repetition found in *any* arm is exactly as sound as the
        // existing per-`if`-branch behavior above — the flagged call is a
        // genuine violation whenever that arm is taken, regardless of
        // what the other arms do.
        crate::Node::Match {
            scrutinee, arms, ..
        } => {
            collect_calls_with_span(scrutinee, calls);
            for (_pattern, guard, body) in arms {
                if let Some(g) = guard {
                    collect_calls_with_span(g, calls);
                }
                collect_calls_with_span(body, calls);
            }
        }
        crate::Node::WhileStatement {
            condition, body, ..
        } => {
            collect_calls_with_span(condition, calls);
            collect_calls_with_span(body, calls);
        }
        crate::Node::ForInStatement { body, .. } => collect_calls_with_span(body, calls),
        crate::Node::InfixExpression { left, right, .. } => {
            collect_calls_with_span(left, calls);
            collect_calls_with_span(right, calls);
        }
        crate::Node::PrefixExpression { right, .. } => collect_calls_with_span(right, calls),
        _ => {}
    }
}

/// A-E5: the actual check backing [`infer`]. Split out so
/// `check_region_aliasing` in `lib.rs` can call it directly (mirroring
/// how it already calls `check_call_site_region_aliasing`), returning
/// every violation rather than stopping at the first.
pub fn check_unannotated_mut_alias(program: &crate::Node, source_path: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let stmts = match program {
        crate::Node::Program(s) => s,
        _ => return errors,
    };

    // fn_name -> parameter types, restricted to non-generic top-level
    // functions with at least one reference-typed parameter. Region-
    // polymorphic functions (`type_params` non-empty) are left to
    // `check_call_site_region_aliasing`.
    let mut callee_table: HashMap<&str, &[(String, String)]> = HashMap::new();
    for spanned in stmts {
        if let crate::Node::Function {
            name,
            type_params,
            parameters,
            ..
        } = &spanned.node
            && type_params.is_empty()
            && parameters.iter().any(|(ty, _)| ty.starts_with('&'))
        {
            callee_table.insert(name.as_str(), parameters.as_slice());
        }
    }
    if callee_table.is_empty() {
        return errors;
    }

    for spanned in stmts {
        let crate::Node::Function { body, .. } = &spanned.node else {
            continue;
        };
        let mut calls: Vec<(&str, &[crate::Node], crate::span::Span)> = Vec::new();
        collect_calls_with_span(body, &mut calls);

        for (callee_name, args, call_span) in calls {
            let Some(param_types) = callee_table.get(callee_name) else {
                continue;
            };
            if args.len() != param_types.len() {
                continue; // arity mismatch — typechecker handles it
            }

            // identifier name -> mutability of each reference-typed
            // slot it was passed into.
            let mut by_name: HashMap<&str, Vec<bool>> = HashMap::new();
            for (arg, (ty, _)) in args.iter().zip(param_types.iter()) {
                if let crate::Node::Identifier { name, .. } = arg
                    && let Some((is_mut, _label)) = region_from_type_str(ty)
                {
                    by_name.entry(name.as_str()).or_default().push(is_mut);
                }
            }

            let mut hits: Vec<(&str, usize)> = by_name
                .into_iter()
                .filter(|(_, muts)| muts.len() >= 2 && muts.iter().any(|m| *m))
                .map(|(name, muts)| (name, muts.len()))
                .collect();
            hits.sort_unstable();

            for (var_name, count) in hits {
                let loc = if call_span.start.line == 0 {
                    "E: ".to_string()
                } else {
                    format!(
                        "{}:{}:{}: E: ",
                        source_path, call_span.start.line, call_span.start.column
                    )
                };
                errors.push(format!(
                    "{}call to `{}` passes `{}` as {} simultaneous reference arguments (at least one `&mut`) — the same binding cannot be both aliased and exclusively borrowed at once",
                    loc, callee_name, var_name, count
                ));
            }
        }
    }

    // RES-4070: second increment — conditional-path-aware alias
    // tracking through `let` reference bindings.
    errors.extend(check_unannotated_let_alias(
        stmts,
        &callee_table,
        source_path,
    ));

    errors
}

// ============================================================
// A-E5 increment 2 (RES-4070): alias tracking through `let`
// reference bindings, with conditional-path awareness
// ============================================================
//
// The first A-E5 increment only catches literal syntactic repetition of
// one identifier within a single call's argument list (`f(x, x)`). This
// pass closes the next provable gap: a reference binding copied into a
// second name via `let`,
//
//     fn bump(&mut int a, &mut int b) { ... }
//     fn caller(&mut int x) {
//         let y = x;      // `y` provably refers to x's region
//         bump(x, y);     // same region behind two &mut params
//     }
//
// Soundness contract (the A-E5 "zero false positives" rule):
//
// - Alias facts are established ONLY by a straight-line `let NAME = IDENT;`
//   whose right-hand side is a plain identifier currently known to be a
//   reference (a `&`/`&mut`-typed parameter of the enclosing function,
//   or a previous alias of one). Copying a reference binding cannot do
//   anything but refer to the same region — there is no address-of or
//   re-seating expression syntax in the language today.
// - Any construct whose effect on a binding is not fully understood
//   KILLS the fact rather than guessing: assignments kill (re-seating
//   semantics not locked in), shadowing `let`s kill and detach the old
//   group, `match` arms are analysed with an empty fact set (pattern
//   bindings can shadow names invisibly), and unrecognised statement
//   forms simply aren't descended into.
// - Conditional paths merge by INTERSECTION: after `if`/`else` (and
//   after loops, which may run zero times) a fact survives only if it
//   holds on every path. A call inside a branch is checked against the
//   facts established on the path that provably reaches it — if that
//   path executes, the violation is real.
//
// Deferred (see issue #4070): Z3-backed branch-condition disjointness,
// aliasing through struct fields / array elements / closures, and
// use-after-move for plain bindings (still blocked on the Copy/Move
// default-semantics design decision — `linear.rs` remains the only
// move-semantics surface).

/// Per-path alias state for [`check_unannotated_let_alias`].
#[derive(Clone, Default)]
struct AliasState {
    /// alias name → root name. Roots are either live reference-typed
    /// parameter names or synthetic detached-group tokens.
    aliases: HashMap<String, String>,
    /// Reference-typed parameter names that are still untouched (never
    /// shadowed or reassigned) and may act as alias roots.
    live_roots: std::collections::HashSet<String>,
}

impl AliasState {
    /// Resolve `name` to its region root, if it is provably a reference.
    fn root_of<'s>(&'s self, name: &'s str) -> Option<&'s str> {
        if let Some(r) = self.aliases.get(name) {
            return Some(r.as_str());
        }
        if self.live_roots.contains(name) {
            return Some(name);
        }
        None
    }

    /// Keep only facts that hold in both `self` and `other`.
    fn intersect(&mut self, other: &AliasState) {
        self.aliases
            .retain(|k, v| other.aliases.get(k).map(String::as_str) == Some(v.as_str()));
        self.live_roots.retain(|r| other.live_roots.contains(r));
    }
}

struct AliasWalker<'a> {
    callee_table: &'a HashMap<&'a str, &'a [(String, String)]>,
    source_path: &'a str,
    errors: Vec<String>,
    /// Counter for synthetic detached-root tokens (contains `\u{0}` so
    /// it can never collide with a source identifier).
    detached: u32,
}

impl<'a> AliasWalker<'a> {
    /// A binding named `name` is being rebound (shadowing `let`,
    /// assignment, or loop/pattern binder). Its old alias group must
    /// survive under a token no new binding can join.
    fn kill_name(&mut self, state: &mut AliasState, name: &str) {
        state.aliases.remove(name);
        if state.live_roots.remove(name) || state.aliases.values().any(|r| r == name) {
            let fresh = format!("{name}\u{0}{}", self.detached);
            self.detached += 1;
            for root in state.aliases.values_mut() {
                if root == name {
                    *root = fresh.clone();
                }
            }
        }
    }

    fn walk_stmt(&mut self, node: &crate::Node, state: &mut AliasState) {
        match node {
            crate::Node::Block { stmts, .. } => {
                for s in stmts {
                    self.walk_stmt(s, state);
                }
            }
            crate::Node::LetStatement { name, value, .. } => {
                self.walk_expr(value, state);
                let new_root = if let crate::Node::Identifier { name: rhs, .. } = value.as_ref() {
                    state.root_of(rhs).map(str::to_owned)
                } else {
                    None
                };
                self.kill_name(state, name);
                if let Some(root) = new_root
                    && root != *name
                {
                    state.aliases.insert(name.clone(), root);
                }
            }
            crate::Node::Assignment { name, value, .. } => {
                self.walk_expr(value, state);
                // Re-seating semantics for reference bindings are not
                // locked in — kill, never re-establish.
                self.kill_name(state, name);
            }
            crate::Node::ExpressionStatement { expr, .. } => self.walk_expr(expr, state),
            crate::Node::ReturnStatement { value: Some(v), .. } => self.walk_expr(v, state),
            crate::Node::ReturnStatement { value: None, .. } => {}
            crate::Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                self.walk_expr(condition, state);
                let mut then_state = state.clone();
                self.walk_stmt(consequence, &mut then_state);
                let mut else_state = state.clone();
                if let Some(alt) = alternative {
                    self.walk_stmt(alt, &mut else_state);
                }
                *state = then_state;
                state.intersect(&else_state);
            }
            crate::Node::WhileStatement {
                condition, body, ..
            } => {
                self.walk_expr(condition, state);
                let mut body_state = state.clone();
                self.walk_stmt(body, &mut body_state);
                state.intersect(&body_state);
            }
            crate::Node::ForInStatement {
                name,
                iterable,
                body,
                ..
            } => {
                self.walk_expr(iterable, state);
                let mut body_state = state.clone();
                self.kill_name(&mut body_state, name);
                self.walk_stmt(body, &mut body_state);
                state.intersect(&body_state);
            }
            crate::Node::Match { .. } => self.walk_expr(node, state),
            // Expressions in statement position and anything not
            // recognised: treat expressions as expressions, skip the
            // rest (conservative accept — no facts, no reports).
            crate::Node::CallExpression { .. }
            | crate::Node::InfixExpression { .. }
            | crate::Node::PrefixExpression { .. } => self.walk_expr(node, state),
            _ => {}
        }
    }

    fn walk_expr(&mut self, node: &crate::Node, state: &mut AliasState) {
        match node {
            crate::Node::Match {
                scrutinee, arms, ..
            } => {
                self.walk_expr(scrutinee, state);
                // Pattern bindings can shadow outer names without a
                // `let`, so arm bodies are analysed with NO facts (they
                // can still report on same-arm `let` aliases of their
                // own — none exist without roots, so effectively arms
                // are opaque). Any name an arm might rebind is killed
                // from the fall-through state.
                for (_pat, guard, arm_body) in arms {
                    let mut arm_state = AliasState::default();
                    if let Some(g) = guard {
                        self.walk_expr(g, &mut arm_state);
                    }
                    self.walk_stmt(arm_body, &mut arm_state);
                    let mut assigned = Vec::new();
                    collect_rebound_names(arm_body, &mut assigned);
                    for n in assigned {
                        self.kill_name(state, &n);
                    }
                }
            }
            crate::Node::CallExpression {
                function,
                arguments,
                span,
            } => {
                for arg in arguments {
                    self.walk_expr(arg, state);
                }
                if let crate::Node::Identifier { name, .. } = function.as_ref() {
                    self.check_call(name, arguments, *span, state);
                }
            }
            crate::Node::InfixExpression { left, right, .. } => {
                self.walk_expr(left, state);
                self.walk_expr(right, state);
            }
            crate::Node::PrefixExpression { right, .. } => self.walk_expr(right, state),
            _ => {}
        }
    }

    fn check_call(
        &mut self,
        callee_name: &str,
        args: &[crate::Node],
        call_span: crate::span::Span,
        state: &AliasState,
    ) {
        let Some(param_types) = self.callee_table.get(callee_name) else {
            return;
        };
        if args.len() != param_types.len() {
            return; // arity mismatch — typechecker handles it
        }

        // region root → (arg names seen, any-&mut-slot flag)
        let mut by_root: HashMap<String, (Vec<&str>, bool)> = HashMap::new();
        for (arg, (ty, _)) in args.iter().zip(param_types.iter()) {
            if let crate::Node::Identifier { name, .. } = arg
                && let Some((is_mut, _label)) = region_from_type_str(ty)
                && let Some(root) = state.root_of(name)
            {
                let entry = by_root.entry(root.to_owned()).or_default();
                entry.0.push(name.as_str());
                entry.1 |= is_mut;
            }
        }

        let mut hits: Vec<(String, Vec<&str>)> = by_root
            .into_iter()
            .filter(|(_, (names, any_mut))| {
                // ≥2 reference slots sharing a root, at least one
                // `&mut`, and at least two DISTINCT identifiers — the
                // same-identifier case is already reported by the
                // syntactic pass above (no double-reporting).
                names.len() >= 2 && *any_mut && names.iter().any(|n| *n != names[0])
            })
            .map(|(root, (mut names, _))| {
                names.sort_unstable();
                names.dedup();
                (root, names)
            })
            .collect();
        hits.sort_unstable();

        for (_root, names) in hits {
            let loc = if call_span.start.line == 0 {
                "E: ".to_string()
            } else {
                format!(
                    "{}:{}:{}: E: ",
                    self.source_path, call_span.start.line, call_span.start.column
                )
            };
            self.errors.push(format!(
                "{}call to `{}` passes `{}` as simultaneous reference arguments (at least one `&mut`) — these bindings provably refer to the same region via `let` reference aliasing",
                loc,
                callee_name,
                names.join("`, `"),
            ));
        }
    }
}

/// Collect every name a subtree might rebind (via `let` or assignment),
/// so `match` fall-through state can conservatively kill them.
fn collect_rebound_names(node: &crate::Node, out: &mut Vec<String>) {
    match node {
        crate::Node::LetStatement { name, value, .. }
        | crate::Node::Assignment { name, value, .. } => {
            out.push(name.clone());
            collect_rebound_names(value, out);
        }
        crate::Node::Block { stmts, .. } => {
            for s in stmts {
                collect_rebound_names(s, out);
            }
        }
        crate::Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_rebound_names(condition, out);
            collect_rebound_names(consequence, out);
            if let Some(alt) = alternative {
                collect_rebound_names(alt, out);
            }
        }
        crate::Node::WhileStatement {
            condition, body, ..
        } => {
            collect_rebound_names(condition, out);
            collect_rebound_names(body, out);
        }
        crate::Node::ForInStatement {
            name,
            iterable,
            body,
            ..
        } => {
            out.push(name.clone());
            collect_rebound_names(iterable, out);
            collect_rebound_names(body, out);
        }
        crate::Node::Match {
            scrutinee, arms, ..
        } => {
            collect_rebound_names(scrutinee, out);
            for (_p, guard, body) in arms {
                if let Some(g) = guard {
                    collect_rebound_names(g, out);
                }
                collect_rebound_names(body, out);
            }
        }
        crate::Node::ExpressionStatement { expr, .. } => collect_rebound_names(expr, out),
        _ => {}
    }
}

/// RES-4070 (A-E5 increment 2): flag calls where two *different*
/// identifiers provably refer to the same region — established by
/// straight-line `let`-copies of reference bindings — and are passed as
/// simultaneous reference arguments with at least one `&mut` slot.
/// Conditional paths are merged by intersection; see the module-level
/// soundness contract above.
fn check_unannotated_let_alias(
    stmts: &[crate::Spanned<crate::Node>],
    callee_table: &HashMap<&str, &[(String, String)]>,
    source_path: &str,
) -> Vec<String> {
    let mut walker = AliasWalker {
        callee_table,
        source_path,
        errors: Vec::new(),
        detached: 0,
    };

    for spanned in stmts {
        let crate::Node::Function {
            parameters, body, ..
        } = &spanned.node
        else {
            continue;
        };
        let mut state = AliasState::default();
        for (ty, pname) in parameters {
            if ty.starts_with('&') {
                state.live_roots.insert(pname.clone());
            }
        }
        if state.live_roots.is_empty() {
            continue; // no reference roots — nothing can alias
        }
        walker.walk_stmt(body, &mut state);
    }

    walker.errors
}

// ============================================================
// Call-site region aliasing check (RES-395 PR D8)
// ============================================================

/// A lightweight record of a callee function's region interface.
///
/// RES-2146: borrows `type_params` and `param_types` directly from the
/// caller's AST instead of owning `Vec<String>` / `Vec<(String, String)>`
/// clones. The consumer (`check_call_site_aliasing`) only reads these
/// slices via `infer_region_subst_from_call`, which already takes them
/// as `&[…]`. The historical owned shape was forcing one
/// `type_params.clone()` plus one `parameters.clone()` (the parameter
/// list is a `Vec<(String, String)>` of `(type, name)` pairs) per
/// region-typed function in the program — every region-substitution
/// pass paid that allocator cost even though every byte of it was
/// available behind `&spanned.node`.
struct CalleeInfo<'a> {
    type_params: &'a [String],
    param_types: &'a [(String, String)],
}

/// Build a table from function name → `CalleeInfo` for all top-level
/// functions with region type params.
fn build_callee_table(stmts: &[crate::Spanned<crate::Node>]) -> HashMap<&str, CalleeInfo<'_>> {
    // RES-1760: pre-size to stmts.len() — at most one insert per
    // top-level statement (when it's a function with region type
    // params). Same shape as the pre-size series for call-graph
    // collections (RES-1742…RES-1756).
    let mut table = HashMap::with_capacity(stmts.len());
    for spanned in stmts {
        if let crate::Node::Function {
            name,
            type_params,
            parameters,
            ..
        } = &spanned.node
            && !type_params.is_empty()
        {
            // RES-2146: borrow name + slices from the AST. The lookup
            // call site below (`callee_table.get(*callee_name)`) passes
            // a `&str` and works unchanged thanks to the
            // `&str: Borrow<str>` blanket impl.
            table.insert(
                name.as_str(),
                CalleeInfo {
                    type_params,
                    param_types: parameters,
                },
            );
        }
    }
    table
}

/// Walk a node tree collecting all `Node::CallExpression` nodes whose
/// function slot is a plain `Node::Identifier`.
///
/// RES-1972: pushed entries borrow into the AST as `(&'a str, &'a [Node])`
/// instead of cloning `(String, Vec<Node>)`. The consumer
/// (`check_call_site_region_aliasing`) only reads the borrowed name
/// for a HashMap lookup and iterates the borrowed slice for the
/// region-aliasing analysis — it never mutates or moves out of either,
/// so the previous owning shape was pure overhead. Skipping the
/// `arguments.clone()` is the dominant win: each per-call-site clone
/// deep-copies the entire argument-expression subtree.
fn collect_calls<'a>(node: &'a crate::Node, calls: &mut Vec<(&'a str, &'a [crate::Node])>) {
    match node {
        crate::Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let crate::Node::Identifier { name, .. } = function.as_ref() {
                calls.push((name.as_str(), arguments.as_slice()));
            }
            // Recurse into arguments even if callee isn't an identifier.
            for arg in arguments {
                collect_calls(arg, calls);
            }
        }
        crate::Node::Block { stmts, .. } => {
            for s in stmts {
                collect_calls(s, calls);
            }
        }
        crate::Node::LetStatement { value, .. } => collect_calls(value, calls),
        crate::Node::Assignment { value, .. } => collect_calls(value, calls),
        crate::Node::ReturnStatement { value: Some(v), .. } => collect_calls(v, calls),
        crate::Node::ReturnStatement { value: None, .. } => {}
        crate::Node::ExpressionStatement { expr, .. } => {
            collect_calls(expr, calls);
        }
        crate::Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_calls(condition, calls);
            collect_calls(consequence, calls);
            if let Some(alt) = alternative {
                collect_calls(alt, calls);
            }
        }
        crate::Node::WhileStatement {
            condition, body, ..
        } => {
            collect_calls(condition, calls);
            collect_calls(body, calls);
        }
        crate::Node::ForInStatement { body, .. } => collect_calls(body, calls),
        crate::Node::InfixExpression { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        crate::Node::PrefixExpression { right, .. } => collect_calls(right, calls),
        _ => {}
    }
}

/// RES-395 D8: Check for region aliasing at call sites.
///
/// For each top-level function, walks its body for call expressions.
/// When a call targets a function with region type params, extracts the
/// region label of each argument (via the caller's parameter types when
/// the argument is a plain identifier), runs `infer_region_subst_from_call`
/// to bind type params to concrete regions, and checks for aliasing.
///
/// Returns a list of diagnostic strings (format: `"path:line:col: E: …"`).
pub fn check_call_site_region_aliasing(program: &crate::Node, source_path: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let stmts = match program {
        crate::Node::Program(s) => s,
        _ => return errors,
    };

    let callee_table = build_callee_table(stmts);
    if callee_table.is_empty() {
        return errors;
    }

    for spanned in stmts {
        if let crate::Node::Function {
            parameters: caller_params,
            body,
            span: caller_span,
            ..
        } = &spanned.node
        {
            // Build name → type for the caller's parameters.
            let caller_param_types: HashMap<String, String> = caller_params
                .iter()
                .map(|(ty, name)| (name.clone(), ty.clone()))
                .collect();

            // RES-1722: pre-size with a small fixed capacity. Each
            // function body typically contains 5-20 call sites; the
            // default `Vec::new()` doubling growth from 0 paid 2-3
            // reallocations per visited fn. Same shape as the
            // RES-1716/1718/1720 pre-size series.
            // RES-1972: entries now borrow into the AST as
            // `(&str, &[Node])` instead of cloning `(String, Vec<Node>)`
            // per call site — eliminates the deep `arguments.clone()`
            // that the consumer never needed.
            let mut calls: Vec<(&str, &[crate::Node])> = Vec::with_capacity(8);
            collect_calls(body, &mut calls);

            for (callee_name, args) in &calls {
                let Some(info) = callee_table.get(*callee_name) else {
                    continue;
                };
                if args.len() != info.param_types.len() {
                    continue; // arity mismatch — typechecker handles it
                }

                // For each argument, extract the region label when the arg is
                // a simple identifier whose type is known from caller params.
                let actual_labels: Vec<Option<String>> = args
                    .iter()
                    .map(|arg| {
                        if let crate::Node::Identifier { name, .. } = arg
                            && let Some(ty) = caller_param_types.get(name)
                        {
                            return region_from_type_str(ty).and_then(|(_, lbl)| lbl);
                        }
                        None
                    })
                    .collect();

                // Build the region substitution.
                let subst = match infer_region_subst_from_call(
                    info.type_params,
                    info.param_types,
                    &actual_labels,
                ) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                // Apply substitution to callee's param region labels; check
                // for aliasing between mutable ref pairs.
                let substituted: Vec<(bool, Region)> = info
                    .param_types
                    .iter()
                    .filter_map(|(ty, _)| {
                        region_from_type_str(ty).map(|(is_mut, lbl)| {
                            let region = match lbl {
                                Some(l) => apply_region_label_subst(&l, &subst),
                                None => return (is_mut, Region::Var(RegionVar(u32::MAX))),
                            };
                            (is_mut, region)
                        })
                    })
                    .collect();

                for i in 0..substituted.len() {
                    for j in (i + 1)..substituted.len() {
                        let (i_mut, ref i_region) = substituted[i];
                        let (j_mut, ref j_region) = substituted[j];
                        if !i_mut && !j_mut {
                            continue;
                        }
                        if i_region == j_region && !matches!(i_region, Region::Var(_)) {
                            let loc = if caller_span.start.line == 0 {
                                "E: ".to_string()
                            } else {
                                format!(
                                    "{}:{}:{}: E: ",
                                    source_path, caller_span.start.line, caller_span.start.column
                                )
                            };
                            errors.push(format!(
                                "{}call to `{}` aliases mutable region `{}` via args {} and {} — callee region params must be disjoint",
                                loc,
                                callee_name,
                                match i_region {
                                    Region::Named(n) => n.as_str(),
                                    _ => "?",
                                },
                                i,
                                j
                            ));
                        }
                    }
                }
            }
        }
    }

    errors
}

// ============================================================
// Region substitution (RES-395 PR D7)
// ============================================================

/// Maps region type-param names (e.g. `"R"`, `"S"`) to concrete `Region`s.
///
/// Built at each call site by `infer_region_subst_from_call` and consumed
/// by `apply_region_label_subst` to rewrite a callee's region labels in
/// terms of the caller's concrete regions.
pub type RegionSubst = HashMap<String, Region>;

/// Apply a region substitution to a label string.
///
/// If `label` is one of the type-param names in `subst`, return the
/// substituted `Region`; otherwise treat it as a concrete `Named` label
/// and return `Region::Named(label)`.
pub fn apply_region_label_subst(label: &str, subst: &RegionSubst) -> Region {
    subst
        .get(label)
        .cloned()
        .unwrap_or_else(|| Region::Named(label.to_string()))
}

/// Infer a `RegionSubst` from the actual argument types at a call site.
///
/// Iterates over `param_types` (the callee's `(type_string, param_name)`
/// pairs) and `actual_labels` (the region label extracted from each actual
/// argument — `None` if the argument is not a reference or has no label).
/// Whenever a param type contains a region label that is one of the callee's
/// `type_params`, record `type_param_name → actual_label` in the returned
/// `RegionSubst`.
///
/// Returns `Err` on arity mismatch or if the same type param is bound to two
/// different concrete labels.
pub fn infer_region_subst_from_call(
    type_params: &[String],
    param_types: &[(String, String)],
    actual_labels: &[Option<String>],
) -> Result<RegionSubst, String> {
    if param_types.len() != actual_labels.len() {
        return Err(format!(
            "region subst arity mismatch: callee has {} params, caller provided {} labels",
            param_types.len(),
            actual_labels.len()
        ));
    }

    let param_set: std::collections::HashSet<&str> =
        type_params.iter().map(|s| s.as_str()).collect();
    let mut subst = RegionSubst::new();

    for ((ty, _pname), actual_label) in param_types.iter().zip(actual_labels.iter()) {
        if let Some((_is_mut, Some(param_label))) = region_from_type_str(ty)
            && param_set.contains(param_label.as_str())
            && let Some(actual) = actual_label
        {
            // This param's region label is a type param — bind it.
            let region = Region::Named(actual.clone());
            match subst.get(&param_label) {
                None => {
                    subst.insert(param_label.clone(), region);
                }
                Some(existing) if *existing == region => {}
                Some(existing) => {
                    return Err(format!(
                        "region param `{}` bound to both `{}` and `{}`",
                        param_label,
                        match existing {
                            Region::Named(n) => n.as_str(),
                            Region::Var(_) => "<var>",
                        },
                        actual
                    ));
                }
            }
        }
    }

    Ok(subst)
}

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_vars_are_distinct() {
        let mut table = RegionTable::new();
        let a = table.fresh();
        let b = table.fresh();
        assert_ne!(a, b);
    }

    #[test]
    fn unbound_var_resolves_to_itself() {
        let mut table = RegionTable::new();
        let v = table.fresh();
        assert_eq!(table.resolve(Region::Var(v)), Region::Var(v));
    }

    #[test]
    fn unify_var_with_named_resolves_to_named() {
        let mut table = RegionTable::new();
        let v = table.fresh();
        table
            .unify(Region::Var(v), Region::named("A"))
            .expect("unify");
        assert_eq!(
            table.resolve(Region::Var(v)),
            Region::Named("A".to_string())
        );
    }

    #[test]
    fn unify_two_vars_chains_to_named() {
        let mut table = RegionTable::new();
        let v1 = table.fresh();
        let v2 = table.fresh();
        table
            .unify(Region::Var(v1), Region::Var(v2))
            .expect("unify v1=v2");
        table
            .unify(Region::Var(v2), Region::named("B"))
            .expect("unify v2=B");
        assert_eq!(
            table.resolve(Region::Var(v1)),
            Region::Named("B".to_string())
        );
    }

    #[test]
    fn unify_two_different_named_regions_errors() {
        let mut table = RegionTable::new();
        let err = table
            .unify(Region::named("X"), Region::named("Y"))
            .unwrap_err();
        assert!(
            err.contains("X") && err.contains("Y"),
            "error should mention both labels: {err}"
        );
    }

    #[test]
    fn unify_same_named_region_is_ok() {
        let mut table = RegionTable::new();
        table
            .unify(Region::named("Z"), Region::named("Z"))
            .expect("same-label unify should succeed");
    }

    #[test]
    fn build_region_map_assigns_vars_to_unlabeled_params() {
        let src = "region A; fn f(&mut[A] int a, &mut int b, int c) {}";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);

        let map = build_region_map(&program);
        let key_a = ParamKey {
            fn_name: "f".to_string(),
            param_idx: 0,
        };
        let key_b = ParamKey {
            fn_name: "f".to_string(),
            param_idx: 1,
        };
        let key_c = ParamKey {
            fn_name: "f".to_string(),
            param_idx: 2,
        };

        // Labeled param → Named region.
        assert_eq!(
            map.get_resolved(&key_a),
            Some(Region::named("A")),
            "labeled param should resolve to Named"
        );
        // Unlabeled ref param → Var (resolved to itself when unbound).
        assert!(
            matches!(map.get_resolved(&key_b), Some(Region::Var(_))),
            "unlabeled ref param should get a RegionVar"
        );
        // Non-ref param → not in map.
        assert_eq!(map.entries.get(&key_c), None, "non-ref param not in map");
    }

    // --- RES-395 D7: region substitution ---

    #[test]
    fn apply_region_label_subst_maps_param_name() {
        let mut subst = RegionSubst::new();
        subst.insert("R".to_string(), Region::named("A"));
        assert_eq!(
            apply_region_label_subst("R", &subst),
            Region::Named("A".to_string())
        );
    }

    #[test]
    fn apply_region_label_subst_passthrough_for_concrete() {
        let subst = RegionSubst::new();
        // A label not in the subst is returned as a Named region.
        assert_eq!(
            apply_region_label_subst("Heap", &subst),
            Region::Named("Heap".to_string())
        );
    }

    #[test]
    fn infer_region_subst_binds_single_param() {
        // fn foo<R>(&mut[R] int x) called with actual label A.
        let type_params = vec!["R".to_string()];
        let param_types = vec![("&mut[R] int".to_string(), "x".to_string())];
        let actual_labels = vec![Some("A".to_string())];
        let subst =
            infer_region_subst_from_call(&type_params, &param_types, &actual_labels).unwrap();
        assert_eq!(subst.get("R"), Some(&Region::Named("A".to_string())));
    }

    #[test]
    fn infer_region_subst_binds_two_distinct_params() {
        // fn foo<R, S>(&mut[R] int a, &mut[S] int b) called with A, B.
        let type_params = vec!["R".to_string(), "S".to_string()];
        let param_types = vec![
            ("&mut[R] int".to_string(), "a".to_string()),
            ("&mut[S] int".to_string(), "b".to_string()),
        ];
        let actual_labels = vec![Some("A".to_string()), Some("B".to_string())];
        let subst =
            infer_region_subst_from_call(&type_params, &param_types, &actual_labels).unwrap();
        assert_eq!(subst.get("R"), Some(&Region::Named("A".to_string())));
        assert_eq!(subst.get("S"), Some(&Region::Named("B".to_string())));
    }

    #[test]
    fn infer_region_subst_conflict_errors() {
        // R can't be both A and B.
        let type_params = vec!["R".to_string()];
        let param_types = vec![
            ("&mut[R] int".to_string(), "a".to_string()),
            ("&mut[R] int".to_string(), "b".to_string()),
        ];
        let actual_labels = vec![Some("A".to_string()), Some("B".to_string())];
        let err =
            infer_region_subst_from_call(&type_params, &param_types, &actual_labels).unwrap_err();
        assert!(err.contains("R"), "error should mention the param: {err}");
    }

    #[test]
    fn infer_region_subst_arity_mismatch_errors() {
        let type_params = vec!["R".to_string()];
        let param_types = vec![("&mut[R] int".to_string(), "x".to_string())];
        let actual_labels: Vec<Option<String>> = vec![];
        let err =
            infer_region_subst_from_call(&type_params, &param_types, &actual_labels).unwrap_err();
        assert!(err.contains("arity"), "error should mention arity: {err}");
    }

    // --- RES-773: local variable region inference ---

    #[test]
    fn build_region_map_collects_labeled_local_bindings() {
        let src = "region A; fn f(int x) { let y: &[A] int = 0; }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);

        let map = build_region_map(&program);
        let key = LocalKey {
            fn_name: "f".to_string(),
            var_name: "y".to_string(),
        };

        // Labeled local → Named region.
        assert_eq!(
            map.get_local_resolved(&key),
            Some(Region::named("A")),
            "labeled local should resolve to Named"
        );
    }

    #[test]
    fn build_region_map_collects_unlabeled_local_bindings() {
        let src = "fn f(int x) { let y: &mut int = 0; }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);

        let map = build_region_map(&program);
        let key = LocalKey {
            fn_name: "f".to_string(),
            var_name: "y".to_string(),
        };

        // Unlabeled local ref → Var (resolved to itself when unbound).
        assert!(
            matches!(map.get_local_resolved(&key), Some(Region::Var(_))),
            "unlabeled local ref should get a RegionVar"
        );
    }

    // --- A-E5: region inference for unannotated code ---

    #[test]
    fn unannotated_two_distinct_vars_to_mut_params_accepted() {
        let src = "fn set_both(&mut int a, &mut int b) {} \
                    fn caller(int x, int y) { set_both(x, y); }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let errors = check_unannotated_mut_alias(&program, "<test>");
        assert!(
            errors.is_empty(),
            "distinct vars passed to distinct &mut params should be accepted, got: {:?}",
            errors
        );
    }

    #[test]
    fn unannotated_same_var_to_two_mut_params_rejected() {
        // Genuine simultaneous mutable alias: `x` is passed to both
        // `&mut` parameters of the same non-generic call — no region
        // label is needed to know this aliases, it's syntactic
        // identity within one call's argument list.
        let src = "fn set_both(&mut int a, &mut int b) {} \
                    fn caller(int x) { set_both(x, x); }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let errors = check_unannotated_mut_alias(&program, "<test>");
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("call to `set_both`") && errors[0].contains("`x`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn unannotated_same_var_to_two_shared_refs_accepted() {
        // Two shared (`&`, non-mut) refs to the same binding cannot
        // conflict — no write is possible through either.
        let src = "fn read_both(& int a, & int b) {} \
                    fn caller(int x) { read_both(x, x); }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let errors = check_unannotated_mut_alias(&program, "<test>");
        assert!(
            errors.is_empty(),
            "two shared refs to the same var should be fine, got: {:?}",
            errors
        );
    }

    #[test]
    fn unannotated_same_var_mixed_shared_and_mut_rejected() {
        // A shared ref and an exclusive ref to the same binding at the
        // same call is still a genuine aliasing violation.
        let src = "fn mix(& int a, &mut int b) {} \
                    fn caller(int x) { mix(x, x); }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let errors = check_unannotated_mut_alias(&program, "<test>");
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn unannotated_value_param_plus_mut_param_same_var_accepted() {
        // One slot is a plain by-value `int` (no reference at all) —
        // only one live reference exists (the `&mut` slot), so this is
        // not an aliasing violation.
        let src = "fn one_ref(int a, &mut int b) {} \
                    fn caller(int x) { one_ref(x, x); }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let errors = check_unannotated_mut_alias(&program, "<test>");
        assert!(
            errors.is_empty(),
            "by-value + single &mut on the same var is not aliasing, got: {:?}",
            errors
        );
    }

    #[test]
    fn unannotated_generic_callee_left_to_call_site_pass() {
        // Region-polymorphic callees are already covered by
        // `check_call_site_region_aliasing` via label substitution;
        // this pass skips them to avoid double-reporting the same
        // violation (see `res395_d8_call_site_same_var_twice_detected`
        // in lib.rs for the generic-callee coverage).
        let src = "region A; \
                    fn update<R, S>(&mut[R] int a, &mut[S] int b) {} \
                    fn caller(&mut[A] int x) { update(x, x); }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let errors = check_unannotated_mut_alias(&program, "<test>");
        assert!(
            errors.is_empty(),
            "generic callees are left to check_call_site_region_aliasing, got: {:?}",
            errors
        );
    }

    #[test]
    fn infer_wraps_check_unannotated_mut_alias() {
        let src = "fn set_both(&mut int a, &mut int b) {} \
                    fn caller(int x) { set_both(x, x); }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let err = infer(&program, "<test>").expect_err("should reject same-var mut alias");
        assert!(
            err.contains("call to `set_both`"),
            "infer() should surface the violation, got: {}",
            err
        );
    }

    #[test]
    fn infer_accepts_safe_unannotated_program() {
        let src = "fn set_both(&mut int a, &mut int b) {} \
                    fn caller(int x, int y) { set_both(x, y); }";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert!(
            infer(&program, "<test>").is_ok(),
            "safe program must be accepted"
        );
    }

    // --- RES-4070 (A-E5 increment 2): alias tracking through `let` ---

    fn run_alias_check(src: &str) -> Vec<String> {
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        check_unannotated_mut_alias(&program, "<test>")
    }

    #[test]
    fn let_alias_of_ref_param_rejected() {
        // `y` is a straight-line `let`-copy of the `&mut` param `x` —
        // passing both to `&mut` slots is a provable aliasing violation.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let y = x; set_both(x, y); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("set_both")
                && errors[0].contains("`x`")
                && errors[0].contains("`y`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn let_alias_chain_rejected() {
        // Transitive chain: z → y → x.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let y = x; let z = y; set_both(x, z); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn let_alias_two_copies_without_root_rejected() {
        // Both call args are copies; the root itself isn't passed.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let y = x; let z = x; set_both(y, z); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn let_alias_in_both_branches_then_call_rejected() {
        // The alias fact holds on EVERY path to the call — the
        // intersection merge keeps it, so this is provable.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int c) { \
                 let y = x; \
                 if (c > 0) { println(\"a\"); } else { println(\"b\"); } \
                 set_both(x, y); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn let_alias_call_inside_branch_rejected() {
        // The call sits inside one branch, but the path that reaches it
        // provably establishes the alias — real violation when taken.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int c) { \
                 let y = x; \
                 if (c > 0) { set_both(x, y); } \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn let_alias_only_on_one_branch_accepted() {
        // `y` aliases `x` only on the then-path; on the else-path it is
        // a fresh non-reference value. The post-if call is NOT provably
        // aliasing on all paths — conservative accept.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int c) { \
                 let y = 0; \
                 if (c > 0) { let y = x; println(\"shadow\"); } \
                 set_both(x, y); \
             }",
        );
        assert!(
            errors.is_empty(),
            "conservative accept expected: {:?}",
            errors
        );
    }

    #[test]
    fn let_alias_killed_by_reassignment_accepted() {
        // `y = 0;` re-binds y before the call — re-seating semantics
        // are not locked in, so the fact is killed, not flagged.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let y = x; y = 0; set_both(x, y); }",
        );
        assert!(errors.is_empty(), "kill-on-assign expected: {:?}", errors);
    }

    #[test]
    fn let_alias_of_shadowed_root_stays_grouped() {
        // After `let x = 5;` shadows the ref param, y and z still alias
        // each other (the ORIGINAL x region) — but neither aliases the
        // new x.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let y = x; let z = x; let x = 5; set_both(y, z); \
             }",
        );
        assert_eq!(errors.len(), 1, "detached group must persist: {:?}", errors);
        // And the shadowed x must NOT be considered aliased to y.
        let errors2 = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let y = x; let x = 5; set_both(x, y); \
             }",
        );
        assert!(
            errors2.is_empty(),
            "shadowed root must not alias old copies: {:?}",
            errors2
        );
    }

    #[test]
    fn let_copy_of_value_param_accepted() {
        // `x` is a plain by-value `int` — copying it creates a new
        // value, not an alias. Nothing to flag.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(int x) { let y = x; set_both(x, y); }",
        );
        assert!(errors.is_empty(), "value copies never alias: {:?}", errors);
    }

    #[test]
    fn let_alias_shared_only_slots_accepted() {
        // Two shared (`&`) slots — no `&mut` involved, no conflict.
        let errors = run_alias_check(
            "fn read_both(& int a, & int b) {} \
             fn caller(& int x) { let y = x; read_both(x, y); }",
        );
        assert!(
            errors.is_empty(),
            "shared-only aliasing is fine: {:?}",
            errors
        );
    }

    #[test]
    fn let_alias_inside_while_body_rejected_and_survives_loop_merge() {
        // Fact established inside the loop body before the call in the
        // same iteration — provable on the path that reaches the call.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int n) { \
                 let i = 0; \
                 while (i < n) { let y = x; set_both(x, y); i = i + 1; } \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn let_alias_killed_across_loop_accepted() {
        // `y` is reassigned inside the loop; after the loop (which may
        // have run), the fact must be gone.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int n) { \
                 let y = x; \
                 let i = 0; \
                 while (i < n) { y = 0; i = i + 1; } \
                 set_both(x, y); \
             }",
        );
        assert!(
            errors.is_empty(),
            "loop-killed fact must not flag: {:?}",
            errors
        );
    }

    #[test]
    fn let_alias_match_arms_are_opaque() {
        // Match arms are analysed with no facts (pattern bindings can
        // shadow silently) and rebinding inside an arm kills the fact
        // in fall-through state — both directions conservative.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int c) { \
                 let y = x; \
                 match c { 0 => { y = 0; }, _ => { println(\"n\"); } } \
                 set_both(x, y); \
             }",
        );
        assert!(
            errors.is_empty(),
            "match must kill rebound facts: {:?}",
            errors
        );
    }

    #[test]
    fn let_alias_no_double_report_with_syntactic_pass() {
        // `set_both(y, y)` is the same identifier twice — the syntactic
        // pass reports it; the let-alias pass must stay silent so the
        // program yields exactly one diagnostic.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let y = x; set_both(y, y); }",
        );
        assert_eq!(errors.len(), 1, "exactly one report expected: {:?}", errors);
        assert!(
            errors[0].contains("simultaneous reference arguments"),
            "unexpected message: {}",
            errors[0]
        );
    }
}
