// region_inference.rs
//
// RES-394 PR 1: region-variable machinery + unification table.
// RES-394 PR 2: inference pass — assigns region vars to unlabeled
//               reference parameters and walks the call graph.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

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
/// Deliberately conservative / deferred (tracked in issue #4070):
/// - Region-polymorphic callees (non-empty `type_params`) are skipped
///   here — `check_call_site_region_aliasing` already covers them via
///   region-label substitution, and skipping avoids double-reporting.
/// - Conditional paths use intersection merging and only retain direct
///   `let` copies plus the narrow direct-reference return summary below;
///   Z3-backed branch-condition disjointness is not attempted.
/// - No use-after-move detection: the language has no Copy/Move type
///   distinction outside `linear T` (see `linear.rs`), so there is no
///   sound way yet to tell whether re-reading a plain local after
///   passing it somewhere is a genuine violation or an ordinary Copy.
///   Enforcing that now would risk false positives on every ordinary
///   value type in the corpus.
/// - No general interprocedural analysis — only direct reference returns,
///   direct `Some`/`Ok`/`Err` returns with one reference payload,
///   direct tagged-enum constructor returns with unambiguous reference
///   payload paths,
///   concrete structs whose reference fields are initialized from parameters,
///   and direct tuples of reference parameters are summarized; arrays,
///   closures, and ambiguous or wrapped return paths remain opaque across
///   function boundaries.
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
            collect_calls_with_span(function, calls);
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
        crate::Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            collect_calls_with_span(target, calls);
            collect_calls_with_span(index, calls);
            collect_calls_with_span(value, calls);
        }
        crate::Node::Slice { target, lo, hi, .. } => {
            collect_calls_with_span(target, calls);
            if let Some(lo) = lo {
                collect_calls_with_span(lo, calls);
            }
            if let Some(hi) = hi {
                collect_calls_with_span(hi, calls);
            }
        }
        crate::Node::InfixExpression { left, right, .. } => {
            collect_calls_with_span(left, calls);
            collect_calls_with_span(right, calls);
        }
        crate::Node::PrefixExpression { right, .. } => collect_calls_with_span(right, calls),
        crate::Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            for clause in requires {
                collect_calls_with_span(clause, calls);
            }
            collect_calls_with_span(body, calls);
            for clause in ensures {
                collect_calls_with_span(clause, calls);
            }
            if let Some(recovers_to) = recovers_to {
                collect_calls_with_span(recovers_to, calls);
            }
        }
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

    // RES-4070: keep only an unambiguous direct-reference return summary.
    // This is deliberately narrower than general interprocedural analysis:
    // `fn expose(&mut int x) -> &mut int { return x; }` and branches whose
    // every explicit return is `x` are transparent, but mixed or wrapped
    // return paths are not. A wrapper is accepted only when it calls a
    // summary already proven in this table; the fixed point below keeps
    // recursive or otherwise ambiguous cycles opaque.
    let mut return_aliases: HashMap<&str, usize> = HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for spanned in stmts {
            if let crate::Node::Function {
                name,
                type_params,
                parameters,
                body,
                return_type,
                ..
            } = &spanned.node
                && type_params.is_empty()
                && callee_table.contains_key(name.as_str())
                && let Some(param_idx) = direct_return_alias_summary(
                    body,
                    parameters,
                    return_type.as_deref(),
                    &return_aliases,
                )
                && return_aliases.get(name.as_str()).copied() != Some(param_idx)
            {
                return_aliases.insert(name.as_str(), param_idx);
                changed = true;
            }
        }
    }

    // Keep only declared struct fields whose types are references. This
    // lets the body walker recognize `Holder { item: x }` without treating
    // ordinary value fields as aliases.
    let mut reference_fields: HashSet<(String, String)> = HashSet::new();
    for spanned in stmts {
        if let crate::Node::StructDecl { name, fields, .. } = &spanned.node {
            for (field_type, field_name) in fields {
                if region_from_type_str(field_type).is_some() {
                    reference_fields.insert((name.clone(), field_name.clone()));
                }
            }
        }
    }

    let mut struct_return_aliases: HashMap<&str, StructReturnAliasSummary> = HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for spanned in stmts {
            if let crate::Node::Function {
                name,
                type_params,
                parameters,
                body,
                return_type,
                ..
            } = &spanned.node
                && type_params.is_empty()
                && callee_table.contains_key(name.as_str())
                && let Some(summary) = struct_return_alias_summary(
                    body,
                    parameters,
                    return_type.as_deref(),
                    &reference_fields,
                    &struct_return_aliases,
                )
                && struct_return_aliases.get(name.as_str()) != Some(&summary)
            {
                struct_return_aliases.insert(name.as_str(), summary);
                changed = true;
            }
        }
    }

    let mut array_return_aliases: HashMap<&str, ArrayReturnAliasSummary> = HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for spanned in stmts {
            if let crate::Node::Function {
                name,
                type_params,
                parameters,
                body,
                return_type,
                ..
            } = &spanned.node
                && type_params.is_empty()
                && callee_table.contains_key(name.as_str())
                && let Some(summary) = array_return_alias_summary(
                    body,
                    parameters,
                    return_type.as_deref(),
                    &reference_fields,
                    &array_return_aliases,
                )
                && array_return_aliases.get(name.as_str()) != Some(&summary)
            {
                array_return_aliases.insert(name.as_str(), summary);
                changed = true;
            }
        }
    }

    let mut tuple_return_aliases: HashMap<&str, TupleReturnAliasSummary> = HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for spanned in stmts {
            if let crate::Node::Function {
                name,
                type_params,
                parameters,
                body,
                return_type,
                ..
            } = &spanned.node
                && type_params.is_empty()
                && callee_table.contains_key(name.as_str())
                && let Some(summary) = tuple_return_alias_summary(
                    body,
                    parameters,
                    return_type.as_deref(),
                    &reference_fields,
                    &tuple_return_aliases,
                    &array_return_aliases,
                )
                && tuple_return_aliases.get(name.as_str()) != Some(&summary)
            {
                tuple_return_aliases.insert(name.as_str(), summary);
                changed = true;
            }
        }
    }

    let mut option_result_return_aliases: HashMap<&str, OptionResultReturnAliasSummary> =
        HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for spanned in stmts {
            if let crate::Node::Function {
                name,
                type_params,
                parameters,
                body,
                ..
            } = &spanned.node
                && type_params.is_empty()
                && callee_table.contains_key(name.as_str())
                && let Some(summary) = option_result_return_alias_summary(body, parameters)
                && option_result_return_aliases.get(name.as_str()) != Some(&summary)
            {
                option_result_return_aliases.insert(name.as_str(), summary);
                changed = true;
            }
        }
    }

    let mut tagged_enum_return_aliases: HashMap<&str, TaggedEnumReturnAliasSummary> =
        HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for spanned in stmts {
            if let crate::Node::Function {
                name,
                type_params,
                parameters,
                body,
                ..
            } = &spanned.node
                && type_params.is_empty()
                && callee_table.contains_key(name.as_str())
                && let Some(summary) = tagged_enum_return_alias_summary(body, parameters)
                && tagged_enum_return_aliases.get(name.as_str()) != Some(&summary)
            {
                tagged_enum_return_aliases.insert(name.as_str(), summary);
                changed = true;
            }
        }
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

    let summaries = AliasSummaries {
        return_aliases: &return_aliases,
        struct_return_aliases: &struct_return_aliases,
        tuple_return_aliases: &tuple_return_aliases,
        array_return_aliases: &array_return_aliases,
        option_result_return_aliases: &option_result_return_aliases,
        tagged_enum_return_aliases: &tagged_enum_return_aliases,
    };

    // RES-4070: second increment — conditional-path-aware alias
    // tracking through `let` reference bindings.
    errors.extend(check_unannotated_let_alias(
        stmts,
        &callee_table,
        &summaries,
        &reference_fields,
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
// - Alias facts are established only by operations with one unambiguous
//   provenance: straight-line `let NAME = IDENT;` copies, direct reference
//   returns, declared reference fields initialized by concrete struct
//   literals (including nested paths), direct tagged-enum/Option/Result
//   constructors nested in concrete struct literals or transferred through
//   tuple/struct destructuring and nested variant-aware match patterns,
//   including their payload leaves through constant tuple/array places,
//   and constant-bound slices of those places,
//   direct tuple and array returns, and
//   direct `Some`/`Ok`/`Err` helper returns with one reference payload,
//   direct tagged-enum helper returns with unambiguous payload paths,
//   non-negative constant array element/slice paths, including nested arrays,
//   fields inside direct array-literal struct elements, and constant-bound
//   slices of those arrays.
//   Copying a reference
//   binding cannot do anything but refer to the same region — there is no
//   address-of or re-seating expression syntax in the language today.
// - Any construct whose effect on a binding is not fully understood
//   KILLS the fact rather than guessing: assignments kill (re-seating
//   semantics not locked in), shadowing `let`s kill and detach the old
//   group, pattern bindings kill any shadowed names in their arm, and
//   unrecognised statement forms simply aren't descended into.
// - Conditional paths merge by INTERSECTION: after `if`/`else` (and
//   after loops, which may run zero times) a fact survives only if it
//   holds on every path. A call inside a branch is checked against the
//   facts established on the path that provably reaches it — if that
//   path executes, the violation is real.
//
// Deferred (see issue #4070): Z3-backed branch-condition disjointness,
// dynamic or transformed array aliasing, and ambiguous interprocedural
// paths. The tracked summaries stay sound because they require known
// reference-typed fields, same-parameter return paths, or forwarding
// through an already-proven helper, including direct tuple and array
// returns.
// Use-after-move for plain bindings
// remains deferred by the Copy/Move default-semantics decision —
// `linear.rs` remains the only move-semantics surface.

/// Per-path alias state for [`check_unannotated_let_alias`].
#[derive(Clone, Default)]
struct AliasState {
    /// alias name → root name. Roots are either live reference-typed
    /// parameter names or synthetic detached-group tokens.
    aliases: HashMap<String, String>,
    /// Reference-typed parameter names that are still untouched (never
    /// shadowed or reassigned) and may act as alias roots.
    live_roots: std::collections::HashSet<String>,
    /// Direct constructor identity for enum-like values whose payload paths
    /// are tracked. The tag is retained only across proven whole-value lets.
    known_constructors: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructReturnAliasSummary {
    fields: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TupleReturnAliasSummary {
    elements: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArrayReturnAliasSummary {
    paths: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OptionResultReturnAliasSummary {
    constructor: String,
    paths: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaggedEnumReturnAliasSummary {
    constructor: String,
    paths: Vec<(String, usize)>,
}

struct AliasSummaries<'a> {
    return_aliases: &'a HashMap<&'a str, usize>,
    struct_return_aliases: &'a HashMap<&'a str, StructReturnAliasSummary>,
    tuple_return_aliases: &'a HashMap<&'a str, TupleReturnAliasSummary>,
    array_return_aliases: &'a HashMap<&'a str, ArrayReturnAliasSummary>,
    option_result_return_aliases: &'a HashMap<&'a str, OptionResultReturnAliasSummary>,
    tagged_enum_return_aliases: &'a HashMap<&'a str, TaggedEnumReturnAliasSummary>,
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
        self.known_constructors
            .retain(|name, constructor| other.known_constructors.get(name) == Some(constructor));
    }
}

struct AliasWalker<'a> {
    callee_table: &'a HashMap<&'a str, &'a [(String, String)]>,
    return_aliases: &'a HashMap<&'a str, usize>,
    struct_return_aliases: &'a HashMap<&'a str, StructReturnAliasSummary>,
    tuple_return_aliases: &'a HashMap<&'a str, TupleReturnAliasSummary>,
    array_return_aliases: &'a HashMap<&'a str, ArrayReturnAliasSummary>,
    option_result_return_aliases: &'a HashMap<&'a str, OptionResultReturnAliasSummary>,
    tagged_enum_return_aliases: &'a HashMap<&'a str, TaggedEnumReturnAliasSummary>,
    reference_fields: &'a HashSet<(String, String)>,
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
        let field_prefix = format!("{name}.");
        let array_prefix = format!("{name}[");
        state.known_constructors.remove(name);
        state.aliases.retain(|place, _| {
            place != name && !place.starts_with(&field_prefix) && !place.starts_with(&array_prefix)
        });
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
                let known_constructor_paths = self.known_constructor_paths(value, state);
                let new_root = self.returned_root(value, state);
                // A whole-value copy preserves every already-proven
                // canonical path below the source place, including
                // struct fields that have no dedicated root summary.
                let composite_roots = self.paths_below(value, state);
                let field_roots = self.struct_field_roots(value, state);
                let array_roots = self.array_element_roots(value, state);
                let enum_payload_roots = self.option_result_payload_roots(value, state);
                let tagged_enum_roots = self.tagged_enum_payload_roots(value, state);
                let array_return_roots = self.array_return_roots(value, state);
                let array_alias_roots = self.array_alias_roots(value, state);
                let nested_array_roots = self.nested_array_literal_roots(value, state);
                let array_field_roots = self.array_field_roots(value, state);
                let array_slice_field_roots = self.array_slice_field_roots(value, state);
                let array_tuple_roots = self.array_tuple_roots(value, state);
                let array_slice_tuple_roots = self.array_slice_tuple_roots(value, state);
                let array_slice_nested_roots = self.array_slice_nested_roots(value, state);
                let tuple_roots = self.tuple_element_roots(value, state);
                let array_option_result_roots = self.array_option_result_roots(value, state);
                let array_slice_option_result_roots =
                    self.array_slice_option_result_roots(value, state);
                let array_tagged_roots = self.array_tagged_enum_roots(value, state);
                self.kill_name(state, name);
                if let Some(root) = new_root
                    && root != *name
                {
                    state.aliases.insert(name.clone(), root);
                }
                for (path, root) in composite_roots {
                    state.aliases.insert(format!("{name}{path}"), root);
                }
                for (field, root) in field_roots {
                    state.aliases.insert(format!("{name}.{field}"), root);
                }
                for (index, root) in array_roots {
                    state.aliases.insert(format!("{name}[{index}]"), root);
                }
                for (path, root) in array_return_roots {
                    state.aliases.insert(format!("{name}{path}"), root);
                }
                for (index, path, root) in array_alias_roots {
                    let place = if path.is_empty() {
                        format!("{name}[{index}]")
                    } else if path.starts_with('[') {
                        format!("{name}[{index}]{path}")
                    } else {
                        format!("{name}[{index}].{path}")
                    };
                    state.aliases.insert(place, root);
                }
                for (path, root) in nested_array_roots {
                    state.aliases.insert(format!("{name}{path}"), root);
                }
                for (index, field, root) in array_field_roots {
                    state
                        .aliases
                        .insert(format!("{name}[{index}].{field}"), root);
                }
                for (index, field, root) in array_slice_field_roots {
                    state
                        .aliases
                        .insert(format!("{name}[{index}].{field}"), root);
                }
                for (index, path, root) in array_tuple_roots {
                    state
                        .aliases
                        .insert(format!("{name}[{index}].{path}"), root);
                }
                for (index, path, root) in array_slice_tuple_roots {
                    state
                        .aliases
                        .insert(format!("{name}[{index}].{path}"), root);
                }
                for (path, root) in array_slice_nested_roots {
                    state.aliases.insert(format!("{name}{path}"), root);
                }
                for (index, root) in tuple_roots {
                    state.aliases.insert(format!("{name}.{index}"), root);
                }
                for (index, path, root) in array_option_result_roots {
                    state
                        .aliases
                        .insert(format!("{name}[{index}].{path}"), root);
                }
                for (index, path, root) in array_slice_option_result_roots {
                    state
                        .aliases
                        .insert(format!("{name}[{index}].{path}"), root);
                }
                for (path, root) in enum_payload_roots {
                    state.aliases.insert(format!("{name}.{path}"), root);
                }
                for (path, root) in tagged_enum_roots {
                    state.aliases.insert(format!("{name}.{path}"), root);
                }
                for (index, path, root) in array_tagged_roots {
                    state
                        .aliases
                        .insert(format!("{name}[{index}].{path}"), root);
                }
                for (path, constructor) in known_constructor_paths {
                    let place = if path.is_empty() {
                        name.clone()
                    } else {
                        format!("{name}{path}")
                    };
                    state.known_constructors.insert(place, constructor);
                }
            }
            crate::Node::LetTupleDestructure { names, value, .. } => {
                self.walk_expr(value, state);
                let tuple_roots = self.tuple_element_roots(value, state);
                let constructor_paths = self.known_constructor_paths(value, state);
                for name in names {
                    self.kill_name(state, name);
                }
                for (path, root) in tuple_roots {
                    let Some((index, suffix)) = Self::tuple_path_parts(&path) else {
                        continue;
                    };
                    let Some(name) = names.get(index) else {
                        continue;
                    };
                    let place = if suffix.is_empty() {
                        name.clone()
                    } else {
                        format!("{name}{suffix}")
                    };
                    state.aliases.insert(place, root);
                }
                for (path, constructor) in constructor_paths {
                    let Some(path) = path.strip_prefix('.') else {
                        continue;
                    };
                    let Some((index, suffix)) = Self::tuple_path_parts(path) else {
                        continue;
                    };
                    let Some(name) = names.get(index) else {
                        continue;
                    };
                    let place = if suffix.is_empty() {
                        name.clone()
                    } else {
                        format!("{name}{suffix}")
                    };
                    state.known_constructors.insert(place, constructor);
                }
            }
            crate::Node::LetDestructureStruct { fields, value, .. } => {
                self.walk_expr(value, state);
                let constructor_paths = self.known_constructor_paths(value, state);
                let mut field_roots = self.struct_field_roots(value, state);
                for (path, root) in self.paths_below(value, state) {
                    let Some(path) = path.strip_prefix('.') else {
                        continue;
                    };
                    field_roots.push((path.to_owned(), root));
                }
                for (_, local) in fields {
                    self.kill_name(state, local);
                }
                for (field, local) in fields {
                    for (path, root) in &field_roots {
                        let Some(suffix) = path.strip_prefix(field) else {
                            continue;
                        };
                        if suffix.is_empty() || suffix.starts_with('.') || suffix.starts_with('[') {
                            state
                                .aliases
                                .insert(format!("{local}{suffix}"), root.clone());
                        }
                    }
                    let field_prefix = format!(".{field}");
                    for (path, constructor) in &constructor_paths {
                        let Some(suffix) = path.strip_prefix(&field_prefix) else {
                            continue;
                        };
                        if suffix.is_empty() || suffix.starts_with('.') || suffix.starts_with('[') {
                            state
                                .known_constructors
                                .insert(format!("{local}{suffix}"), constructor.clone());
                        }
                    }
                }
            }
            crate::Node::Assignment { name, value, .. } => {
                self.walk_expr(value, state);
                // Re-seating semantics for reference bindings are not
                // locked in — kill, never re-establish.
                self.kill_name(state, name);
            }
            crate::Node::ExpressionStatement { expr, .. } => self.walk_expr(expr, state),
            crate::Node::FieldAssignment { .. } => self.walk_expr(node, state),
            crate::Node::IndexAssignment {
                target,
                index,
                value,
                ..
            } => {
                self.walk_expr(target, state);
                self.walk_expr(index, state);
                self.walk_expr(value, state);
                if let Some(place) = Self::array_place(target, index) {
                    self.kill_name(state, &place);
                } else if let Some(root) = Self::array_root(target) {
                    self.kill_name(state, &root);
                }
            }
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

    /// Return known reference leaves below a struct-valued match scrutinee.
    /// Direct literals and summarized struct-return calls are collected
    /// alongside canonical paths already tracked for a named place.
    fn struct_pattern_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(String, String)> {
        let mut roots = self.struct_field_roots(value, state);
        for (path, root) in self.paths_below(value, state) {
            let Some(path) = path.strip_prefix('.') else {
                continue;
            };
            roots.push((path.to_owned(), root));
        }
        roots
    }

    /// Rebase canonical leaves selected by a concrete struct pattern onto
    /// the pattern's local bindings. OR-patterns and other shapes without a
    /// single known path remain killed by the caller.
    fn bind_struct_pattern_aliases(
        pattern: &crate::Pattern,
        prefix: &str,
        roots: &[(String, String)],
        known_constructors: &[(String, String)],
        state: &mut AliasState,
    ) {
        if let Some(constructor) = known_constructors.iter().find_map(|(path, constructor)| {
            (path.trim_start_matches('.') == prefix).then_some(constructor.as_str())
        }) && !Self::pattern_matches_constructor(pattern, constructor)
        {
            return;
        }
        match pattern {
            crate::Pattern::Identifier(name) => {
                for (path, root) in roots {
                    let Some(suffix) = path.strip_prefix(prefix) else {
                        continue;
                    };
                    if suffix.is_empty() || suffix.starts_with('.') || suffix.starts_with('[') {
                        state
                            .aliases
                            .insert(format!("{name}{suffix}"), root.clone());
                    }
                }
            }
            crate::Pattern::Bind(name, inner) => {
                let whole_value = crate::Pattern::Identifier(name.clone());
                Self::bind_struct_pattern_aliases(
                    &whole_value,
                    prefix,
                    roots,
                    known_constructors,
                    state,
                );
                Self::bind_struct_pattern_aliases(inner, prefix, roots, known_constructors, state);
            }
            crate::Pattern::Struct { fields, .. } => {
                for (field, subpattern) in fields {
                    let path = if prefix.is_empty() {
                        field.clone()
                    } else {
                        format!("{prefix}.{field}")
                    };
                    Self::bind_struct_pattern_aliases(
                        subpattern,
                        &path,
                        roots,
                        known_constructors,
                        state,
                    );
                }
            }
            crate::Pattern::TupleStruct { fields, .. } | crate::Pattern::Tuple(fields) => {
                for (index, subpattern) in fields.iter().enumerate() {
                    let path = if prefix.is_empty() {
                        index.to_string()
                    } else {
                        format!("{prefix}.{index}")
                    };
                    Self::bind_struct_pattern_aliases(
                        subpattern,
                        &path,
                        roots,
                        known_constructors,
                        state,
                    );
                }
            }
            crate::Pattern::Some(inner)
            | crate::Pattern::Ok(inner)
            | crate::Pattern::Err(inner) => {
                let path = if prefix.is_empty() {
                    "0".to_owned()
                } else {
                    format!("{prefix}.0")
                };
                Self::bind_struct_pattern_aliases(inner, &path, roots, known_constructors, state);
            }
            crate::Pattern::EnumVariant { payload, .. } => match payload {
                crate::EnumPatternPayload::Named(fields) => {
                    for (field, subpattern) in fields {
                        let path = if prefix.is_empty() {
                            field.clone()
                        } else {
                            format!("{prefix}.{field}")
                        };
                        Self::bind_struct_pattern_aliases(
                            subpattern,
                            &path,
                            roots,
                            known_constructors,
                            state,
                        );
                    }
                }
                crate::EnumPatternPayload::Tuple(fields) => {
                    for (index, subpattern) in fields.iter().enumerate() {
                        let path = if prefix.is_empty() {
                            index.to_string()
                        } else {
                            format!("{prefix}.{index}")
                        };
                        Self::bind_struct_pattern_aliases(
                            subpattern,
                            &path,
                            roots,
                            known_constructors,
                            state,
                        );
                    }
                }
                crate::EnumPatternPayload::None => {}
            },
            crate::Pattern::Literal(_)
            | crate::Pattern::Wildcard
            | crate::Pattern::Or(_)
            | crate::Pattern::Range { .. }
            | crate::Pattern::None => {}
        }
    }

    /// Return the known reference payload of a direct Option or Result
    /// constructor. The wrapper payload is represented as path "0", matching
    /// the positional shape used by tuple and tuple-struct patterns.
    fn option_result_payload_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(String, String)> {
        let crate::Node::CallExpression {
            function,
            arguments,
            ..
        } = value
        else {
            return Vec::new();
        };
        let crate::Node::Identifier { name, .. } = function.as_ref() else {
            return Vec::new();
        };
        if matches!(name.as_str(), "Some" | "Ok" | "Err") {
            let Some(payload) = arguments.first() else {
                return Vec::new();
            };
            let mut roots = Vec::new();
            if let Some(root) = self.returned_root(payload, state) {
                roots.push(("0".to_owned(), root));
            }
            for (path, root) in self.paths_below(payload, state) {
                roots.push((format!("0{path}"), root));
            }
            return roots;
        }

        let Some(summary) = self.option_result_return_aliases.get(name.as_str()) else {
            return Vec::new();
        };
        summary
            .paths
            .iter()
            .filter_map(|(path, parameter_idx)| {
                self.tracked_reference_argument_root(arguments.get(*parameter_idx)?, state)
                    .map(|root| (path.clone(), root))
            })
            .collect()
    }

    /// Return known reference leaves carried by a concrete tagged-enum
    /// constructor. Qualified call names denote tuple payloads; qualified
    /// struct literals denote named payloads. Unqualified or transformed
    /// expressions remain opaque.
    fn tagged_enum_payload_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(String, String)> {
        match value {
            crate::Node::StructLiteral { name, fields, .. } if name.contains("::") => {
                let mut roots = Vec::new();
                for (field, payload) in fields {
                    if let Some(root) = self.returned_root(payload, state) {
                        roots.push((field.clone(), root));
                    }
                    for (path, root) in self.paths_below(payload, state) {
                        roots.push((format!("{field}{path}"), root));
                    }
                    for (path, root) in self.struct_field_roots(payload, state) {
                        roots.push((format!("{field}.{path}"), root));
                    }
                    for (path, root) in self.tuple_element_roots(payload, state) {
                        roots.push((format!("{field}.{path}"), root));
                    }
                }
                roots
            }
            crate::Node::CallExpression {
                function,
                arguments,
                ..
            } => {
                let crate::Node::Identifier { name, .. } = function.as_ref() else {
                    return Vec::new();
                };
                if name.contains("::") {
                    let mut roots = Vec::new();
                    for (index, payload) in arguments.iter().enumerate() {
                        if let Some(root) = self.returned_root(payload, state) {
                            roots.push((index.to_string(), root));
                        }
                        for (path, root) in self.paths_below(payload, state) {
                            roots.push((format!("{index}{path}"), root));
                        }
                        for (path, root) in self.struct_field_roots(payload, state) {
                            roots.push((format!("{index}.{path}"), root));
                        }
                        for (path, root) in self.tuple_element_roots(payload, state) {
                            roots.push((format!("{index}.{path}"), root));
                        }
                    }
                    return roots;
                }
                let Some(summary) = self.tagged_enum_return_aliases.get(name.as_str()) else {
                    return Vec::new();
                };
                summary
                    .paths
                    .iter()
                    .filter_map(|(path, parameter_idx)| {
                        self.tracked_reference_argument_root(arguments.get(*parameter_idx)?, state)
                            .map(|root| (path.clone(), root))
                    })
                    .collect()
            }
            _ => {
                let Some(_constructor) = self.known_constructor_for(value, state) else {
                    return Vec::new();
                };
                let Some(source) = Self::place_name(value) else {
                    return Vec::new();
                };
                let prefix = format!("{source}.");
                state
                    .aliases
                    .iter()
                    .filter_map(|(place, root)| {
                        place
                            .strip_prefix(&prefix)
                            .filter(|suffix| Self::valid_path_suffix(&format!(".{suffix}")))
                            .map(|suffix| (suffix.to_owned(), root.clone()))
                    })
                    .collect()
            }
        }
    }

    /// Resolve the region carried by a value expression when the pass can
    /// prove that it is a reference alias. Direct identifiers are the
    /// existing local-copy rule. Calls are accepted only through narrow
    /// summaries whose reference arguments are tracked canonical places.
    fn tracked_reference_argument_root(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Option<String> {
        match value {
            crate::Node::Identifier { name, .. } => state.root_of(name).map(str::to_owned),
            crate::Node::FieldAccess { .. }
            | crate::Node::IndexExpression { .. }
            | crate::Node::TupleIndex { .. } => {
                let place = Self::place_name(value)?;
                state.aliases.get(&place).cloned()
            }
            _ => None,
        }
    }

    fn returned_root(&self, value: &crate::Node, state: &AliasState) -> Option<String> {
        match value {
            crate::Node::Identifier { name, .. } => state.root_of(name).map(str::to_owned),
            crate::Node::FieldAccess { target, field, .. } => {
                if let Some(place) = Self::field_place(target, field) {
                    return state.aliases.get(&place).cloned();
                }
                self.struct_field_roots(target, state)
                    .into_iter()
                    .find_map(|(path, root)| (path == *field).then_some(root))
            }
            crate::Node::IndexExpression { target, index, .. } => {
                let place = Self::array_place(target, index)?;
                state.aliases.get(&place).cloned()
            }
            crate::Node::TupleIndex { tuple, index, .. } => {
                if let Some(item) = Self::direct_tuple_item(tuple, *index) {
                    return self.returned_root(item, state);
                }
                if let Some(root) = self.tuple_call_element_root(tuple, *index, state) {
                    return Some(root);
                }
                let place = Self::tuple_place(tuple, *index)?;
                state.aliases.get(&place).cloned()
            }
            crate::Node::CallExpression {
                function,
                arguments,
                ..
            } => {
                let crate::Node::Identifier { name: callee, .. } = function.as_ref() else {
                    return None;
                };
                let param_idx = *self.return_aliases.get(callee.as_str())?;
                self.tracked_reference_argument_root(arguments.get(param_idx)?, state)
            }
            _ => None,
        }
    }

    /// Return a canonical place for the field paths this pass understands.
    /// Nested place expressions are retained when each step has a stable
    /// source-level name or constant index. Computed expressions remain
    /// opaque rather than guessing their identity.
    fn field_place(target: &crate::Node, field: &str) -> Option<String> {
        Some(format!("{}.{field}", Self::place_name(target)?))
    }

    /// Return a canonical path for an array element when every index in the
    /// path is a non-negative integer literal. Dynamic and negative indices
    /// stay opaque because the runtime resolves them from the current array.
    fn array_place(target: &crate::Node, index: &crate::Node) -> Option<String> {
        let prefix = Self::array_root(target)?;
        let crate::Node::IntegerLiteral { value, .. } = index else {
            return None;
        };
        (*value >= 0).then(|| format!("{prefix}[{value}]"))
    }

    fn array_root(node: &crate::Node) -> Option<String> {
        Self::place_name(node)
    }

    fn tuple_place(tuple: &crate::Node, index: usize) -> Option<String> {
        Some(format!("{}.{index}", Self::place_name(tuple)?))
    }

    fn tuple_path_parts(path: &str) -> Option<(usize, &str)> {
        let boundary = path.find(['.', '[']).unwrap_or(path.len());
        let index = path[..boundary].parse().ok()?;
        let suffix = &path[boundary..];
        (suffix.is_empty() || Self::valid_path_suffix(suffix)).then_some((index, suffix))
    }

    fn place_name(node: &crate::Node) -> Option<String> {
        match node {
            crate::Node::Identifier { name, .. } => Some(name.clone()),
            crate::Node::FieldAccess { target, field, .. } => Self::field_place(target, field),
            crate::Node::IndexExpression { target, index, .. } => Self::array_place(target, index),
            crate::Node::TupleIndex { tuple, index, .. } => Self::tuple_place(tuple, *index),
            _ => None,
        }
    }

    /// Return the statically selected item from a tuple expression when the
    /// complete tuple path is made of direct literals and constant indices.
    /// This is intentionally syntax-only: an unknown tuple value remains
    /// opaque and is handled by the canonical alias-place lookup instead.
    fn direct_tuple_item(node: &crate::Node, index: usize) -> Option<&crate::Node> {
        match node {
            crate::Node::TupleLiteral { items, .. } => items.get(index),
            crate::Node::TupleIndex {
                tuple,
                index: nested_index,
                ..
            } => {
                let nested = Self::direct_tuple_item(tuple, *nested_index)?;
                Self::direct_tuple_item(nested, index)
            }
            _ => None,
        }
    }

    fn tuple_element_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(String, String)> {
        let mut roots = Vec::new();
        match value {
            crate::Node::TupleLiteral { .. } => {
                self.collect_direct_tuple_roots(value, "", state, &mut roots);
            }
            crate::Node::CallExpression {
                function,
                arguments,
                ..
            } => {
                roots.extend(self.tuple_call_element_roots(function, arguments, state));
            }
            _ => self.collect_tuple_alias_roots(value, "", state, &mut roots),
        }
        roots
    }

    fn tuple_call_element_roots(
        &self,
        function: &crate::Node,
        arguments: &[crate::Node],
        state: &AliasState,
    ) -> Vec<(String, String)> {
        let crate::Node::Identifier { name: callee, .. } = function else {
            return Vec::new();
        };
        let Some(summary) = self.tuple_return_aliases.get(callee.as_str()) else {
            return Vec::new();
        };
        summary
            .elements
            .iter()
            .filter_map(|(path, param_idx)| {
                self.tracked_reference_argument_root(arguments.get(*param_idx)?, state)
                    .map(|root| (path.clone(), root))
            })
            .collect()
    }

    fn tuple_call_element_root(
        &self,
        tuple: &crate::Node,
        index: usize,
        state: &AliasState,
    ) -> Option<String> {
        let path = Self::tuple_call_path(tuple, index)?;
        let crate::Node::TupleIndex { tuple: call, .. } = tuple else {
            return self
                .tuple_call_element_roots_from_call(tuple, state)
                .into_iter()
                .find_map(|(candidate, root)| (candidate == path).then_some(root));
        };
        self.tuple_call_element_roots_from_call(Self::tuple_call_base(call)?, state)
            .into_iter()
            .find_map(|(candidate, root)| (candidate == path).then_some(root))
    }

    fn tuple_call_element_roots_from_call(
        &self,
        call: &crate::Node,
        state: &AliasState,
    ) -> Vec<(String, String)> {
        let crate::Node::CallExpression {
            function,
            arguments,
            ..
        } = call
        else {
            return Vec::new();
        };
        self.tuple_call_element_roots(function, arguments, state)
    }

    fn tuple_call_base(node: &crate::Node) -> Option<&crate::Node> {
        match node {
            crate::Node::CallExpression { .. } => Some(node),
            crate::Node::TupleIndex { tuple, .. } => Self::tuple_call_base(tuple),
            _ => None,
        }
    }

    fn tuple_call_path(node: &crate::Node, index: usize) -> Option<String> {
        match node {
            crate::Node::CallExpression { .. } => Some(index.to_string()),
            crate::Node::TupleIndex {
                tuple,
                index: nested_index,
                ..
            } => Some(format!(
                "{}.{}",
                Self::tuple_call_path(tuple, *nested_index)?,
                index
            )),
            _ => None,
        }
    }

    /// Collect reference leaves from a direct tuple literal, including
    /// nested tuple literals and tuple-valued aliases whose element paths are
    /// already known. Every emitted path consists solely of constant tuple
    /// indices, so it cannot conflate distinct runtime values.
    fn collect_direct_tuple_roots(
        &self,
        value: &crate::Node,
        prefix: &str,
        state: &AliasState,
        roots: &mut Vec<(String, String)>,
    ) {
        let crate::Node::TupleLiteral { items, .. } = value else {
            return;
        };
        for (index, item) in items.iter().enumerate() {
            let path = if prefix.is_empty() {
                index.to_string()
            } else {
                format!("{prefix}.{index}")
            };
            if let Some(root) = self.returned_root(item, state) {
                roots.push((path.clone(), root));
            }
            match item {
                crate::Node::TupleLiteral { .. } => {
                    self.collect_direct_tuple_roots(item, &path, state, roots);
                }
                crate::Node::ArrayLiteral { .. } => {
                    for (index, root) in self.array_element_roots(item, state) {
                        roots.push((format!("{path}[{index}]"), root));
                    }
                    for (index, nested_path, root) in self.array_option_result_roots(item, state) {
                        roots.push((format!("{path}[{index}].{nested_path}"), root));
                    }
                    for (index, nested_path, root) in self.array_tagged_enum_roots(item, state) {
                        roots.push((format!("{path}[{index}].{nested_path}"), root));
                    }
                    for (nested_path, root) in self.nested_array_literal_roots(item, state) {
                        roots.push((format!("{path}{nested_path}"), root));
                    }
                }
                crate::Node::StructLiteral { .. } => {
                    for (nested_field, root) in self.struct_field_roots(item, state) {
                        roots.push((format!("{path}.{nested_field}"), root));
                    }
                }
                crate::Node::CallExpression {
                    function,
                    arguments,
                    ..
                } => {
                    for (nested_path, root) in
                        self.tuple_call_element_roots(function, arguments, state)
                    {
                        roots.push((format!("{path}.{nested_path}"), root));
                    }
                    for (nested_field, root) in
                        self.struct_call_field_roots(function, arguments, state)
                    {
                        roots.push((format!("{path}.{nested_field}"), root));
                    }
                    for (nested_path, root) in self.array_return_roots(item, state) {
                        roots.push((format!("{path}{nested_path}"), root));
                    }
                    for (nested_path, root) in self.option_result_payload_roots(item, state) {
                        roots.push((format!("{path}.{nested_path}"), root));
                    }
                    for (nested_path, root) in self.tagged_enum_payload_roots(item, state) {
                        roots.push((format!("{path}.{nested_path}"), root));
                    }
                }
                crate::Node::Identifier { .. }
                | crate::Node::FieldAccess { .. }
                | crate::Node::IndexExpression { .. }
                | crate::Node::TupleIndex { .. } => {
                    for (nested_path, root) in self.paths_below(item, state) {
                        roots.push((format!("{path}{nested_path}"), root));
                    }
                }
                crate::Node::Slice { .. } => {
                    for (index, root) in self.array_element_roots(item, state) {
                        roots.push((format!("{path}[{index}]"), root));
                    }
                    for (index, field, root) in self.array_slice_field_roots(item, state) {
                        roots.push((format!("{path}[{index}].{field}"), root));
                    }
                    for (index, nested_path, root) in self.array_slice_tuple_roots(item, state) {
                        roots.push((format!("{path}[{index}].{nested_path}"), root));
                    }
                    for (index, nested_path, root) in
                        self.array_slice_option_result_roots(item, state)
                    {
                        roots.push((format!("{path}[{index}].{nested_path}"), root));
                    }
                    for (nested_path, root) in self.array_slice_nested_roots(item, state) {
                        roots.push((format!("{path}{nested_path}"), root));
                    }
                }
                _ => self.collect_tuple_alias_roots(item, &path, state, roots),
            }
        }
    }

    /// Copy known tuple paths from an existing tuple-valued place into a new
    /// tuple path. Composite suffixes remain attached to their constant
    /// tuple index so destructuring can rebase them onto the new binding.
    fn collect_tuple_alias_roots(
        &self,
        value: &crate::Node,
        prefix: &str,
        state: &AliasState,
        roots: &mut Vec<(String, String)>,
    ) {
        let Some(source) = Self::place_name(value) else {
            return;
        };
        let source_prefix = format!("{source}.");
        for (place, root) in &state.aliases {
            let Some(suffix) = place.strip_prefix(&source_prefix) else {
                continue;
            };
            if Self::tuple_path_parts(suffix).is_none() {
                continue;
            }
            let path = if prefix.is_empty() {
                suffix.to_owned()
            } else {
                format!("{prefix}.{suffix}")
            };
            roots.push((path, root.clone()));
        }
    }

    fn struct_field_roots(&self, value: &crate::Node, state: &AliasState) -> Vec<(String, String)> {
        match value {
            crate::Node::StructLiteral { name, fields, .. } => {
                let mut roots = Vec::new();
                self.collect_struct_field_roots(name, fields, "", state, &mut roots);
                roots
            }
            crate::Node::CallExpression {
                function,
                arguments,
                ..
            } => self.struct_call_field_roots(function, arguments, state),
            _ => Vec::new(),
        }
    }

    fn struct_call_field_roots(
        &self,
        function: &crate::Node,
        arguments: &[crate::Node],
        state: &AliasState,
    ) -> Vec<(String, String)> {
        let crate::Node::Identifier { name: callee, .. } = function else {
            return Vec::new();
        };
        let Some(summary) = self.struct_return_aliases.get(callee.as_str()) else {
            return Vec::new();
        };
        summary
            .fields
            .iter()
            .filter_map(|(field, param_idx)| {
                self.tracked_reference_argument_root(arguments.get(*param_idx)?, state)
                    .map(|root| (field.clone(), root))
            })
            .collect()
    }

    fn collect_struct_field_roots(
        &self,
        struct_name: &str,
        fields: &[(String, crate::Node)],
        prefix: &str,
        state: &AliasState,
        roots: &mut Vec<(String, String)>,
    ) {
        for (field, value) in fields {
            let path = if prefix.is_empty() {
                field.clone()
            } else {
                format!("{prefix}.{field}")
            };
            if self
                .reference_fields
                .contains(&(struct_name.to_string(), field.clone()))
                && let Some(root) = self.returned_root(value, state)
            {
                roots.push((path.clone(), root));
            }
            match value {
                crate::Node::StructLiteral {
                    name: nested_name,
                    fields: nested_fields,
                    ..
                } => {
                    self.collect_struct_field_roots(
                        nested_name,
                        nested_fields,
                        &path,
                        state,
                        roots,
                    );
                    for (nested_path, root) in self.tagged_enum_payload_roots(value, state) {
                        roots.push((format!("{path}.{nested_path}"), root));
                    }
                }
                crate::Node::CallExpression {
                    function,
                    arguments,
                    ..
                } => {
                    for (nested_field, root) in
                        self.struct_call_field_roots(function, arguments, state)
                    {
                        roots.push((format!("{path}.{nested_field}"), root));
                    }
                    for (nested_path, root) in self.tuple_element_roots(value, state) {
                        roots.push((format!("{path}.{nested_path}"), root));
                    }
                    for (nested_path, root) in self.array_return_roots(value, state) {
                        roots.push((format!("{path}{nested_path}"), root));
                    }
                    for (nested_path, root) in self.tagged_enum_payload_roots(value, state) {
                        roots.push((format!("{path}.{nested_path}"), root));
                    }
                    for (nested_path, root) in self.option_result_payload_roots(value, state) {
                        roots.push((format!("{path}.{nested_path}"), root));
                    }
                }
                crate::Node::TupleLiteral { .. } => {
                    for (nested_path, root) in self.tuple_element_roots(value, state) {
                        roots.push((format!("{path}.{nested_path}"), root));
                    }
                }
                crate::Node::ArrayLiteral { .. } => {
                    for (index, root) in self.array_element_roots(value, state) {
                        roots.push((format!("{path}[{index}]"), root));
                    }
                    for (index, nested_field, root) in self.array_field_roots(value, state) {
                        roots.push((format!("{path}[{index}].{nested_field}"), root));
                    }
                    for (index, nested_path, root) in self.array_tuple_roots(value, state) {
                        roots.push((format!("{path}[{index}].{nested_path}"), root));
                    }
                    for (nested_path, root) in self.nested_array_literal_roots(value, state) {
                        roots.push((format!("{path}{nested_path}"), root));
                    }
                }
                crate::Node::Identifier { .. }
                | crate::Node::FieldAccess { .. }
                | crate::Node::IndexExpression { .. }
                | crate::Node::TupleIndex { .. } => {
                    for (nested_path, root) in self.paths_below(value, state) {
                        roots.push((format!("{path}{nested_path}"), root));
                    }
                }
                crate::Node::Slice { .. } => {
                    for (index, root) in self.array_element_roots(value, state) {
                        roots.push((format!("{path}[{index}]"), root));
                    }
                    for (index, field, root) in self.array_slice_field_roots(value, state) {
                        roots.push((format!("{path}[{index}].{field}"), root));
                    }
                    for (index, nested_path, root) in self.array_slice_tuple_roots(value, state) {
                        roots.push((format!("{path}[{index}].{nested_path}"), root));
                    }
                    for (index, nested_path, root) in
                        self.array_slice_option_result_roots(value, state)
                    {
                        roots.push((format!("{path}[{index}].{nested_path}"), root));
                    }
                    for (nested_path, root) in self.array_slice_nested_roots(value, state) {
                        roots.push((format!("{path}{nested_path}"), root));
                    }
                }
                _ => {}
            }
        }
    }

    fn array_element_roots(&self, value: &crate::Node, state: &AliasState) -> Vec<(usize, String)> {
        match value {
            crate::Node::ArrayLiteral { items, .. } => items
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    self.returned_root(value, state).map(|root| (index, root))
                })
                .collect(),
            crate::Node::Slice {
                target,
                lo,
                hi,
                inclusive,
                ..
            } => self.array_slice_element_roots(
                target,
                lo.as_deref(),
                hi.as_deref(),
                *inclusive,
                state,
            ),
            _ => Vec::new(),
        }
    }

    fn array_return_roots(&self, value: &crate::Node, state: &AliasState) -> Vec<(String, String)> {
        let crate::Node::CallExpression {
            function,
            arguments,
            ..
        } = value
        else {
            return Vec::new();
        };
        let crate::Node::Identifier { name: callee, .. } = function.as_ref() else {
            return Vec::new();
        };
        let Some(summary) = self.array_return_aliases.get(callee.as_str()) else {
            return Vec::new();
        };
        summary
            .paths
            .iter()
            .filter_map(|(path, param_idx)| {
                self.tracked_reference_argument_root(arguments.get(*param_idx)?, state)
                    .map(|root| (path.clone(), root))
            })
            .collect()
    }

    /// Copy already-proven paths from an array-valued place into a new array
    /// binding. Only constant element indices, optionally followed by known
    /// array, field, or tuple paths, are rebased.
    fn array_alias_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(usize, String, String)> {
        let Some(source) = Self::place_name(value) else {
            return Vec::new();
        };
        let prefix = format!("{source}[");
        let mut roots = Vec::new();
        for (place, root) in &state.aliases {
            let Some(rest) = place.strip_prefix(&prefix) else {
                continue;
            };
            let Some((index_text, suffix)) = rest.split_once(']') else {
                continue;
            };
            let Ok(index) = index_text.parse::<usize>() else {
                continue;
            };
            let path = if suffix.is_empty() {
                String::new()
            } else if suffix.starts_with('[') {
                if !Self::valid_array_suffix(suffix) {
                    continue;
                }
                suffix.to_owned()
            } else {
                let Some(path) = suffix.strip_prefix('.') else {
                    continue;
                };
                if path.is_empty() || path.split('.').any(str::is_empty) {
                    continue;
                }
                path.to_owned()
            };
            roots.push((index, path, root.clone()));
        }
        roots
    }

    /// Collect nested reference leaves from a direct array literal. A nested
    /// literal or an already-tracked array binding is copied only through
    /// constant paths, so a dynamic or transformed array remains opaque.
    fn nested_array_literal_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(String, String)> {
        let crate::Node::ArrayLiteral { items, .. } = value else {
            return Vec::new();
        };
        let mut roots = Vec::new();
        for (index, item) in items.iter().enumerate() {
            self.collect_nested_array_literal_roots(
                item,
                &format!("[{index}]"),
                0,
                state,
                &mut roots,
            );
        }
        roots
    }

    fn collect_nested_array_literal_roots(
        &self,
        value: &crate::Node,
        prefix: &str,
        nested_array_depth: usize,
        state: &AliasState,
        roots: &mut Vec<(String, String)>,
    ) {
        match value {
            crate::Node::ArrayLiteral { items, .. } => {
                for (index, item) in items.iter().enumerate() {
                    let path = format!("{prefix}[{index}]");
                    if let Some(root) = self.returned_root(item, state) {
                        roots.push((path.clone(), root));
                    }
                    self.collect_nested_array_literal_roots(
                        item,
                        &path,
                        nested_array_depth + 1,
                        state,
                        roots,
                    );
                }
            }
            crate::Node::StructLiteral { .. } if nested_array_depth > 0 => {
                for (field, root) in self.struct_field_roots(value, state) {
                    roots.push((format!("{prefix}.{field}"), root));
                }
            }
            crate::Node::TupleLiteral { .. } if nested_array_depth > 0 => {
                for (path, root) in self.tuple_element_roots(value, state) {
                    roots.push((format!("{prefix}.{path}"), root));
                }
            }
            crate::Node::Identifier { .. }
            | crate::Node::FieldAccess { .. }
            | crate::Node::IndexExpression { .. }
            | crate::Node::TupleIndex { .. } => {
                for (path, root) in self.paths_below(value, state) {
                    roots.push((format!("{prefix}{path}"), root));
                }
            }
            crate::Node::Slice { .. } => {
                for (index, root) in self.array_element_roots(value, state) {
                    roots.push((format!("{prefix}[{index}]"), root));
                }
                for (index, field, root) in self.array_slice_field_roots(value, state) {
                    roots.push((format!("{prefix}[{index}].{field}"), root));
                }
                for (index, nested_path, root) in self.array_slice_tuple_roots(value, state) {
                    roots.push((format!("{prefix}[{index}].{nested_path}"), root));
                }
                for (index, nested_path, root) in self.array_slice_option_result_roots(value, state)
                {
                    roots.push((format!("{prefix}[{index}].{nested_path}"), root));
                }
                for (nested_path, root) in self.array_slice_nested_roots(value, state) {
                    roots.push((format!("{prefix}{nested_path}"), root));
                }
            }
            crate::Node::CallExpression { .. } => {
                for (path, root) in self.array_return_roots(value, state) {
                    roots.push((format!("{prefix}{path}"), root));
                }
                for (path, root) in self.tuple_element_roots(value, state) {
                    roots.push((format!("{prefix}.{path}"), root));
                }
                for (field, root) in self.struct_field_roots(value, state) {
                    roots.push((format!("{prefix}.{field}"), root));
                }
            }
            _ => {}
        }
    }

    /// Return known canonical paths below a tracked place, retaining array,
    /// field, and tuple segments for rebasing below a nested array element.
    fn paths_below(&self, value: &crate::Node, state: &AliasState) -> Vec<(String, String)> {
        let Some(source) = Self::place_name(value) else {
            return Vec::new();
        };
        let mut roots = Vec::new();
        for (place, root) in &state.aliases {
            let Some(suffix) = place.strip_prefix(&source) else {
                continue;
            };
            if Self::valid_path_suffix(suffix) {
                roots.push((suffix.to_owned(), root.clone()));
            }
        }
        roots
    }

    fn valid_path_suffix(suffix: &str) -> bool {
        let mut rest = suffix;
        let mut saw_segment = false;
        while !rest.is_empty() {
            if rest.starts_with('[') {
                let Some(close) = rest.find(']') else {
                    return false;
                };
                if rest[1..close].parse::<usize>().is_err() {
                    return false;
                }
                rest = &rest[close + 1..];
            } else if let Some(field) = rest.strip_prefix('.') {
                let end = field.find(['.', '[']).unwrap_or(field.len());
                if end == 0 {
                    return false;
                }
                rest = &field[end..];
            } else {
                return false;
            }
            saw_segment = true;
        }
        saw_segment
    }

    fn valid_array_suffix(suffix: &str) -> bool {
        let mut rest = suffix;
        let mut saw_index = false;
        while rest.starts_with('[') {
            let Some(close) = rest.find(']') else {
                return false;
            };
            if rest[1..close].parse::<usize>().is_err() {
                return false;
            }
            saw_index = true;
            rest = &rest[close + 1..];
        }
        if !saw_index {
            return false;
        }
        if rest.is_empty() {
            return true;
        }
        let Some(path) = rest.strip_prefix('.') else {
            return false;
        };
        !path.is_empty() && !path.split('.').any(str::is_empty)
    }

    fn array_field_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(usize, String, String)> {
        // Only direct array literals are summarized here. Slice propagation
        // is handled separately and transformed arrays remain opaque.
        let crate::Node::ArrayLiteral { items, .. } = value else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                self.struct_field_roots(item, state)
                    .into_iter()
                    .map(move |(field, root)| (index, field, root))
            })
            .collect()
    }

    fn array_tuple_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(usize, String, String)> {
        let crate::Node::ArrayLiteral { items, .. } = value else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                self.tuple_element_roots(item, state)
                    .into_iter()
                    .map(move |(path, root)| (index, path, root))
            })
            .collect()
    }

    fn array_slice_field_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(usize, String, String)> {
        let crate::Node::Slice {
            target,
            lo,
            hi,
            inclusive,
            ..
        } = value
        else {
            return Vec::new();
        };
        let Some(source) = Self::array_root(target) else {
            return Vec::new();
        };
        let Some((start, end)) =
            Self::constant_slice_bounds(lo.as_deref(), hi.as_deref(), *inclusive)
        else {
            return Vec::new();
        };
        let prefix = format!("{source}[");
        let mut roots = Vec::new();
        for (place, root) in &state.aliases {
            let Some(rest) = place.strip_prefix(&prefix) else {
                continue;
            };
            let Some((index_text, field)) = rest.split_once("].") else {
                continue;
            };
            let Ok(index) = index_text.parse::<usize>() else {
                continue;
            };
            if index < start || end.is_some_and(|end| index >= end) {
                continue;
            }
            roots.push((index - start, field.to_string(), root.clone()));
        }
        roots
    }

    fn array_slice_tuple_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(usize, String, String)> {
        let crate::Node::Slice {
            target,
            lo,
            hi,
            inclusive,
            ..
        } = value
        else {
            return Vec::new();
        };
        let Some(source) = Self::array_root(target) else {
            return Vec::new();
        };
        let Some((start, end)) =
            Self::constant_slice_bounds(lo.as_deref(), hi.as_deref(), *inclusive)
        else {
            return Vec::new();
        };
        let prefix = format!("{source}[");
        let mut roots = Vec::new();
        for (place, root) in &state.aliases {
            let Some(rest) = place.strip_prefix(&prefix) else {
                continue;
            };
            let Some((index_text, tuple_path)) = rest.split_once("].") else {
                continue;
            };
            if tuple_path
                .split('.')
                .any(|segment| segment.parse::<usize>().is_err())
            {
                continue;
            }
            let Ok(index) = index_text.parse::<usize>() else {
                continue;
            };
            if index < start || end.is_some_and(|end| index >= end) {
                continue;
            }
            roots.push((index - start, tuple_path.to_owned(), root.clone()));
        }
        roots
    }

    fn array_slice_option_result_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(usize, String, String)> {
        let crate::Node::Slice {
            target,
            lo,
            hi,
            inclusive,
            ..
        } = value
        else {
            return Vec::new();
        };
        let Some(source) = Self::array_root(target) else {
            return Vec::new();
        };
        let Some((start, end)) =
            Self::constant_slice_bounds(lo.as_deref(), hi.as_deref(), *inclusive)
        else {
            return Vec::new();
        };
        let prefix = format!("{source}[");
        let mut roots = Vec::new();
        for (place, root) in &state.aliases {
            let Some(rest) = place.strip_prefix(&prefix) else {
                continue;
            };
            let Some((index_text, payload_path)) = rest.split_once("].") else {
                continue;
            };
            let Ok(index) = index_text.parse::<usize>() else {
                continue;
            };
            if index < start || end.is_some_and(|end| index >= end) {
                continue;
            }
            let constructor = state.known_constructors.get(&format!("{source}[{index}]"));
            if !constructor.is_some_and(|name| matches!(name.as_str(), "Some" | "Ok" | "Err")) {
                continue;
            }
            if Self::tuple_path_parts(payload_path).is_none() {
                continue;
            }
            roots.push((index - start, payload_path.to_owned(), root.clone()));
        }
        roots
    }

    fn constant_slice_bounds(
        lo: Option<&crate::Node>,
        hi: Option<&crate::Node>,
        inclusive: bool,
    ) -> Option<(usize, Option<usize>)> {
        let start = match lo {
            Some(lo) => Self::nonnegative_integer(lo)?,
            None => 0,
        };
        let end = match hi.map(Self::nonnegative_integer) {
            Some(Some(end)) => Some(if inclusive {
                end.saturating_add(1)
            } else {
                end
            }),
            Some(None) => return None,
            None => None,
        };
        Some((start, end))
    }

    fn array_slice_element_roots(
        &self,
        target: &crate::Node,
        lo: Option<&crate::Node>,
        hi: Option<&crate::Node>,
        inclusive: bool,
        state: &AliasState,
    ) -> Vec<(usize, String)> {
        let Some(source) = Self::array_root(target) else {
            return Vec::new();
        };
        let Some((start, end)) = Self::constant_slice_bounds(lo, hi, inclusive) else {
            return Vec::new();
        };
        let prefix = format!("{source}[");
        let mut roots = Vec::new();
        for (place, root) in &state.aliases {
            let Some(index_text) = place
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                continue;
            };
            let Ok(index) = index_text.parse::<usize>() else {
                continue;
            };
            if index < start || end.is_some_and(|end| index >= end) {
                continue;
            }
            roots.push((index - start, root.clone()));
        }
        roots
    }

    fn array_slice_nested_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(String, String)> {
        let crate::Node::Slice {
            target,
            lo,
            hi,
            inclusive,
            ..
        } = value
        else {
            return Vec::new();
        };
        let Some(source) = Self::array_root(target) else {
            return Vec::new();
        };
        let Some((start, end)) =
            Self::constant_slice_bounds(lo.as_deref(), hi.as_deref(), *inclusive)
        else {
            return Vec::new();
        };
        let prefix = format!("{source}[");
        let mut roots = Vec::new();
        for (place, root) in &state.aliases {
            let Some(rest) = place.strip_prefix(&prefix) else {
                continue;
            };
            let Some((index_text, suffix)) = rest.split_once(']') else {
                continue;
            };
            if !suffix.starts_with('[') || !Self::valid_array_suffix(suffix) {
                continue;
            }
            let Ok(index) = index_text.parse::<usize>() else {
                continue;
            };
            if index < start || end.is_some_and(|end| index >= end) {
                continue;
            }
            roots.push((format!("[{}]{suffix}", index - start), root.clone()));
        }
        roots
    }

    fn nonnegative_integer(node: &crate::Node) -> Option<usize> {
        let crate::Node::IntegerLiteral { value, .. } = node else {
            return None;
        };
        (*value).try_into().ok()
    }

    fn walk_expr(&mut self, node: &crate::Node, state: &mut AliasState) {
        match node {
            crate::Node::Match {
                scrutinee, arms, ..
            } => {
                self.walk_expr(scrutinee, state);
                let known_constructor_paths = self.known_constructor_paths(scrutinee, state);
                let mut scrutinee_pattern_roots = self.struct_pattern_roots(scrutinee, state);
                scrutinee_pattern_roots.extend(self.tuple_element_roots(scrutinee, state));
                scrutinee_pattern_roots.extend(self.option_result_payload_roots(scrutinee, state));
                scrutinee_pattern_roots.extend(self.tagged_enum_payload_roots(scrutinee, state));
                // Pattern bindings can shadow outer names without a
                // `let`, so remove those names from the incoming facts
                // before checking the arm. Facts established before the
                // match remain valid for every arm unless that arm
                // rebinds them.
                let mut merged_state: Option<AliasState> = None;
                for (pat, guard, arm_body) in arms {
                    let mut arm_state = state.clone();
                    let mut pattern_bindings = Vec::new();
                    collect_pattern_bindings(pat, &mut pattern_bindings);
                    pattern_bindings.sort_unstable();
                    pattern_bindings.dedup();
                    for name in pattern_bindings {
                        self.kill_name(&mut arm_state, &name);
                    }
                    Self::bind_struct_pattern_aliases(
                        pat,
                        "",
                        &scrutinee_pattern_roots,
                        &known_constructor_paths,
                        &mut arm_state,
                    );
                    if let Some(g) = guard {
                        self.walk_expr(g, &mut arm_state);
                    }
                    self.walk_stmt(arm_body, &mut arm_state);
                    let mut assigned = Vec::new();
                    collect_rebound_names(arm_body, &mut assigned);
                    assigned.sort_unstable();
                    assigned.dedup();
                    for name in assigned {
                        self.kill_name(&mut arm_state, &name);
                    }
                    if let Some(merged) = &mut merged_state {
                        merged.intersect(&arm_state);
                    } else {
                        merged_state = Some(arm_state);
                    }
                }
                if let Some(merged) = merged_state {
                    *state = merged;
                }
            }
            crate::Node::CallExpression {
                function,
                arguments,
                span,
            } => {
                self.walk_expr(function, state);
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
            crate::Node::StructLiteral { fields, base, .. } => {
                if let Some(base) = base {
                    self.walk_expr(base, state);
                }
                for (_, value) in fields {
                    self.walk_expr(value, state);
                }
            }
            crate::Node::FieldAccess { target, .. } => self.walk_expr(target, state),
            crate::Node::FieldAssignment {
                target,
                field,
                value,
                ..
            } => {
                self.walk_expr(target, state);
                self.walk_expr(value, state);
                if let Some(place) = Self::field_place(target, field) {
                    self.kill_name(state, &place);
                }
            }
            crate::Node::ArrayLiteral { items, .. } => {
                for item in items {
                    self.walk_expr(item, state);
                }
            }
            crate::Node::TupleLiteral { items, .. } => {
                for item in items {
                    self.walk_expr(item, state);
                }
            }
            crate::Node::IndexExpression { target, index, .. } => {
                self.walk_expr(target, state);
                self.walk_expr(index, state);
            }
            crate::Node::TupleIndex { tuple, .. } => self.walk_expr(tuple, state),
            crate::Node::Slice { target, lo, hi, .. } => {
                self.walk_expr(target, state);
                if let Some(lo) = lo {
                    self.walk_expr(lo, state);
                }
                if let Some(hi) = hi {
                    self.walk_expr(hi, state);
                }
            }
            crate::Node::FunctionLiteral {
                parameters,
                body,
                requires,
                ensures,
                recovers_to,
                ..
            } => {
                // Function literals capture their defining environment by
                // value. A captured reference therefore keeps the same
                // region inside the closure, while closure parameters
                // shadow captured names and establish their own roots.
                let mut closure_state = state.clone();
                for (ty, name) in parameters {
                    self.kill_name(&mut closure_state, name);
                    if ty.starts_with('&') {
                        closure_state.live_roots.insert(name.clone());
                    }
                }
                for clause in requires {
                    self.walk_expr(clause, &mut closure_state);
                }
                self.walk_stmt(body, &mut closure_state);
                for clause in ensures {
                    self.walk_expr(clause, &mut closure_state);
                }
                if let Some(recovers_to) = recovers_to {
                    self.walk_expr(recovers_to, &mut closure_state);
                }
            }
            _ => {}
        }
    }

    /// Return the constructor identity when the scrutinee is a direct
    /// tagged-enum, Option, or Result constructor. Keeping this identity
    /// alongside the payload paths prevents a known payload from leaking
    /// into a statically mismatched match arm.
    fn known_constructor_name(value: &crate::Node) -> Option<&str> {
        match value {
            crate::Node::StructLiteral { name, .. } if name.rsplit_once("::").is_some() => {
                Some(name.as_str())
            }
            crate::Node::CallExpression { function, .. } => {
                let crate::Node::Identifier { name, .. } = function.as_ref() else {
                    return None;
                };
                (matches!(name.as_str(), "Some" | "Ok" | "Err") || name.contains("::"))
                    .then_some(name.as_str())
            }
            _ => None,
        }
    }

    /// Resolve a constructor tag from a direct constructor or a whole-value
    /// alias whose tag survived the same conservative path merge as its
    /// payload facts.
    fn known_constructor_for(&self, value: &crate::Node, state: &AliasState) -> Option<String> {
        Self::known_constructor_name(value)
            .map(str::to_owned)
            .or_else(|| {
                let crate::Node::CallExpression { function, .. } = value else {
                    return None;
                };
                let crate::Node::Identifier { name, .. } = function.as_ref() else {
                    return None;
                };
                self.option_result_return_aliases
                    .get(name.as_str())
                    .map(|summary| summary.constructor.clone())
            })
            .or_else(|| {
                let crate::Node::CallExpression { function, .. } = value else {
                    return None;
                };
                let crate::Node::Identifier { name, .. } = function.as_ref() else {
                    return None;
                };
                self.tagged_enum_return_aliases
                    .get(name.as_str())
                    .map(|summary| summary.constructor.clone())
            })
            .or_else(|| {
                Self::place_name(value)
                    .and_then(|place| state.known_constructors.get(&place).cloned())
            })
    }

    /// Collect constructor tags at direct constant tuple and array paths.
    /// Transformed values and dynamic paths are intentionally omitted.
    fn known_constructor_paths(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(String, String)> {
        let mut paths = Vec::new();
        self.collect_known_constructor_paths(value, "", state, &mut paths);
        paths
    }

    fn collect_known_constructor_paths(
        &self,
        value: &crate::Node,
        prefix: &str,
        state: &AliasState,
        paths: &mut Vec<(String, String)>,
    ) {
        if let Some(constructor) = Self::known_constructor_name(value) {
            paths.push((prefix.to_owned(), constructor.to_owned()));
            return;
        }
        if let crate::Node::CallExpression { function, .. } = value
            && let crate::Node::Identifier { name, .. } = function.as_ref()
            && let Some(summary) = self.option_result_return_aliases.get(name.as_str())
        {
            paths.push((prefix.to_owned(), summary.constructor.clone()));
            return;
        }
        if let crate::Node::CallExpression { function, .. } = value
            && let crate::Node::Identifier { name, .. } = function.as_ref()
            && let Some(summary) = self.tagged_enum_return_aliases.get(name.as_str())
        {
            paths.push((prefix.to_owned(), summary.constructor.clone()));
            return;
        }
        if let Some(source) = Self::place_name(value) {
            let source_tag = state.known_constructors.get(&source).cloned();
            if let Some(constructor) = source_tag {
                paths.push((prefix.to_owned(), constructor));
            }
            for (place, constructor) in &state.known_constructors {
                let Some(suffix) = place.strip_prefix(&source) else {
                    continue;
                };
                if !suffix.is_empty() && Self::valid_path_suffix(suffix) {
                    paths.push((format!("{prefix}{suffix}"), constructor.clone()));
                }
            }
            return;
        }
        match value {
            crate::Node::TupleLiteral { items, .. } => {
                for (index, item) in items.iter().enumerate() {
                    let path = format!("{prefix}.{index}");
                    self.collect_known_constructor_paths(item, &path, state, paths);
                }
            }
            crate::Node::ArrayLiteral { items, .. } => {
                for (index, item) in items.iter().enumerate() {
                    let path = format!("{prefix}[{index}]");
                    self.collect_known_constructor_paths(item, &path, state, paths);
                }
            }
            crate::Node::StructLiteral { fields, .. } => {
                for (field, item) in fields {
                    let path = format!("{prefix}.{field}");
                    self.collect_known_constructor_paths(item, &path, state, paths);
                }
            }
            crate::Node::Slice {
                target,
                lo,
                hi,
                inclusive,
                ..
            } => {
                let Some(source) = Self::array_root(target) else {
                    return;
                };
                let Some((start, end)) =
                    Self::constant_slice_bounds(lo.as_deref(), hi.as_deref(), *inclusive)
                else {
                    return;
                };
                let source_prefix = format!("{source}[");
                for (place, constructor) in &state.known_constructors {
                    let Some(rest) = place.strip_prefix(&source_prefix) else {
                        continue;
                    };
                    let Some((index_text, suffix)) = rest.split_once(']') else {
                        continue;
                    };
                    let Ok(index) = index_text.parse::<usize>() else {
                        continue;
                    };
                    if index < start || end.is_some_and(|end| index >= end) {
                        continue;
                    }
                    if !suffix.is_empty() && !Self::valid_path_suffix(suffix) {
                        continue;
                    }
                    let path = if suffix.is_empty() {
                        format!("{prefix}[{}]", index - start)
                    } else {
                        format!("{prefix}[{}]{suffix}", index - start)
                    };
                    paths.push((path, constructor.clone()));
                }
            }
            _ => {}
        }
    }

    fn array_tagged_enum_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(usize, String, String)> {
        let crate::Node::ArrayLiteral { items, .. } = value else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                self.tagged_enum_payload_roots(item, state)
                    .into_iter()
                    .map(move |(path, root)| (index, path, root))
            })
            .collect()
    }

    fn array_option_result_roots(
        &self,
        value: &crate::Node,
        state: &AliasState,
    ) -> Vec<(usize, String, String)> {
        let crate::Node::ArrayLiteral { items, .. } = value else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                self.option_result_payload_roots(item, state)
                    .into_iter()
                    .map(move |(path, root)| (index, path, root))
            })
            .collect()
    }

    /// Check whether a pattern can select the known direct constructor. The
    /// alias binder already stays conservative for OR and opaque patterns;
    /// this additional check only prevents payload paths from a known
    /// constructor being reused by a different variant arm.
    fn pattern_matches_constructor(pattern: &crate::Pattern, constructor: &str) -> bool {
        match pattern {
            crate::Pattern::Bind(_, inner) => Self::pattern_matches_constructor(inner, constructor),
            crate::Pattern::Or(branches) => branches
                .iter()
                .any(|branch| Self::pattern_matches_constructor(branch, constructor)),
            crate::Pattern::Some(_) => constructor == "Some",
            crate::Pattern::Ok(_) => constructor == "Ok",
            crate::Pattern::Err(_) => constructor == "Err",
            crate::Pattern::EnumVariant {
                type_name,
                variant_name,
                ..
            } => {
                let Some((known_type, known_variant)) = constructor.rsplit_once("::") else {
                    return false;
                };
                known_variant == variant_name
                    && type_name
                        .as_deref()
                        .is_none_or(|pattern_type| pattern_type == known_type)
            }
            crate::Pattern::Identifier(_)
            | crate::Pattern::Literal(_)
            | crate::Pattern::Wildcard
            | crate::Pattern::Range { .. }
            | crate::Pattern::None
            | crate::Pattern::Struct { .. }
            | crate::Pattern::TupleStruct { .. }
            | crate::Pattern::Tuple(_) => false,
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

        // region root → (argument places seen, any-&mut-slot flag)
        let mut by_root: HashMap<String, (Vec<String>, bool)> = HashMap::new();
        for (arg, (ty, _)) in args.iter().zip(param_types.iter()) {
            if let Some((is_mut, _label)) = region_from_type_str(ty)
                && let Some(root) = self.returned_root(arg, state)
                && let Some(place) = Self::place_name(arg)
            {
                let entry = by_root.entry(root.to_owned()).or_default();
                entry.0.push(place);
                entry.1 |= is_mut;
            }
        }

        let mut hits: Vec<(String, Vec<String>)> = by_root
            .into_iter()
            .filter(|(_, (names, any_mut))| {
                // ≥2 reference slots sharing a root, at least one
                // `&mut`, and at least two DISTINCT identifiers — the
                // A repeated plain identifier is already reported by the
                // syntactic pass above. Repeated field places still need
                // this path because the syntactic pass only sees identifiers.
                let repeated_plain_identifier = names.len() >= 2
                    && names.iter().all(|name| name == &names[0])
                    && !names[0].contains('.');
                names.len() >= 2 && *any_mut && !repeated_plain_identifier
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
            let alias_detail = if names
                .iter()
                .all(|name| !name.contains('.') && !name.contains('['))
            {
                "these bindings provably refer to the same region via `let` reference aliasing"
            } else {
                "these places provably refer to the same region via tracked reference aliasing"
            };
            self.errors.push(format!(
                "{}call to `{}` passes `{}` as simultaneous reference arguments (at least one `&mut`) — {}",
                loc,
                callee_name,
                names.join("`, `"),
                alias_detail,
            ));
        }
    }
}

/// Collect names introduced by a match pattern. Pattern bindings are scoped
/// to one arm, so the caller kills them in that arm's alias state before
/// checking guards and the body. A binding is intentionally never inferred
/// to alias the scrutinee: the pattern's value/reference semantics are not
/// rich enough for that to be a sound conclusion here.
fn collect_pattern_bindings(pattern: &crate::Pattern, out: &mut Vec<String>) {
    match pattern {
        crate::Pattern::Identifier(name) => out.push(name.clone()),
        crate::Pattern::Bind(name, inner) => {
            out.push(name.clone());
            collect_pattern_bindings(inner, out);
        }
        crate::Pattern::Or(branches) => {
            for branch in branches {
                collect_pattern_bindings(branch, out);
            }
        }
        crate::Pattern::Struct { fields, .. } => {
            for (_, field) in fields {
                collect_pattern_bindings(field, out);
            }
        }
        crate::Pattern::Some(inner) | crate::Pattern::Ok(inner) | crate::Pattern::Err(inner) => {
            collect_pattern_bindings(inner, out)
        }
        crate::Pattern::EnumVariant { payload, .. } => match payload {
            crate::EnumPatternPayload::None => {}
            crate::EnumPatternPayload::Named(fields) => {
                for (_, field) in fields {
                    collect_pattern_bindings(field, out);
                }
            }
            crate::EnumPatternPayload::Tuple(fields) => {
                for field in fields {
                    collect_pattern_bindings(field, out);
                }
            }
        },
        crate::Pattern::TupleStruct { fields, .. } | crate::Pattern::Tuple(fields) => {
            for field in fields {
                collect_pattern_bindings(field, out);
            }
        }
        crate::Pattern::Literal(_)
        | crate::Pattern::Wildcard
        | crate::Pattern::Range { .. }
        | crate::Pattern::None => {}
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
        crate::Node::FieldAssignment {
            target,
            field,
            value,
            ..
        } => {
            if let Some(place) = AliasWalker::field_place(target, field) {
                out.push(place);
            }
            collect_rebound_names(target, out);
            collect_rebound_names(value, out);
        }
        crate::Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            if let Some(place) = AliasWalker::array_place(target, index) {
                out.push(place);
            } else if let Some(root) = AliasWalker::array_root(target) {
                out.push(root);
            }
            collect_rebound_names(target, out);
            collect_rebound_names(index, out);
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

/// RES-4070 (A-E5 increments 2–5): flag calls where two *different*
/// identifiers provably refer to the same region — established by
/// straight-line `let`-copies of reference bindings or narrow direct
/// reference-return/struct-field/tuple-return/array-return summaries — and
/// are passed as simultaneous reference arguments with at least one `&mut`
/// slot.
/// Conditional paths are merged by intersection; see the module-level
/// soundness contract above.
fn check_unannotated_let_alias(
    stmts: &[crate::Spanned<crate::Node>],
    callee_table: &HashMap<&str, &[(String, String)]>,
    summaries: &AliasSummaries<'_>,
    reference_fields: &HashSet<(String, String)>,
    source_path: &str,
) -> Vec<String> {
    let mut walker = AliasWalker {
        callee_table,
        return_aliases: summaries.return_aliases,
        struct_return_aliases: summaries.struct_return_aliases,
        tuple_return_aliases: summaries.tuple_return_aliases,
        array_return_aliases: summaries.array_return_aliases,
        option_result_return_aliases: summaries.option_result_return_aliases,
        tagged_enum_return_aliases: summaries.tagged_enum_return_aliases,
        reference_fields,
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

fn struct_return_alias_summary(
    body: &crate::Node,
    parameters: &[(String, String)],
    return_type: Option<&str>,
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, StructReturnAliasSummary>,
) -> Option<StructReturnAliasSummary> {
    let return_type = return_type?.trim();
    if return_type.is_empty()
        || return_type
            .chars()
            .any(|character| matches!(character, ' ' | '<' | '&' | '['))
    {
        return None;
    }

    let mut returns = Vec::new();
    collect_struct_return_provenance(
        body,
        parameters,
        return_type,
        reference_fields,
        known_returns,
        &mut returns,
    );
    let Some(Some(summary)) = returns.first() else {
        return None;
    };
    if returns
        .iter()
        .any(|candidate| candidate.as_ref() != Some(summary))
    {
        return None;
    }
    Some(summary.clone())
}

fn collect_struct_return_provenance(
    node: &crate::Node,
    parameters: &[(String, String)],
    return_type: &str,
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, StructReturnAliasSummary>,
    out: &mut Vec<Option<StructReturnAliasSummary>>,
) {
    match node {
        crate::Node::ReturnStatement { value, .. } => {
            out.push(value.as_deref().and_then(|value| {
                struct_return_aliases_for_value(
                    value,
                    parameters,
                    return_type,
                    reference_fields,
                    known_returns,
                )
            }));
        }
        crate::Node::Block { stmts, .. } => {
            for stmt in stmts {
                collect_struct_return_provenance(
                    stmt,
                    parameters,
                    return_type,
                    reference_fields,
                    known_returns,
                    out,
                );
            }
        }
        crate::Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_struct_return_provenance(
                consequence,
                parameters,
                return_type,
                reference_fields,
                known_returns,
                out,
            );
            if let Some(alternative) = alternative {
                collect_struct_return_provenance(
                    alternative,
                    parameters,
                    return_type,
                    reference_fields,
                    known_returns,
                    out,
                );
            }
        }
        crate::Node::WhileStatement { body, .. } | crate::Node::ForInStatement { body, .. } => {
            collect_struct_return_provenance(
                body,
                parameters,
                return_type,
                reference_fields,
                known_returns,
                out,
            )
        }
        crate::Node::Match { arms, .. } => {
            for (_pattern, _guard, body) in arms {
                collect_struct_return_provenance(
                    body,
                    parameters,
                    return_type,
                    reference_fields,
                    known_returns,
                    out,
                );
            }
        }
        crate::Node::FunctionLiteral { .. } => {}
        _ => {}
    }
}

fn struct_return_aliases_for_value(
    value: &crate::Node,
    parameters: &[(String, String)],
    return_type: &str,
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, StructReturnAliasSummary>,
) -> Option<StructReturnAliasSummary> {
    if let crate::Node::CallExpression {
        function,
        arguments,
        ..
    } = value
    {
        let crate::Node::Identifier { name: callee, .. } = function.as_ref() else {
            return None;
        };
        let summary = known_returns.get(callee.as_str())?;
        let fields = summary
            .fields
            .iter()
            .filter_map(|(field, parameter_idx)| {
                let crate::Node::Identifier { name: argument, .. } =
                    arguments.get(*parameter_idx)?
                else {
                    return None;
                };
                let outer_idx = direct_reference_parameter_name_index(argument, parameters)?;
                Some((field.clone(), outer_idx))
            })
            .collect::<Vec<_>>();
        return (!fields.is_empty()).then_some(StructReturnAliasSummary { fields });
    }

    let crate::Node::StructLiteral {
        name, fields, base, ..
    } = value
    else {
        return None;
    };
    if base.is_some() || name != return_type {
        return None;
    }

    let mut aliases = Vec::new();
    collect_struct_return_aliases(name, fields, "", parameters, reference_fields, &mut aliases)?;
    (!aliases.is_empty()).then_some(StructReturnAliasSummary { fields: aliases })
}

fn collect_struct_return_aliases(
    struct_name: &str,
    fields: &[(String, crate::Node)],
    prefix: &str,
    parameters: &[(String, String)],
    reference_fields: &HashSet<(String, String)>,
    aliases: &mut Vec<(String, usize)>,
) -> Option<()> {
    let mut reference_field_names: Vec<&str> = reference_fields
        .iter()
        .filter_map(|(candidate_name, field)| {
            (candidate_name == struct_name).then_some(field.as_str())
        })
        .collect();
    reference_field_names.sort_unstable();

    for field in reference_field_names {
        let value = fields.iter().find(|(name, _)| name == field)?.1.clone();
        let parameter_idx = direct_reference_parameter_index(&value, parameters)?;
        let path = if prefix.is_empty() {
            field.to_owned()
        } else {
            format!("{prefix}.{field}")
        };
        aliases.push((path, parameter_idx));
    }

    for (field, value) in fields {
        let crate::Node::StructLiteral {
            name: nested_name,
            fields: nested_fields,
            base,
            ..
        } = value
        else {
            continue;
        };
        if !reference_fields
            .iter()
            .any(|(candidate_name, _)| candidate_name == nested_name)
        {
            continue;
        }
        if base.is_some() {
            return None;
        }
        let nested_prefix = if prefix.is_empty() {
            field.clone()
        } else {
            format!("{prefix}.{field}")
        };
        collect_struct_return_aliases(
            nested_name,
            nested_fields,
            &nested_prefix,
            parameters,
            reference_fields,
            aliases,
        )?;
    }

    Some(())
}

fn option_result_return_alias_summary(
    body: &crate::Node,
    parameters: &[(String, String)],
) -> Option<OptionResultReturnAliasSummary> {
    let mut returns = Vec::new();
    collect_option_result_return_provenance(body, parameters, &mut returns);
    let Some(Some(summary)) = returns.first() else {
        return None;
    };
    if returns
        .iter()
        .any(|candidate| candidate.as_ref() != Some(summary))
    {
        return None;
    }
    Some(summary.clone())
}

fn collect_option_result_return_provenance(
    node: &crate::Node,
    parameters: &[(String, String)],
    out: &mut Vec<Option<OptionResultReturnAliasSummary>>,
) {
    match node {
        crate::Node::ReturnStatement { value, .. } => {
            out.push(
                value
                    .as_deref()
                    .and_then(|value| option_result_return_aliases_for_value(value, parameters)),
            );
        }
        crate::Node::Block { stmts, .. } => {
            for stmt in stmts {
                collect_option_result_return_provenance(stmt, parameters, out);
            }
        }
        crate::Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_option_result_return_provenance(consequence, parameters, out);
            if let Some(alternative) = alternative {
                collect_option_result_return_provenance(alternative, parameters, out);
            }
        }
        crate::Node::WhileStatement { body, .. } | crate::Node::ForInStatement { body, .. } => {
            collect_option_result_return_provenance(body, parameters, out)
        }
        crate::Node::Match { arms, .. } => {
            for (_pattern, _guard, body) in arms {
                collect_option_result_return_provenance(body, parameters, out);
            }
        }
        crate::Node::FunctionLiteral { .. } => {}
        _ => {}
    }
}

fn option_result_return_aliases_for_value(
    value: &crate::Node,
    parameters: &[(String, String)],
) -> Option<OptionResultReturnAliasSummary> {
    let crate::Node::CallExpression {
        function,
        arguments,
        ..
    } = value
    else {
        return None;
    };
    let crate::Node::Identifier { name, .. } = function.as_ref() else {
        return None;
    };
    if !matches!(name.as_str(), "Some" | "Ok" | "Err") {
        return None;
    }
    let payload = arguments.first()?;
    let parameter_idx = direct_reference_parameter_index(payload, parameters)?;
    Some(OptionResultReturnAliasSummary {
        constructor: name.clone(),
        paths: vec![("0".to_owned(), parameter_idx)],
    })
}

fn tagged_enum_return_alias_summary(
    body: &crate::Node,
    parameters: &[(String, String)],
) -> Option<TaggedEnumReturnAliasSummary> {
    let mut returns = Vec::new();
    collect_tagged_enum_return_provenance(body, parameters, &mut returns);
    let Some(Some(summary)) = returns.first() else {
        return None;
    };
    if returns
        .iter()
        .any(|candidate| candidate.as_ref() != Some(summary))
    {
        return None;
    }
    Some(summary.clone())
}

fn collect_tagged_enum_return_provenance(
    node: &crate::Node,
    parameters: &[(String, String)],
    out: &mut Vec<Option<TaggedEnumReturnAliasSummary>>,
) {
    match node {
        crate::Node::ReturnStatement { value, .. } => {
            out.push(
                value
                    .as_deref()
                    .and_then(|value| tagged_enum_return_aliases_for_value(value, parameters)),
            );
        }
        crate::Node::Block { stmts, .. } => {
            for stmt in stmts {
                collect_tagged_enum_return_provenance(stmt, parameters, out);
            }
        }
        crate::Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_tagged_enum_return_provenance(consequence, parameters, out);
            if let Some(alternative) = alternative {
                collect_tagged_enum_return_provenance(alternative, parameters, out);
            }
        }
        crate::Node::WhileStatement { body, .. } | crate::Node::ForInStatement { body, .. } => {
            collect_tagged_enum_return_provenance(body, parameters, out)
        }
        crate::Node::Match { arms, .. } => {
            for (_pattern, _guard, body) in arms {
                collect_tagged_enum_return_provenance(body, parameters, out);
            }
        }
        crate::Node::FunctionLiteral { .. } => {}
        _ => {}
    }
}

fn tagged_enum_return_aliases_for_value(
    value: &crate::Node,
    parameters: &[(String, String)],
) -> Option<TaggedEnumReturnAliasSummary> {
    match value {
        crate::Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let crate::Node::Identifier { name, .. } = function.as_ref() else {
                return None;
            };
            if !name.contains("::") {
                return None;
            }
            let mut paths = Vec::new();
            for (index, payload) in arguments.iter().enumerate() {
                collect_tagged_enum_return_paths(
                    payload,
                    &index.to_string(),
                    parameters,
                    &mut paths,
                );
            }
            Some(TaggedEnumReturnAliasSummary {
                constructor: name.clone(),
                paths,
            })
        }
        crate::Node::StructLiteral { name, fields, .. } if name.contains("::") => {
            let mut paths = Vec::new();
            for (field, payload) in fields {
                collect_tagged_enum_return_paths(payload, field, parameters, &mut paths);
            }
            Some(TaggedEnumReturnAliasSummary {
                constructor: name.clone(),
                paths,
            })
        }
        _ => None,
    }
}

fn collect_tagged_enum_return_paths(
    value: &crate::Node,
    prefix: &str,
    parameters: &[(String, String)],
    paths: &mut Vec<(String, usize)>,
) {
    if let Some(parameter_idx) = direct_reference_parameter_index(value, parameters) {
        paths.push((prefix.to_owned(), parameter_idx));
        return;
    }
    match value {
        crate::Node::StructLiteral { fields, .. } => {
            for (field, payload) in fields {
                collect_tagged_enum_return_paths(
                    payload,
                    &format!("{prefix}.{field}"),
                    parameters,
                    paths,
                );
            }
        }
        crate::Node::TupleLiteral { items, .. } => {
            for (index, payload) in items.iter().enumerate() {
                collect_tagged_enum_return_paths(
                    payload,
                    &format!("{prefix}.{index}"),
                    parameters,
                    paths,
                );
            }
        }
        crate::Node::ArrayLiteral { items, .. } => {
            for (index, payload) in items.iter().enumerate() {
                collect_tagged_enum_return_paths(
                    payload,
                    &format!("{prefix}[{index}]"),
                    parameters,
                    paths,
                );
            }
        }
        _ => {}
    }
}

fn tuple_return_alias_summary(
    body: &crate::Node,
    parameters: &[(String, String)],
    return_type: Option<&str>,
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, TupleReturnAliasSummary>,
    array_returns: &HashMap<&str, ArrayReturnAliasSummary>,
) -> Option<TupleReturnAliasSummary> {
    let return_type = return_type?.trim();
    if !return_type.starts_with('(') || !return_type.ends_with(')') {
        return None;
    }

    let mut returns = Vec::new();
    collect_tuple_return_provenance(
        body,
        parameters,
        reference_fields,
        known_returns,
        array_returns,
        &mut returns,
    );
    let Some(Some(summary)) = returns.first() else {
        return None;
    };
    if returns
        .iter()
        .any(|candidate| candidate.as_ref() != Some(summary))
    {
        return None;
    }
    Some(summary.clone())
}

fn collect_tuple_return_provenance(
    node: &crate::Node,
    parameters: &[(String, String)],
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, TupleReturnAliasSummary>,
    array_returns: &HashMap<&str, ArrayReturnAliasSummary>,
    out: &mut Vec<Option<TupleReturnAliasSummary>>,
) {
    match node {
        crate::Node::ReturnStatement { value, .. } => {
            out.push(value.as_deref().and_then(|value| {
                tuple_return_aliases_for_value(
                    value,
                    parameters,
                    reference_fields,
                    known_returns,
                    array_returns,
                )
            }));
        }
        crate::Node::Block { stmts, .. } => {
            for stmt in stmts {
                collect_tuple_return_provenance(
                    stmt,
                    parameters,
                    reference_fields,
                    known_returns,
                    array_returns,
                    out,
                );
            }
        }
        crate::Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_tuple_return_provenance(
                consequence,
                parameters,
                reference_fields,
                known_returns,
                array_returns,
                out,
            );
            if let Some(alternative) = alternative {
                collect_tuple_return_provenance(
                    alternative,
                    parameters,
                    reference_fields,
                    known_returns,
                    array_returns,
                    out,
                );
            }
        }
        crate::Node::WhileStatement { body, .. } | crate::Node::ForInStatement { body, .. } => {
            collect_tuple_return_provenance(
                body,
                parameters,
                reference_fields,
                known_returns,
                array_returns,
                out,
            )
        }
        crate::Node::Match { arms, .. } => {
            for (_pattern, _guard, body) in arms {
                collect_tuple_return_provenance(
                    body,
                    parameters,
                    reference_fields,
                    known_returns,
                    array_returns,
                    out,
                );
            }
        }
        crate::Node::FunctionLiteral { .. } => {}
        _ => {}
    }
}

fn tuple_return_aliases_for_value(
    value: &crate::Node,
    parameters: &[(String, String)],
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, TupleReturnAliasSummary>,
    array_returns: &HashMap<&str, ArrayReturnAliasSummary>,
) -> Option<TupleReturnAliasSummary> {
    if let crate::Node::CallExpression {
        function,
        arguments,
        ..
    } = value
    {
        let crate::Node::Identifier { name: callee, .. } = function.as_ref() else {
            return None;
        };
        let summary = known_returns.get(callee.as_str())?;
        let elements = summary
            .elements
            .iter()
            .filter_map(|(path, parameter_idx)| {
                let crate::Node::Identifier { name: argument, .. } =
                    arguments.get(*parameter_idx)?
                else {
                    return None;
                };
                let outer_idx = direct_reference_parameter_name_index(argument, parameters)?;
                Some((path.clone(), outer_idx))
            })
            .collect::<Vec<_>>();
        return (!elements.is_empty()).then_some(TupleReturnAliasSummary { elements });
    }

    let mut elements = Vec::new();
    collect_tuple_return_elements(
        value,
        "",
        parameters,
        reference_fields,
        array_returns,
        &mut elements,
    )?;
    (!elements.is_empty()).then_some(TupleReturnAliasSummary { elements })
}

fn array_return_alias_summary(
    body: &crate::Node,
    parameters: &[(String, String)],
    return_type: Option<&str>,
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, ArrayReturnAliasSummary>,
) -> Option<ArrayReturnAliasSummary> {
    let return_type = return_type?.trim();
    if return_type != "array" && !return_type.starts_with('[') && !return_type.starts_with("array<")
    {
        return None;
    }

    let mut returns = Vec::new();
    collect_array_return_provenance(
        body,
        parameters,
        reference_fields,
        known_returns,
        &mut returns,
    );
    let Some(Some(summary)) = returns.first() else {
        return None;
    };
    if returns
        .iter()
        .any(|candidate| candidate.as_ref() != Some(summary))
    {
        return None;
    }
    Some(summary.clone())
}

fn collect_array_return_provenance(
    node: &crate::Node,
    parameters: &[(String, String)],
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, ArrayReturnAliasSummary>,
    out: &mut Vec<Option<ArrayReturnAliasSummary>>,
) {
    match node {
        crate::Node::ReturnStatement { value, .. } => {
            out.push(value.as_deref().and_then(|value| {
                array_return_aliases_for_value(value, parameters, reference_fields, known_returns)
            }));
        }
        crate::Node::Block { stmts, .. } => {
            for stmt in stmts {
                collect_array_return_provenance(
                    stmt,
                    parameters,
                    reference_fields,
                    known_returns,
                    out,
                );
            }
        }
        crate::Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_array_return_provenance(
                consequence,
                parameters,
                reference_fields,
                known_returns,
                out,
            );
            if let Some(alternative) = alternative {
                collect_array_return_provenance(
                    alternative,
                    parameters,
                    reference_fields,
                    known_returns,
                    out,
                );
            }
        }
        crate::Node::WhileStatement { body, .. } | crate::Node::ForInStatement { body, .. } => {
            collect_array_return_provenance(body, parameters, reference_fields, known_returns, out)
        }
        crate::Node::Match { arms, .. } => {
            for (_pattern, _guard, body) in arms {
                collect_array_return_provenance(
                    body,
                    parameters,
                    reference_fields,
                    known_returns,
                    out,
                );
            }
        }
        crate::Node::FunctionLiteral { .. } => {}
        _ => {}
    }
}

fn array_return_aliases_for_value(
    value: &crate::Node,
    parameters: &[(String, String)],
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, ArrayReturnAliasSummary>,
) -> Option<ArrayReturnAliasSummary> {
    match value {
        crate::Node::ArrayLiteral { .. } => {
            let mut paths = Vec::new();
            collect_array_return_paths(
                value,
                "",
                parameters,
                reference_fields,
                known_returns,
                &mut paths,
            );
            (!paths.is_empty()).then_some(ArrayReturnAliasSummary { paths })
        }
        crate::Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let crate::Node::Identifier { name: callee, .. } = function.as_ref() else {
                return None;
            };
            let summary = known_returns.get(callee.as_str())?;
            let paths = summary
                .paths
                .iter()
                .filter_map(|(path, parameter_idx)| {
                    let crate::Node::Identifier { name: argument, .. } =
                        arguments.get(*parameter_idx)?
                    else {
                        return None;
                    };
                    let outer_idx =
                        parameters
                            .iter()
                            .enumerate()
                            .find_map(|(idx, (ty, parameter))| {
                                (parameter == argument && region_from_type_str(ty).is_some())
                                    .then_some(idx)
                            })?;
                    Some((path.clone(), outer_idx))
                })
                .collect::<Vec<_>>();
            (!paths.is_empty()).then_some(ArrayReturnAliasSummary { paths })
        }
        _ => None,
    }
}

fn collect_array_return_paths(
    value: &crate::Node,
    prefix: &str,
    parameters: &[(String, String)],
    reference_fields: &HashSet<(String, String)>,
    known_returns: &HashMap<&str, ArrayReturnAliasSummary>,
    paths: &mut Vec<(String, usize)>,
) {
    match value {
        crate::Node::ArrayLiteral { items, .. } | crate::Node::TupleLiteral { items, .. } => {
            for (index, item) in items.iter().enumerate() {
                let path = if matches!(value, crate::Node::ArrayLiteral { .. }) {
                    format!("{prefix}[{index}]")
                } else if prefix.is_empty() {
                    format!(".{index}")
                } else {
                    format!("{prefix}.{index}")
                };
                collect_array_return_paths(
                    item,
                    &path,
                    parameters,
                    reference_fields,
                    known_returns,
                    paths,
                );
            }
        }
        crate::Node::StructLiteral {
            name, fields, base, ..
        } => {
            if base.is_some() {
                return;
            }
            for (field, item) in fields {
                let path = format!("{prefix}.{field}");
                if reference_fields.contains(&(name.clone(), field.clone()))
                    && let Some(param_idx) = direct_reference_parameter_index(item, parameters)
                {
                    paths.push((path.clone(), param_idx));
                }
                if matches!(item, crate::Node::StructLiteral { .. }) {
                    collect_array_return_paths(
                        item,
                        &path,
                        parameters,
                        reference_fields,
                        known_returns,
                        paths,
                    );
                }
            }
        }
        crate::Node::CallExpression { .. } => {
            let _ =
                collect_array_return_call_paths(value, prefix, parameters, known_returns, paths);
        }
        crate::Node::Identifier { .. } => {
            if let Some(param_idx) = direct_reference_parameter_index(value, parameters) {
                paths.push((prefix.to_owned(), param_idx));
            }
        }
        _ => {}
    }
}

fn collect_array_return_call_paths(
    value: &crate::Node,
    prefix: &str,
    parameters: &[(String, String)],
    known_returns: &HashMap<&str, ArrayReturnAliasSummary>,
    elements: &mut Vec<(String, usize)>,
) -> Option<()> {
    let crate::Node::CallExpression {
        function,
        arguments,
        ..
    } = value
    else {
        return None;
    };
    let crate::Node::Identifier { name: callee, .. } = function.as_ref() else {
        return None;
    };
    let summary = known_returns.get(callee.as_str())?;
    let mut mapped = Vec::new();
    for (path, parameter_idx) in &summary.paths {
        let crate::Node::Identifier { name: argument, .. } = arguments.get(*parameter_idx)? else {
            return None;
        };
        let outer_idx = direct_reference_parameter_name_index(argument, parameters)?;
        mapped.push((format!("{prefix}{path}"), outer_idx));
    }
    elements.extend(mapped);
    Some(())
}

fn direct_reference_parameter_index(
    value: &crate::Node,
    parameters: &[(String, String)],
) -> Option<usize> {
    let crate::Node::Identifier { name, .. } = value else {
        return None;
    };
    direct_reference_parameter_name_index(name, parameters)
}

fn direct_reference_parameter_name_index(
    name: &str,
    parameters: &[(String, String)],
) -> Option<usize> {
    parameters
        .iter()
        .enumerate()
        .find_map(|(index, (ty, parameter))| {
            (parameter == name && region_from_type_str(ty).is_some()).then_some(index)
        })
}

fn collect_tuple_return_elements(
    value: &crate::Node,
    prefix: &str,
    parameters: &[(String, String)],
    reference_fields: &HashSet<(String, String)>,
    array_returns: &HashMap<&str, ArrayReturnAliasSummary>,
    elements: &mut Vec<(String, usize)>,
) -> Option<()> {
    let crate::Node::TupleLiteral { items, .. } = value else {
        return None;
    };
    for (index, item) in items.iter().enumerate() {
        let path = if prefix.is_empty() {
            index.to_string()
        } else {
            format!("{prefix}.{index}")
        };
        match item {
            crate::Node::TupleLiteral { .. } => {
                collect_tuple_return_elements(
                    item,
                    &path,
                    parameters,
                    reference_fields,
                    array_returns,
                    elements,
                )?;
            }
            crate::Node::StructLiteral {
                name, fields, base, ..
            } => {
                if base.is_some() {
                    return None;
                }
                collect_struct_return_aliases(
                    name,
                    fields,
                    &path,
                    parameters,
                    reference_fields,
                    elements,
                )?;
            }
            crate::Node::ArrayLiteral { .. } => {
                collect_array_return_paths(
                    item,
                    &path,
                    parameters,
                    reference_fields,
                    array_returns,
                    elements,
                );
            }
            crate::Node::CallExpression { .. } => {
                collect_array_return_call_paths(item, &path, parameters, array_returns, elements)?;
            }
            crate::Node::Identifier { name, .. } => {
                let parameter_idx =
                    parameters
                        .iter()
                        .enumerate()
                        .find_map(|(idx, (ty, parameter))| {
                            (parameter == name && region_from_type_str(ty).is_some()).then_some(idx)
                        })?;
                elements.push((path, parameter_idx));
            }
            _ => return None,
        }
    }
    Some(())
}

/// Resolve a return expression to one of the enclosing function's reference
/// parameters. A direct identifier is the base case; a call is accepted only
/// when its callee already has a summary and the corresponding argument is a
/// plain enclosing parameter.
fn return_alias_param_index(
    value: &crate::Node,
    parameters: &[(String, String)],
    known_returns: &HashMap<&str, usize>,
) -> Option<usize> {
    let parameter_index = |name: &str| {
        parameters
            .iter()
            .enumerate()
            .find(|(_, (_, parameter_name))| parameter_name == name)
            .and_then(|(idx, (ty, _))| region_from_type_str(ty).map(|_| idx))
    };

    match value {
        crate::Node::Identifier { name, .. } => parameter_index(name),
        crate::Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let crate::Node::Identifier { name: callee, .. } = function.as_ref() else {
                return None;
            };
            let callee_param_idx = *known_returns.get(callee.as_str())?;
            let crate::Node::Identifier { name: argument, .. } = arguments.get(callee_param_idx)?
            else {
                return None;
            };
            parameter_index(argument)
        }
        _ => None,
    }
}

/// Collect explicit return provenance without descending into nested
/// function literals. `None` marks a bare, wrapped, or otherwise unknown
/// return, all of which make the enclosing summary ineligible.
fn collect_return_provenance(
    node: &crate::Node,
    parameters: &[(String, String)],
    known_returns: &HashMap<&str, usize>,
    out: &mut Vec<Option<usize>>,
) {
    match node {
        crate::Node::ReturnStatement { value, .. } => {
            let provenance = value
                .as_deref()
                .and_then(|value| return_alias_param_index(value, parameters, known_returns));
            out.push(provenance);
        }
        crate::Node::Block { stmts, .. } => {
            for stmt in stmts {
                collect_return_provenance(stmt, parameters, known_returns, out);
            }
        }
        crate::Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_return_provenance(consequence, parameters, known_returns, out);
            if let Some(alternative) = alternative {
                collect_return_provenance(alternative, parameters, known_returns, out);
            }
        }
        crate::Node::WhileStatement { body, .. } | crate::Node::ForInStatement { body, .. } => {
            collect_return_provenance(body, parameters, known_returns, out)
        }
        crate::Node::Match { arms, .. } => {
            for (_pattern, _guard, body) in arms {
                collect_return_provenance(body, parameters, known_returns, out);
            }
        }
        // A nested closure returns to its own caller, not to this function.
        crate::Node::FunctionLiteral { .. } => {}
        _ => {}
    }
}

/// Summarize a reference-typed function when every explicit return is the
/// same reference parameter. This remains independent of branch conditions:
/// no matter which collected path returns, the region provenance is the same.
/// A mixed parameter return, unknown expression, or no explicit return makes
/// the function ineligible so ambiguous provenance remains unchecked.
fn direct_return_alias_summary(
    body: &crate::Node,
    parameters: &[(String, String)],
    return_type: Option<&str>,
    known_returns: &HashMap<&str, usize>,
) -> Option<usize> {
    return_type.and_then(region_from_type_str)?;
    let mut returns = Vec::new();
    collect_return_provenance(body, parameters, known_returns, &mut returns);
    let Some(Some(param_idx)) = returns.first() else {
        return None;
    };
    if returns
        .iter()
        .any(|return_idx| return_idx != &Some(*param_idx))
    {
        return None;
    }
    Some(*param_idx)
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
    fn let_alias_through_direct_reference_return_rejected() {
        // A helper that returns one reference parameter unchanged cannot
        // hide the alias from the caller's region check.
        let errors = run_alias_check(
            "fn expose(&mut int x) -> &mut int { return x; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let y = expose(x); set_both(x, y); }",
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
    fn let_alias_through_proven_reference_return_forward_rejected() {
        // A wrapper may forward a reference only through a helper whose
        // return provenance was already proven; the fixed point handles
        // either declaration order.
        let errors = run_alias_check(
            "fn forward(&mut int x) -> &mut int { return expose(x); } \
             fn expose(&mut int x) -> &mut int { return x; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let y = forward(x); set_both(x, y); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn let_alias_through_same_reference_return_paths_rejected() {
        // Branching is safe to summarize when every explicit return still
        // returns the same parameter.
        let errors = run_alias_check(
            "fn expose(&mut int x, int c) -> &mut int { \
                 if (c > 0) { return x; } else { return x; } \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int c) { let y = expose(x, c); set_both(x, y); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn let_alias_through_reference_struct_field_rejected() {
        // A declared reference field initialized from x preserves the
        // region provenance through the field access expression.
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let h = new Holder { item: x }; set_both(x, h.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`h.item`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn let_alias_through_nested_reference_struct_field_rejected() {
        // Nested struct literals preserve provenance through each concrete
        // declared field path, not just the outer reference field.
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Outer { Inner inner } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let outer = new Outer { inner: new Inner { item: x } }; \
                 set_both(x, outer.inner.item); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`outer.inner.item`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_reference_struct_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn make_holder(&mut int x) -> Holder { return new Holder { item: x }; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let h = make_holder(x); set_both(x, h.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`h.item`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_reference_struct_field_forward_chain_rejected() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn outer(&mut int x) -> Holder { return inner(x); } \
             fn inner(&mut int x) -> Holder { return new Holder { item: x }; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let h = outer(x); set_both(x, h.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`h.item`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_nested_reference_struct_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Outer { Inner inner } \
             fn make_outer(&mut int x) -> Outer { \
                 return new Outer { inner: new Inner { item: x } }; \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let outer = make_outer(x); set_both(x, outer.inner.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`outer.inner.item`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_nested_reference_struct_forward_chain_rejected() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Outer { Inner inner } \
             fn outer(&mut int x) -> Outer { return inner(x); } \
             fn inner(&mut int x) -> Outer { \
                 return new Outer { inner: new Inner { item: x } }; \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let value = outer(x); set_both(x, value.inner.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn wrapped_nested_reference_struct_return_stays_conservative() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Outer { Inner inner } \
             fn make_outer(&mut int x) -> Outer { \
                 let alias = x; \
                 return new Outer { inner: new Inner { item: alias } }; \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let outer = make_outer(x); set_both(x, outer.inner.item); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped nested struct helper returns must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn wrapped_reference_struct_field_forward_chain_stays_conservative() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn outer(&mut int x) -> Holder { let alias = x; return inner(alias); } \
             fn inner(&mut int x) -> Holder { return new Holder { item: x }; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let h = outer(x); set_both(x, h.item); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped struct helper arguments must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn wrapped_reference_struct_return_stays_conservative() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn make_holder(&mut int x) -> Holder { \
                 let alias = x; return new Holder { item: alias }; \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let h = make_holder(x); set_both(x, h.item); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped helper returns must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn helper_returned_reference_tuple_element_alias_rejected() {
        let errors = run_alias_check(
            "fn make_pair(&mut int x, &mut int y) -> (&mut int, &mut int) { return (x, y); } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let pair = make_pair(x, y); set_both(x, pair.0); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`pair.0`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_nested_reference_struct_tuple_element_alias_rejected() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Holder { Inner inner } \
             fn make_pair(&mut int x) -> (Holder, Holder) { \
                 return (new Holder { inner: new Inner { item: x } }, \
                         new Holder { inner: new Inner { item: x } }); \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let pair = make_pair(x); set_both(x, pair.0.inner.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`pair.0.inner.item`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_nested_reference_struct_tuple_forward_chain_rejected() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Holder { Inner inner } \
             fn outer(&mut int x) -> (Holder, Holder) { return inner(x); } \
             fn inner(&mut int x) -> (Holder, Holder) { \
                 return (new Holder { inner: new Inner { item: x } }, \
                         new Holder { inner: new Inner { item: x } }); \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let pair = outer(x); set_both(x, pair.0.inner.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn wrapped_nested_reference_struct_tuple_return_stays_conservative() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Holder { Inner inner } \
             fn make_pair(&mut int x) -> (Holder, Holder) { \
                 let alias = x; \
                 return (new Holder { inner: new Inner { item: alias } }, \
                         new Holder { inner: new Inner { item: alias } }); \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let pair = make_pair(x); set_both(x, pair.0.inner.item); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped nested tuple struct returns must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn helper_returned_reference_tuple_array_element_alias_rejected() {
        let errors = run_alias_check(
            "fn make_pair(&mut int x, &mut int y) -> (array, array) { \
                 return ([x], [y]); \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let pair = make_pair(x, y); set_both(x, pair.0[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`pair.0[0]`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_reference_tuple_array_forward_chain_rejected() {
        let errors = run_alias_check(
            "fn outer(&mut int x, &mut int y) -> (array, array) { return inner(x, y); } \
             fn inner(&mut int x, &mut int y) -> (array, array) { return ([x], [y]); } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let pair = outer(x, y); set_both(x, pair.0[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn wrapped_reference_tuple_array_return_stays_conservative() {
        let errors = run_alias_check(
            "fn make_pair(&mut int x, &mut int y) -> (array, array) { \
                 let alias = x; \
                 return ([alias], [y]); \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let pair = make_pair(x, y); set_both(x, pair.0[0]); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped tuple array returns must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn helper_returned_reference_tuple_array_call_element_alias_rejected() {
        let errors = run_alias_check(
            "fn make_array(&mut int x, &mut int y) -> array { return [x, y]; } \
             fn make_pair(&mut int x, &mut int y) -> (array, array) { \
                 return (make_array(x, y), [y]); \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let pair = make_pair(x, y); set_both(x, pair.0[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("pair.0[0]"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_reference_tuple_array_call_forward_chain_rejected() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { return [x]; } \
             fn inner(&mut int x) -> (array, array) { return (make_array(x), [x]); } \
             fn outer(&mut int x) -> (array, array) { return inner(x); } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let pair = outer(x); set_both(x, pair.0[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn wrapped_reference_tuple_array_call_element_stays_conservative() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { return [x]; } \
             fn make_pair(&mut int x) -> (array, array) { \
                 let alias = x; return (make_array(alias), [x]); \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let pair = make_pair(x); set_both(x, pair.0[0]); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped tuple array helper calls must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn helper_returned_reference_tuple_forward_chain_rejected() {
        let errors = run_alias_check(
            "fn outer(&mut int x, &mut int y) -> (&mut int, &mut int) { return inner(x, y); } \
             fn inner(&mut int x, &mut int y) -> (&mut int, &mut int) { return (x, y); } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let pair = outer(x, y); set_both(x, pair.0); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`pair.0`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn wrapped_reference_tuple_forward_chain_stays_conservative() {
        let errors = run_alias_check(
            "fn outer(&mut int x, &mut int y) -> (&mut int, &mut int) { \
                 let alias = x; return inner(alias, y); \
             } \
             fn inner(&mut int x, &mut int y) -> (&mut int, &mut int) { return (x, y); } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let pair = outer(x, y); set_both(x, pair.0); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped tuple helper arguments must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn helper_returned_nested_reference_tuple_element_alias_rejected() {
        let errors = run_alias_check(
            "fn make_nested(&mut int x, &mut int y) -> ((&mut int, &mut int), &mut int) { \
                 return ((x, y), y); \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let pair = make_nested(x, y); set_both(x, pair.0.0); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`pair.0.0`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn wrapped_reference_tuple_return_stays_conservative() {
        let errors = run_alias_check(
            "fn make_pair(&mut int x, &mut int y) -> (&mut int, &mut int) { \
                 let first = x; return (first, y); \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let pair = make_pair(x, y); set_both(x, pair.0); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped tuple returns must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn helper_returned_reference_array_element_alias_rejected() {
        let errors = run_alias_check(
            "fn make_array(&mut int x, &mut int y) -> array { return [x, y]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { let items = make_array(x, y); set_both(x, items[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`items[0]`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_calls_accept_tracked_composite_reference_arguments() {
        let direct_errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn identity(&mut int item) -> &mut int { return item; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let source = new Holder { item: x }; \
                 let copy = identity(source.item); \
                 set_both(x, copy); \
             }",
        );
        assert_eq!(direct_errors.len(), 1, "got: {:?}", direct_errors);

        let struct_errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn make_holder(&mut int item) -> Holder { \
                 return new Holder { item: item }; \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let source = new Holder { item: x }; \
                 let copy = make_holder(source.item); \
                 set_both(x, copy.item); \
             }",
        );
        assert_eq!(struct_errors.len(), 1, "got: {:?}", struct_errors);

        let tuple_errors = run_alias_check(
            "fn make_pair(&mut int item) -> (&mut int, &mut int) { \
                 return (item, item); \
             } \
             struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let source = new Holder { item: x }; \
                 let pair = make_pair(source.item); \
                 set_both(x, pair.0); \
             }",
        );
        assert_eq!(tuple_errors.len(), 1, "got: {:?}", tuple_errors);

        let array_errors = run_alias_check(
            "fn make_array(&mut int item) -> array { return [item]; } \
             struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let source = new Holder { item: x }; \
                 let items = make_array(source.item); \
                 set_both(x, items[0]); \
             }",
        );
        assert_eq!(array_errors.len(), 1, "got: {:?}", array_errors);
    }

    #[test]
    fn helper_returned_reference_array_forward_chain_rejected() {
        let errors = run_alias_check(
            "fn outer(&mut int x) -> array { return inner(x); } \
             fn inner(&mut int x) -> array { return [x]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = outer(x); set_both(x, items[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`items[0]`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn wrapped_reference_array_forward_chain_stays_conservative() {
        let errors = run_alias_check(
            "fn outer(&mut int x) -> array { let alias = x; return inner(alias); } \
             fn inner(&mut int x) -> array { return [x]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = outer(x); set_both(x, items[0]); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped helper arguments must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn helper_returned_reference_array_tuple_path_alias_rejected() {
        let errors = run_alias_check(
            "fn make_items(&mut int x) -> array { return [(x, 0)]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = make_items(x); set_both(x, items[0].0); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`items[0].0`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_reference_array_struct_path_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn make_items(&mut int x) -> array { return [new Holder { item: x }]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = make_items(x); set_both(x, items[0].item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`items[0].item`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_reference_array_nested_path_alias_rejected() {
        let errors = run_alias_check(
            "fn make_items(&mut int x) -> array { return [[x]]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = make_items(x); set_both(x, items[0][0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`items[0][0]`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn wrapped_reference_array_composite_return_stays_conservative() {
        let errors = run_alias_check(
            "fn make_items(&mut int x) -> array { \
                 let alias = x; return [(alias, 0)]; \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = make_items(x); set_both(x, items[0].0); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped composite array returns must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn helper_returned_reference_array_paths_survive_alias_and_slice() {
        let errors = run_alias_check(
            "fn make_array(&mut int x, &mut int y) -> array { return [x, y]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { \
                 let items = make_array(x, y); \
                 let copy = items; \
                 let selected = copy[0..1]; \
                 set_both(x, selected[0]); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`selected[0]`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn wrapped_reference_array_return_stays_conservative() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { \
                 let alias = x; return [alias]; \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = make_array(x); set_both(x, items[0]); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped array returns must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn ambiguous_reference_array_return_stays_conservative() {
        let errors = run_alias_check(
            "fn choose(&mut int x, &mut int y, int tag) -> array { \
                 if (tag > 0) { return [x]; } else { return [y]; } \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y, int tag) { \
                 let items = choose(x, y, tag); \
                 set_both(x, items[0]); \
             }",
        );
        assert!(
            errors.is_empty(),
            "ambiguous array returns must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn helper_returned_reference_nested_array_element_alias_rejected() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { return [x]; } \
             fn outer(&mut int x) -> array { return [make_array(x)]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = outer(x); set_both(x, items[0][0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("items[0][0]"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn helper_returned_reference_nested_array_forward_chain_rejected() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { return [x]; } \
             fn inner(&mut int x) -> array { return [make_array(x)]; } \
             fn outer(&mut int x) -> array { return inner(x); } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = outer(x); set_both(x, items[0][0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn nested_array_tuple_helper_path_alias_rejected() {
        let errors = run_alias_check(
            "fn make_pair(&mut int x, &mut int y) -> (&mut int, &mut int) { return (x, y); } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { \
                 let matrix = [[make_pair(x, y)]]; \
                 set_both(x, matrix[0][0].0); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`matrix[0][0].0`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn nested_array_struct_helper_path_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn make_holder(&mut int x) -> Holder { \
                 return new Holder { item: x }; \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let matrix = [[make_holder(x)]]; \
                 set_both(x, matrix[0][0].item); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`matrix[0][0].item`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn wrapped_reference_nested_array_helper_stays_conservative() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { return [x]; } \
             fn outer(&mut int x) -> array { \
                 let alias = x; return [make_array(alias)]; \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = outer(x); set_both(x, items[0][0]); }",
        );
        assert!(
            errors.is_empty(),
            "wrapped nested array helper values must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn direct_array_literal_helper_element_alias_rejected() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { return [x]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = [make_array(x)]; set_both(x, items[0][0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("items[0][0]"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_array_literal_helper_chain_preserves_nested_path() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { return [x]; } \
             fn inner(&mut int x) -> array { return [make_array(x)]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = [inner(x)]; set_both(x, items[0][0][0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn unknown_array_helper_inside_literal_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let items = [unknown(x)]; set_both(x, items[0][0]); }",
        );
        assert!(
            errors.is_empty(),
            "unknown array helper values must stay opaque: {:?}",
            errors
        );
    }

    #[test]
    fn direct_tuple_literal_helper_element_alias_rejected() {
        let errors = run_alias_check(
            "fn make_pair(&mut int x, &mut int y) -> (&mut int, &mut int) { return (x, y); } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x, &mut int y) { let pair = (make_pair(x, y), make_pair(y, x)); set_both(x, pair.0.0); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("pair.0.0"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_tuple_literal_helper_chain_preserves_nested_path() {
        let errors = run_alias_check(
            "fn make_pair(&mut int x, &mut int y) -> (&mut int, &mut int) { return (x, y); } fn inner(&mut int x, &mut int y) -> (&mut int, &mut int) { return make_pair(x, y); } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x, &mut int y) { let pair = (inner(x, y), make_pair(y, x)); set_both(x, pair.0.0); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn unknown_tuple_helper_inside_literal_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let pair = (unknown(x), (x, x)); set_both(x, pair.0.0); }",
        );
        assert!(
            errors.is_empty(),
            "unknown tuple helper values must stay opaque: {:?}",
            errors
        );
    }

    #[test]
    fn direct_tuple_literal_struct_helper_element_alias_rejected() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } fn make_inner(&mut int x) -> Inner { return new Inner { item: x }; } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = (make_inner(x), 0); set_both(x, pair.0.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("pair.0.item"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_tuple_literal_struct_helper_chain_preserves_nested_field() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } fn make_inner(&mut int x) -> Inner { return new Inner { item: x }; } fn forward(&mut int x) -> Inner { return make_inner(x); } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = (forward(x), 0); set_both(x, pair.0.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn unknown_struct_helper_inside_tuple_literal_stays_conservative() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = (unknown(x), 0); set_both(x, pair.0.item); }",
        );
        assert!(
            errors.is_empty(),
            "unknown struct helper values must stay opaque: {:?}",
            errors
        );
    }

    #[test]
    fn direct_tuple_literal_array_helper_element_alias_rejected() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { return [x]; } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = (make_array(x), 0); set_both(x, pair.0[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("pair.0[0]"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_tuple_literal_array_helper_chain_preserves_nested_path() {
        let errors = run_alias_check(
            "fn make_array(&mut int x) -> array { return [x]; } fn forward(&mut int x) -> array { return make_array(x); } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = (forward(x), 0); set_both(x, pair.0[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn unknown_array_helper_inside_tuple_literal_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = (unknown(x), 0); set_both(x, pair.0[0]); }",
        );
        assert!(
            errors.is_empty(),
            "unknown array helper values must stay opaque: {:?}",
            errors
        );
    }

    #[test]
    fn direct_tuple_literal_array_value_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = ([x], 0); set_both(x, pair.0[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("pair.0[0]"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_tuple_literal_array_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let items = [x]; let pair = (items, 0); set_both(x, pair.0[0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("pair.0[0]"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_tuple_literal_struct_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let holder = new Holder { item: x }; let pair = (holder, 0); set_both(x, pair.0.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("pair.0.item"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn unknown_composite_alias_inside_tuple_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let items = unknown(x); let pair = (items, 0); set_both(x, pair.0[0]); }",
        );
        assert!(
            errors.is_empty(),
            "unknown tuple composite values must stay opaque: {:?}",
            errors
        );
    }

    #[test]
    fn nested_tuple_literal_array_value_preserves_nested_path() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = ([[x]], 0); set_both(x, pair.0[0][0]); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn unknown_tuple_literal_array_value_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = ([unknown(x)], 0); set_both(x, pair.0[0]); }",
        );
        assert!(
            errors.is_empty(),
            "unknown array values must stay opaque: {:?}",
            errors
        );
    }

    #[test]
    fn direct_tuple_literal_struct_value_alias_rejected() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = (new Inner { item: x }, 0); set_both(x, pair.0.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("pair.0.item"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn nested_tuple_literal_struct_value_preserves_nested_field() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } struct Outer { Inner inner } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = (new Outer { inner: new Inner { item: x } }, 0); set_both(x, pair.0.inner.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn tuple_literal_value_struct_field_stays_conservative() {
        let errors = run_alias_check(
            "struct Inner { int item } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let pair = (new Inner { item: x }, 0); set_both(x, pair.0.item); }",
        );
        assert!(
            errors.is_empty(),
            "value fields must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn direct_struct_literal_helper_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } struct Outer { Inner inner } fn make_inner(&mut int x) -> Inner { return new Inner { item: x }; } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let outer = new Outer { inner: make_inner(x) }; set_both(x, outer.inner.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("outer.inner.item"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_struct_literal_helper_chain_preserves_nested_field() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } struct Outer { Inner inner } fn make_inner(&mut int x) -> Inner { return new Inner { item: x }; } fn forward(&mut int x) -> Inner { return make_inner(x); } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let outer = new Outer { inner: forward(x) }; set_both(x, outer.inner.item); }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn direct_struct_literal_tuple_helper_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { (&mut int, &mut int) pair } \
             fn make_pair(&mut int x, &mut int y) -> (&mut int, &mut int) { return (x, y); } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { \
                 let holder = new Holder { pair: make_pair(x, y) }; \
                 set_both(x, holder.pair.0); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`holder.pair.0`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_struct_literal_array_helper_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { array items } \
             fn make_array(&mut int x) -> array { return [x]; } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let holder = new Holder { items: make_array(x) }; \
                 set_both(x, holder.items[0]); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`holder.items[0]`"),
            "message shape wrong: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_struct_literal_tuple_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { (&mut int, &mut int) pair } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int y) { \
                 let holder = new Holder { pair: (x, y) }; \
                 set_both(x, holder.pair.0); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn direct_struct_literal_array_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { array items } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let holder = new Holder { items: [x] }; \
                 set_both(x, holder.items[0]); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn direct_struct_literal_array_slice_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { array items } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [0, x]; \
                 let holder = new Holder { items: items[1..2] }; \
                 set_both(x, holder.items[0]); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("holder.items[0]"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn direct_struct_literal_array_slice_preserves_nested_paths() {
        let tuple_errors = run_alias_check(
            "struct Holder { array items } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [(x, 0)]; \
                 let holder = new Holder { items: items[0..1] }; \
                 set_both(x, holder.items[0].0); \
             }",
        );
        assert_eq!(tuple_errors.len(), 1, "got: {:?}", tuple_errors);

        let struct_errors = run_alias_check(
            "struct Inner { &mut int item } struct Holder { array items } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [new Inner { item: x }]; \
                 let holder = new Holder { items: items[0..1] }; \
                 set_both(x, holder.items[0].item); \
             }",
        );
        assert_eq!(struct_errors.len(), 1, "got: {:?}", struct_errors);
    }

    #[test]
    fn direct_struct_literal_field_access_preserves_nested_paths() {
        let array_errors = run_alias_check(
            "struct Source { array items } \
             struct Holder { array items } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let source = new Source { items: [x] }; \
                 let holder = new Holder { items: source.items }; \
                 set_both(x, holder.items[0]); \
             }",
        );
        assert_eq!(array_errors.len(), 1, "got: {:?}", array_errors);

        let struct_errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Source { Inner inner } \
             struct Outer { Inner inner } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let source = new Source { inner: new Inner { item: x } }; \
                 let outer = new Outer { inner: source.inner }; \
                 set_both(x, outer.inner.item); \
             }",
        );
        assert_eq!(struct_errors.len(), 1, "got: {:?}", struct_errors);
    }

    #[test]
    fn direct_struct_literal_tuple_index_preserves_nested_paths() {
        let array_errors = run_alias_check(
            "struct Holder { array items } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = ([x], 0); \
                 let holder = new Holder { items: pair.0 }; \
                 set_both(x, holder.items[0]); \
             }",
        );
        assert_eq!(array_errors.len(), 1, "got: {:?}", array_errors);

        let tuple_errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Holder { Inner inner } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = (new Inner { item: x }, 0); \
                 let holder = new Holder { inner: pair.0 }; \
                 set_both(x, holder.inner.item); \
             }",
        );
        assert_eq!(tuple_errors.len(), 1, "got: {:?}", tuple_errors);
    }

    #[test]
    fn dynamic_array_slice_struct_field_stays_conservative() {
        let errors = run_alias_check(
            "struct Holder { array items } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int hi) { \
                 let items = [x]; \
                 let holder = new Holder { items: items[0..hi] }; \
                 set_both(x, holder.items[0]); \
             }",
        );
        assert!(
            errors.is_empty(),
            "dynamic struct-field slices must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn direct_struct_literal_composite_value_fields_stay_conservative() {
        let tuple_errors = run_alias_check(
            "struct Holder { (int, int) pair } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let holder = new Holder { pair: (0, 1) }; \
                 set_both(x, holder.pair.0); \
             }",
        );
        assert!(
            tuple_errors.is_empty(),
            "value tuple fields must stay outside alias tracking: {:?}",
            tuple_errors
        );

        let array_errors = run_alias_check(
            "struct Holder { array items } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let holder = new Holder { items: [0] }; \
                 set_both(x, holder.items[0]); \
             }",
        );
        assert!(
            array_errors.is_empty(),
            "value array fields must stay outside alias tracking: {:?}",
            array_errors
        );
    }

    #[test]
    fn unknown_struct_helper_inside_literal_stays_conservative() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } struct Outer { Inner inner } fn set_both(&mut int a, &mut int b) {} fn caller(&mut int x) { let outer = new Outer { inner: unknown(x) }; set_both(x, outer.inner.item); }",
        );
        assert!(
            errors.is_empty(),
            "unknown struct helper values must stay opaque: {:?}",
            errors
        );
    }

    #[test]
    fn value_nested_struct_field_is_not_treated_as_reference_alias() {
        // A value field at the nested path must not inherit the same-name
        // reference field from a different struct.
        let errors = run_alias_check(
            "struct Inner { int item } \
             struct Outer { Inner inner } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let outer = new Outer { inner: new Inner { item: x } }; \
                 set_both(x, outer.inner.item); \
             }",
        );
        assert!(
            errors.is_empty(),
            "value fields must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn reassigned_nested_reference_struct_field_is_killed_conservatively() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Outer { Inner inner } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let outer = new Outer { inner: new Inner { item: x } }; \
                 outer.inner.item = x; \
                 set_both(x, outer.inner.item); \
             }",
        );
        assert!(
            errors.is_empty(),
            "nested field writes must kill the tracked fact: {:?}",
            errors
        );
    }

    #[test]
    fn value_struct_field_is_not_treated_as_reference_alias() {
        // A same-named value field must not be inferred as a reference just
        // because another struct declares a reference field with that name.
        let errors = run_alias_check(
            "struct Holder { int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let h = new Holder { item: x }; set_both(x, h.item); }",
        );
        assert!(
            errors.is_empty(),
            "value fields must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn reassigned_reference_struct_field_is_killed_conservatively() {
        // Field re-seating semantics are not modeled, so a write detaches
        // the old provenance instead of claiming a new alias.
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { let h = new Holder { item: x }; h.item = x; set_both(x, h.item); }",
        );
        assert!(
            errors.is_empty(),
            "field writes must kill the tracked fact: {:?}",
            errors
        );
    }

    #[test]
    fn ambiguous_reference_return_stays_conservatively_unchecked() {
        // A conditional return may choose a different region, so the
        // narrow direct-return summary must not claim an alias.
        let errors = run_alias_check(
            "fn choose(&mut int a, &mut int b, int c) -> &mut int { \
                 if (c > 0) { return a; } else { return b; } \
             } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, &mut int z, int c) { let y = choose(x, z, c); set_both(x, y); }",
        );
        assert!(
            errors.is_empty(),
            "ambiguous return provenance must remain unchecked: {:?}",
            errors
        );
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
    fn let_alias_match_arm_alias_is_rejected() {
        // Facts established before a match remain available inside each
        // arm, so an alias created in one arm is checked on that path.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int c) { \
                 match c { 0 => { let y = x; set_both(x, y); }, _ => { println(\"n\"); } } \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "match-arm alias must be rejected: {:?}",
            errors
        );
    }

    #[test]
    fn let_alias_match_preserves_facts_across_all_arms() {
        // A fact that survives every arm remains provable after the match.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int c) { \
                 let y = x; \
                 match c { 0 => { println(\"a\"); }, _ => { println(\"b\"); } } \
                 set_both(x, y); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "match merge must preserve aliases: {:?}",
            errors
        );
    }

    #[test]
    fn let_alias_match_pattern_binding_shadows_outer_fact() {
        // A pattern binding named `y` shadows the outer alias only in its
        // arm; the checker must not mistake it for the old reference.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int y) { \
                 let alias = x; \
                 match y { alias => { set_both(x, alias); }, _ => { println(\"n\"); } } \
             }",
        );
        assert!(
            errors.is_empty(),
            "pattern shadow must not reuse outer alias: {:?}",
            errors
        );
    }

    #[test]
    fn let_alias_nested_match_pattern_binding_shadows_outer_fact() {
        // Binding collectors must recurse through enum payload patterns as
        // well as handling a bare identifier pattern.
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, Option<int> value) { \
                 let alias = x; \
                 match value { Some(alias) => { set_both(x, alias); }, None => { println(\"n\"); } } \
             }",
        );
        assert!(
            errors.is_empty(),
            "nested pattern shadow must not reuse outer alias: {:?}",
            errors
        );
    }

    #[test]
    fn match_struct_pattern_preserves_composite_paths() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Outer { Inner inner, (&mut int, int) pair } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let holder = new Outer { \
                     inner: new Inner { item: x }, \
                     pair: (x, 0), \
                 }; \
                 match holder { \
                     Outer { \
                         inner: Inner { item }, \
                         pair: (first, _), \
                     } => { \
                         set_both(x, item); \
                         set_both(x, first); \
                     }, \
                     _ => { println(\"unreachable\"); }, \
                 } \
             }",
        );
        assert_eq!(errors.len(), 2, "got: {:?}", errors);
    }

    #[test]
    fn match_tuple_pattern_preserves_composite_paths() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = ((x, 0), 0); \
                 match pair { \
                     ((first, _), _) => { set_both(x, first); }, \
                     _ => { println(\"unreachable\"); }, \
                 } \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn match_option_result_patterns_preserve_payload_paths() {
        let errors = run_alias_check(
            r#"fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let some = Some(x);
                   match some {
                       Some(alias) => { set_both(x, alias); },
                       None => { println("none"); },
                   }
                   match Some(x) {
                       Some(alias) => { set_both(x, alias); },
                       None => { println("none"); },
                   }
                   match Ok(x) {
                       Ok(alias) => { set_both(x, alias); },
                       Err(_) => { println("err"); },
                   }
                   match Err(x) {
                       Ok(_) => { println("ok"); },
                       Err(alias) => { set_both(x, alias); },
                   }
               }"#,
        );
        assert_eq!(errors.len(), 4, "got: {:?}", errors);
    }

    #[test]
    fn match_tagged_enum_patterns_preserve_payload_paths() {
        let errors = run_alias_check(
            r#"struct Holder { &mut int item }
               enum Packet {
                   Item(Holder),
                   Named { holder: Holder },
               }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   match Packet::Item(new Holder { item: x }) {
                       Packet::Item(alias) => { set_both(x, alias.item); },
                       _ => { println("unreachable"); },
                   }
                   let named = new Packet::Named {
                       holder: new Holder { item: x },
                   };
                   match named {
                       Packet::Named { holder } => { set_both(x, holder.item); },
                       _ => { println("unreachable"); },
                   }
               }"#,
        );
        assert_eq!(errors.len(), 2, "got: {:?}", errors);
    }

    #[test]
    fn mismatched_known_constructor_patterns_do_not_reuse_payload_paths() {
        let errors = run_alias_check(
            r#"struct Holder { &mut int item }
               enum Packet {
                   Item(Holder),
                   Other(Holder),
               }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   match Packet::Item(new Holder { item: x }) {
                       Packet::Other(alias) => { set_both(x, alias.item); },
                       Packet::Item(_) => { println("item"); },
                   }
                   match Some(x) {
                       None => { println("none"); },
                       Some(alias) => { println(alias); },
                   }
                   match Ok(x) {
                       Err(alias) => { set_both(x, alias); },
                       Ok(_) => { println("ok"); },
                   }
               }"#,
        );
        assert!(
            errors.is_empty(),
            "mismatched constructors must not inherit payload aliases: {:?}",
            errors
        );
    }

    #[test]
    fn constructor_identity_survives_whole_value_aliases() {
        let errors = run_alias_check(
            r#"struct Holder { &mut int item }
               enum Packet {
                   Item(Holder),
                   Other(Holder),
               }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let packet = Packet::Item(new Holder { item: x });
                   let packet_copy = packet;
                   match packet_copy {
                       Packet::Other(alias) => { set_both(x, alias.item); },
                       Packet::Item(_) => { println("item"); },
                   }
                   let result = Ok(x);
                   match result {
                       Err(alias) => { set_both(x, alias); },
                       Ok(_) => { println("ok"); },
                   }
               }"#,
        );
        assert!(
            errors.is_empty(),
            "constructor tags must survive direct aliases: {:?}",
            errors
        );
    }

    #[test]
    fn nested_constructor_places_remain_variant_aware() {
        let errors = run_alias_check(
            r#"struct Holder { &mut int item }
               enum Packet {
                   Item(Holder),
                   Other(Holder),
               }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let pair = (Packet::Item(new Holder { item: x }), 0);
                   match pair.0 {
                       Packet::Item(alias) => { set_both(x, alias.item); },
                       Packet::Other(alias) => { set_both(x, alias.item); },
                   }
                   let items = [Packet::Item(new Holder { item: x })];
                   match items[0] {
                       Packet::Item(alias) => { set_both(x, alias.item); },
                       Packet::Other(alias) => { set_both(x, alias.item); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            2,
            "only matching nested constructor arms should report: {:?}",
            errors
        );
    }

    #[test]
    fn struct_field_constructor_places_remain_variant_aware() {
        let errors = run_alias_check(
            r#"struct Holder { &mut int item }
               enum Packet {
                   Item(Holder),
                   Other(Holder),
               }
               struct Wrapper { Packet packet }
               struct Outer { Wrapper wrapper }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let outer = new Outer {
                       wrapper: new Wrapper {
                           packet: Packet::Item(new Holder { item: x }),
                       },
                   };
                   match outer.wrapper.packet {
                       Packet::Item(alias) => { set_both(x, alias.item); },
                       Packet::Other(alias) => { set_both(x, alias.item); },
                   }
                   let copy = outer;
                   match copy.wrapper.packet {
                       Packet::Item(alias) => { set_both(x, alias.item); },
                       Packet::Other(alias) => { set_both(x, alias.item); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            2,
            "only matching struct-field constructor arms should report: {:?}",
            errors
        );
    }

    #[test]
    fn option_result_struct_field_places_remain_variant_aware() {
        let errors = run_alias_check(
            r#"struct OptionWrapper { Option<&mut int> value }
               struct ResultWrapper { Result<&mut int, int> value }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let some = new OptionWrapper { value: Some(x) };
                   match some.value {
                       Some(alias) => { set_both(x, alias); },
                       None => { println("none"); },
                   }
                   let ok = new ResultWrapper { value: Ok(x) };
                   match ok.value {
                       Ok(alias) => { set_both(x, alias); },
                       Err(_) => { println("err"); },
                   }
                   let err = new ResultWrapper { value: Err(x) };
                   match err.value {
                       Ok(alias) => { set_both(x, alias); },
                       Err(alias) => { set_both(x, alias); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            3,
            "only matching Option/Result constructor arms should report: {:?}",
            errors
        );
    }

    #[test]
    fn let_alias_match_rebound_fact_is_killed_after_match() {
        // Rebinding in one arm means the alias is not available after the
        // match, even though another arm leaves it untouched.
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

    #[test]
    fn closure_capture_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let y = x; \
                 let f = fn() { set_both(x, y); }; \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "captured alias must be reported: {:?}",
            errors
        );
        assert!(
            errors[0].contains("set_both"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn closure_local_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let f = fn() { let y = x; set_both(x, y); }; \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "closure-local alias must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn closure_parameter_shadows_captured_name() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let y = x; \
                 let f = fn(&mut int x) { set_both(x, y); }; \
             }",
        );
        assert!(
            errors.is_empty(),
            "independent closure parameter must not alias captured reference: {:?}",
            errors
        );
    }

    #[test]
    fn array_literal_element_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [x]; \
                 set_both(x, items[0]); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "array element alias must be reported: {:?}",
            errors
        );
        assert!(
            errors[0].contains("items[0]") && errors[0].contains("tracked reference aliasing"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn array_literal_struct_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [new Holder { item: x }]; \
                 set_both(x, items[0].item); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`items[0].item`"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn array_literal_value_struct_field_stays_conservative() {
        let errors = run_alias_check(
            "struct Holder { int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [new Holder { item: x }]; \
                 set_both(x, items[0].item); \
             }",
        );
        assert!(
            errors.is_empty(),
            "value-typed array fields must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn nested_array_literal_struct_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let matrix = [[new Holder { item: x }]]; \
                 set_both(x, matrix[0][0].item); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`matrix[0][0].item`"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn nested_array_literal_struct_field_preserves_nested_path() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Outer { Inner inner } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let matrix = [[new Outer { inner: new Inner { item: x } }]]; \
                 set_both(x, matrix[0][0].inner.item); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn nested_array_literal_value_struct_field_stays_conservative() {
        let errors = run_alias_check(
            "struct Holder { int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let matrix = [[new Holder { item: x }]]; \
                 set_both(x, matrix[0][0].item); \
             }",
        );
        assert!(
            errors.is_empty(),
            "nested value-typed array fields must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn nested_array_literal_tuple_element_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let matrix = [[(x, 0)]]; \
                 set_both(x, matrix[0][0].0); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`matrix[0][0].0`"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn nested_array_literal_tuple_struct_field_preserves_nested_path() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let matrix = [[(new Inner { item: x }, 0)]]; \
                 set_both(x, matrix[0][0].0.item); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn nested_array_literal_tuple_value_element_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let matrix = [[(0, 1)]]; \
                 set_both(x, matrix[0][0].0); \
             }",
        );
        assert!(
            errors.is_empty(),
            "nested value-typed tuple elements must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn nested_array_alias_preserves_tuple_element() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = (x, 0); \
                 let matrix = [[pair]]; \
                 set_both(x, matrix[0][0].0); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`matrix[0][0].0`"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn nested_array_alias_preserves_struct_field() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let holder = new Holder { item: x }; \
                 let matrix = [[holder]]; \
                 set_both(x, matrix[0][0].item); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`matrix[0][0].item`"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn array_literal_distinct_elements_with_same_root_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [x, x]; \
                 set_both(items[0], items[1]); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "array element aliases must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn array_alias_preserves_element_alias() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [x, x]; \
                 let copy = items; \
                 set_both(x, copy[0]); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "array alias must be reported: {:?}",
            errors
        );
        assert!(
            errors[0].contains("copy[0]"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn nested_array_literal_alias_is_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let matrix = [[x]]; \
                 set_both(x, matrix[0][0]); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "nested array literal alias must be reported: {:?}",
            errors
        );
        assert!(
            errors[0].contains("matrix[0][0]"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn nested_array_alias_and_slice_are_rejected() {
        let alias_errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let row = [x]; \
                 let matrix = [row]; \
                 let copy = matrix; \
                 set_both(x, copy[0][0]); \
             }",
        );
        assert_eq!(
            alias_errors.len(),
            1,
            "nested array alias must be reported: {:?}",
            alias_errors
        );

        let slice_errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let matrix = [[x], [0]]; \
                 let selected = matrix[0..1]; \
                 set_both(x, selected[0][0]); \
             }",
        );
        assert_eq!(
            slice_errors.len(),
            1,
            "nested array slice alias must be reported: {:?}",
            slice_errors
        );
    }

    #[test]
    fn nested_array_literal_slice_preserves_element_alias() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [0, x]; \
                 let matrix = [items[1..2]]; \
                 set_both(x, matrix[0][0]); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("matrix[0][0]"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn nested_array_literal_slice_preserves_nested_paths() {
        let tuple_errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [(x, 0)]; \
                 let matrix = [items[0..1]]; \
                 set_both(x, matrix[0][0].0); \
             }",
        );
        assert_eq!(tuple_errors.len(), 1, "got: {:?}", tuple_errors);

        let struct_errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [new Holder { item: x }]; \
                 let matrix = [items[0..1]]; \
                 set_both(x, matrix[0][0].item); \
             }",
        );
        assert_eq!(struct_errors.len(), 1, "got: {:?}", struct_errors);
    }

    #[test]
    fn dynamic_nested_array_slice_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int hi) { \
                 let items = [x]; \
                 let matrix = [items[0..hi]]; \
                 set_both(x, matrix[0][0]); \
             }",
        );
        assert!(
            errors.is_empty(),
            "dynamic nested array slices must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn nested_array_literal_field_access_preserves_nested_paths() {
        let array_errors = run_alias_check(
            "struct Source { array items } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let source = new Source { items: [x] }; \
                 let matrix = [[source.items]]; \
                 set_both(x, matrix[0][0][0]); \
             }",
        );
        assert_eq!(array_errors.len(), 1, "got: {:?}", array_errors);

        let struct_errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Source { Inner inner } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let source = new Source { inner: new Inner { item: x } }; \
                 let matrix = [[source.inner]]; \
                 set_both(x, matrix[0][0].item); \
             }",
        );
        assert_eq!(struct_errors.len(), 1, "got: {:?}", struct_errors);
    }

    #[test]
    fn nested_array_literal_tuple_index_preserves_nested_paths() {
        let array_errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = ([x], 0); \
                 let matrix = [[pair.0]]; \
                 set_both(x, matrix[0][0][0]); \
             }",
        );
        assert_eq!(array_errors.len(), 1, "got: {:?}", array_errors);

        let tuple_errors = run_alias_check(
            "struct Inner { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = (new Inner { item: x }, 0); \
                 let matrix = [[pair.0]]; \
                 set_both(x, matrix[0][0].item); \
             }",
        );
        assert_eq!(tuple_errors.len(), 1, "got: {:?}", tuple_errors);
    }

    #[test]
    fn dynamic_nested_array_paths_stay_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int index) { \
                 let matrix = [[x]]; \
                 set_both(x, matrix[index][0]); \
             }",
        );
        assert!(
            errors.is_empty(),
            "dynamic nested array paths must stay conservative: {:?}",
            errors
        );
    }

    #[test]
    fn array_alias_preserves_nested_field_and_tuple_paths() {
        let field_errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [new Holder { item: x }]; \
                 let copy = items; \
                 set_both(x, copy[0].item); \
             }",
        );
        assert_eq!(
            field_errors.len(),
            1,
            "array field alias must be reported: {:?}",
            field_errors
        );

        let tuple_errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [(x, 0)]; \
                 let copy = items; \
                 let selected = copy[0..1]; \
                 set_both(x, selected[0].0); \
             }",
        );
        assert_eq!(
            tuple_errors.len(),
            1,
            "array tuple alias must be reported: {:?}",
            tuple_errors
        );
    }

    #[test]
    fn tuple_literal_element_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = (x, 0); \
                 set_both(x, pair.0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "tuple element alias must be reported: {:?}",
            errors
        );
        assert!(
            errors[0].contains("pair.0") && errors[0].contains("tracked reference aliasing"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn tuple_literal_distinct_elements_with_same_root_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = (x, x); \
                 set_both(pair.0, pair.1); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "tuple element aliases must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn nested_tuple_literal_chained_index_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = ((x, 0), 1); \
                 set_both(x, pair.0.0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "nested tuple element alias must be reported: {:?}",
            errors
        );
        assert!(
            errors[0].contains("pair.0.0"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn nested_tuple_literal_distinct_paths_with_same_root_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = ((x, 0), (x, 1)); \
                 set_both(pair.0.0, pair.1.0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "nested tuple paths must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn tuple_alias_preserves_nested_element_path() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let inner = (x, 0); \
                 let pair = (inner, 1); \
                 set_both(x, pair.0.0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "nested tuple alias must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn nested_tuple_value_path_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = ((0, 1), 2); \
                 set_both(x, pair.0.0); \
             }",
        );
        assert!(
            errors.is_empty(),
            "value-typed nested tuple paths must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn array_tuple_element_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [(x, 0)]; \
                 set_both(x, items[0].0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "array tuple element alias must be reported: {:?}",
            errors
        );
        assert!(
            errors[0].contains("items[0].0"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn array_tuple_distinct_paths_with_same_root_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [(x, 0), (x, 1)]; \
                 set_both(items[0].0, items[1].0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "array tuple paths must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn array_tuple_alias_preserves_nested_tuple_path() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let inner = ((x, 0), 1); \
                 let items = [inner]; \
                 set_both(x, items[0].0.0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "array tuple nested path must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn composite_let_copy_preserves_struct_paths() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Outer { Inner inner } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let source = new Outer { inner: new Inner { item: x } }; \
                 let copy = source; \
                 let nested = source.inner; \
                 set_both(x, copy.inner.item); \
                 set_both(x, nested.item); \
             }",
        );
        assert_eq!(errors.len(), 2, "got: {:?}", errors);
    }

    #[test]
    fn array_tuple_value_path_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [(0, 1)]; \
                 set_both(x, items[0].0); \
             }",
        );
        assert!(
            errors.is_empty(),
            "value-typed array tuple paths must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn array_tuple_slice_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [(x, 0)]; \
                 let selected = items[0..1]; \
                 set_both(x, selected[0].0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "array tuple slice alias must be reported: {:?}",
            errors
        );
        assert!(
            errors[0].contains("selected[0].0"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn inclusive_array_tuple_slice_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [(x, 0), (x, 1)]; \
                 let selected = items[0..=0]; \
                 set_both(x, selected[0].0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "inclusive array tuple slice alias must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn dynamic_array_tuple_slice_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int hi) { \
                 let items = [(x, 0)]; \
                 let selected = items[0..hi]; \
                 set_both(x, selected[0].0); \
             }",
        );
        assert!(
            errors.is_empty(),
            "dynamic array tuple slices must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn array_tuple_slice_preserves_nested_tuple_path() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [((x, 0), 1)]; \
                 let selected = items[0..1]; \
                 set_both(x, selected[0].0.0); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "nested array tuple slice path must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn tuple_literal_slice_preserves_element_alias() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [0, x]; \
                 let pair = (items[1..2], 0); \
                 set_both(x, pair.0[0]); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("pair.0[0]"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn tuple_literal_slice_preserves_nested_paths() {
        let tuple_errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [(x, 0)]; \
                 let pair = (items[0..1], 0); \
                 set_both(x, pair.0[0].0); \
             }",
        );
        assert_eq!(tuple_errors.len(), 1, "got: {:?}", tuple_errors);

        let struct_errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [new Holder { item: x }]; \
                 let pair = (items[0..1], 0); \
                 set_both(x, pair.0[0].item); \
             }",
        );
        assert_eq!(struct_errors.len(), 1, "got: {:?}", struct_errors);
    }

    #[test]
    fn dynamic_slice_inside_tuple_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int hi) { \
                 let items = [x]; \
                 let pair = (items[0..hi], 0); \
                 set_both(x, pair.0[0]); \
             }",
        );
        assert!(
            errors.is_empty(),
            "dynamic tuple slices must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn tuple_literal_value_element_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = (0, 1); \
                 set_both(x, pair.0); \
             }",
        );
        assert!(
            errors.is_empty(),
            "value-typed tuple elements must stay outside alias tracking: {:?}",
            errors
        );
    }

    #[test]
    fn tuple_destructure_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let (first, second) = (x, 0); \
                 set_both(x, first); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "tuple destructure alias must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn tuple_destructure_preserves_composite_paths() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let pair = (new Holder { item: x }, 0); \
                 let (holder, discard) = pair; \
                 let items = [x]; \
                 let array_pair = (items, 0); \
                 let (copy, other) = array_pair; \
                 let nested_pair = ((new Holder { item: x }, 0), 0); \
                 let (nested, last) = nested_pair; \
                 set_both(x, holder.item); \
                 set_both(x, copy[0]); \
                 set_both(x, nested.0.item); \
             }",
        );
        assert_eq!(errors.len(), 3, "got: {:?}", errors);
    }

    #[test]
    fn tuple_destructure_preserves_element_positions() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let (first, second) = (0, x); \
                 set_both(x, first); \
             }",
        );
        assert!(
            errors.is_empty(),
            "value element must not inherit a later tuple alias: {:?}",
            errors
        );
    }

    #[test]
    fn struct_destructure_preserves_composite_paths() {
        let errors = run_alias_check(
            "struct Inner { &mut int item } \
             struct Outer { Inner inner, Array items, (&mut int, int) pair } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let outer = new Outer { \
                     inner: new Inner { item: x }, \
                     items: [new Inner { item: x }], \
                     pair: (x, 0), \
                 }; \
                 let Outer { inner, items, pair, .. } = outer; \
                 set_both(x, inner.item); \
                 set_both(x, items[0].item); \
                 set_both(x, pair.0); \
             }",
        );
        assert_eq!(errors.len(), 3, "got: {:?}", errors);
    }

    #[test]
    fn struct_destructure_value_field_stays_conservative() {
        let errors = run_alias_check(
            "struct Holder { &mut int item, int value } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let holder = new Holder { item: x, value: 0 }; \
                 let Holder { item: alias, value, .. } = holder; \
                 set_both(x, alias); \
                 set_both(x, value); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
    }

    #[test]
    fn constructor_identity_survives_tuple_and_struct_destructuring() {
        let errors = run_alias_check(
            r#"struct Holder { &mut int item }
               enum Packet {
                   Item(Holder),
                   Other(Holder),
               }
               struct OptionWrapper { Option<&mut int> value }
               struct ResultWrapper { Result<&mut int, int> value }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let pair = (Packet::Item(new Holder { item: x }), 0);
                   let (packet, _) = pair;
                   match packet {
                       Packet::Item(alias) => { set_both(x, alias.item); },
                       Packet::Other(alias) => { set_both(x, alias.item); },
                   }
                   let option_wrapper = new OptionWrapper { value: Some(x) };
                   let OptionWrapper { value: some, .. } = option_wrapper;
                   match some {
                       Some(alias) => { set_both(x, alias); },
                       None => { println("none"); },
                   }
                   let result_wrapper = new ResultWrapper { value: Err(x) };
                   let ResultWrapper { value: err, .. } = result_wrapper;
                   match err {
                       Ok(alias) => { set_both(x, alias); },
                       Err(alias) => { set_both(x, alias); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            3,
            "only matching destructured constructor arms should report: {:?}",
            errors
        );
    }

    #[test]
    fn nested_constructor_patterns_do_not_reuse_mismatched_payloads() {
        let errors = run_alias_check(
            r#"struct Holder { &mut int item }
               enum Packet {
                   Item(Holder),
                   Other(Holder),
               }
               struct PacketWrapper { Packet packet }
               struct OptionWrapper { Option<&mut int> value }
               struct ResultWrapper { Result<&mut int, &mut int> value }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let matching_packet = new PacketWrapper {
                       packet: Packet::Item(new Holder { item: x }),
                   };
                   match matching_packet {
                       PacketWrapper { packet: Packet::Other(alias) } => {
                           set_both(x, alias.item);
                       },
                       PacketWrapper { packet: Packet::Item(alias) } => {
                           set_both(x, alias.item);
                       },
                   }
                   let matching_option = new OptionWrapper { value: Some(x) };
                   match matching_option {
                       OptionWrapper { value: Some(alias) } => { set_both(x, alias); },
                       OptionWrapper { value: None } => { println("none"); },
                   }
                   let option_wrapper = new OptionWrapper { value: None };
                   match option_wrapper {
                       OptionWrapper { value: Some(alias) } => { set_both(x, alias); },
                       OptionWrapper { value: None } => { println("none"); },
                   }
                   let result_wrapper = new ResultWrapper { value: Err(x) };
                   match result_wrapper {
                       ResultWrapper { value: Ok(alias) } => { set_both(x, alias); },
                       ResultWrapper { value: Err(_) } => { println("err"); },
                   }
                   let matching_result = new ResultWrapper { value: Err(x) };
                   match matching_result {
                       ResultWrapper { value: Ok(_) } => { println("ok"); },
                       ResultWrapper { value: Err(alias) } => { set_both(x, alias); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            3,
            "only matching nested constructors should report: {:?}",
            errors
        );
    }

    #[test]
    fn option_result_constructor_payloads_survive_tuple_and_array_places() {
        let errors = run_alias_check(
            r#"fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let pair = (Some(x), 0);
                   match pair.0 {
                       Some(alias) => { set_both(x, alias); },
                       None => { println("none"); },
                   }
                   let items = [Err(x)];
                   match items[0] {
                       Ok(alias) => { set_both(x, alias); },
                       Err(alias) => { set_both(x, alias); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            2,
            "matching Option and Result payloads should report: {:?}",
            errors
        );
    }

    #[test]
    fn option_result_constructor_payloads_survive_constant_array_slices() {
        let errors = run_alias_check(
            r#"fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let options = [Some(x)];
                   let selected = options[0..1];
                   match selected[0] {
                       Some(alias) => { set_both(x, alias); },
                       None => { println("none"); },
                   }
                   let results = [Err(x)];
                   let chosen = results[0..1];
                   match chosen[0] {
                       Ok(alias) => { set_both(x, alias); },
                       Err(alias) => { set_both(x, alias); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            2,
            "matching sliced Option and Result payloads should report: {:?}",
            errors
        );
    }

    #[test]
    fn option_result_constructor_identity_survives_direct_helper_returns() {
        let errors = run_alias_check(
            r#"fn expose_some(&mut int value) -> Option<&mut int> {
                   return Some(value);
               }
               fn expose_err(&mut int value) -> Result<int, &mut int> {
                   return Err(value);
               }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   match expose_some(x) {
                       Some(alias) => { set_both(x, alias); },
                       None => { println("none"); },
                   }
                   let result = expose_err(x);
                   match result {
                       Ok(alias) => { set_both(x, alias); },
                       Err(alias) => { set_both(x, alias); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            2,
            "matching helper constructors should report: {:?}",
            errors
        );
    }

    #[test]
    fn tagged_enum_constructor_identity_survives_constant_array_slices() {
        let errors = run_alias_check(
            r#"struct Holder { &mut int item }
               enum Packet {
                   Item(Holder),
                   Other(Holder),
               }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   let items = [Packet::Item(new Holder { item: x })];
                   let selected = items[0..1];
                   match selected[0] {
                       Packet::Item(alias) => { set_both(x, alias.item); },
                       Packet::Other(alias) => { set_both(x, alias.item); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            1,
            "only the matching sliced constructor should report: {:?}",
            errors
        );
    }

    #[test]
    fn tagged_enum_constructor_identity_survives_direct_helper_returns() {
        let errors = run_alias_check(
            r#"struct Holder { &mut int item }
               enum Packet {
                   Item(Holder),
                   Other(Holder),
               }
               fn expose_item(&mut int value) -> Packet {
                   return Packet::Item(new Holder { item: value });
               }
               fn expose_other(&mut int value) -> Packet {
                   return Packet::Other(new Holder { item: value });
               }
               fn set_both(&mut int a, &mut int b) {}
               fn caller(&mut int x) {
                   match expose_item(x) {
                       Packet::Item(alias) => { set_both(x, alias.item); },
                       Packet::Other(alias) => { set_both(x, alias.item); },
                   }
                   let packet = expose_other(x);
                   match packet {
                       Packet::Item(alias) => { set_both(x, alias.item); },
                       Packet::Other(alias) => { set_both(x, alias.item); },
                   }
               }"#,
        );
        assert_eq!(
            errors.len(),
            2,
            "matching helper constructors should report: {:?}",
            errors
        );
    }

    #[test]
    fn array_element_write_kills_only_known_fact() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [x, x]; \
                 items[0] = 0; \
                 set_both(x, items[0]); \
             }",
        );
        assert!(
            errors.is_empty(),
            "known element write must kill its alias fact: {:?}",
            errors
        );
    }

    #[test]
    fn dynamic_array_element_write_kills_array_facts() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int i) { \
                 let items = [x]; \
                 items[i] = 0; \
                 set_both(x, items[0]); \
             }",
        );
        assert!(
            errors.is_empty(),
            "dynamic element write must kill array facts: {:?}",
            errors
        );
    }

    #[test]
    fn conditional_array_element_write_kills_fact_after_match() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int tag) { \
                 let items = [x]; \
                 match tag { 0 => { items[0] = 0; }, _ => { println(\"keep\"); } } \
                 set_both(x, items[0]); \
             }",
        );
        assert!(
            errors.is_empty(),
            "conditional element write must kill the post-match fact: {:?}",
            errors
        );
    }

    #[test]
    fn constant_array_slice_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [0, x]; \
                 let selected = items[1..2]; \
                 set_both(x, selected[0]); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "slice alias must be reported: {:?}",
            errors
        );
        assert!(
            errors[0].contains("selected[0]"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn constant_array_slice_struct_field_alias_rejected() {
        let errors = run_alias_check(
            "struct Holder { &mut int item } \
             fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [new Holder { item: x }]; \
                 let selected = items[0..1]; \
                 set_both(x, selected[0].item); \
             }",
        );
        assert_eq!(errors.len(), 1, "got: {:?}", errors);
        assert!(
            errors[0].contains("`x`") && errors[0].contains("`selected[0].item`"),
            "unexpected message: {}",
            errors[0]
        );
    }

    #[test]
    fn inclusive_constant_array_slice_alias_rejected() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x) { \
                 let items = [0, x]; \
                 let selected = items[1..=1]; \
                 set_both(x, selected[0]); \
             }",
        );
        assert_eq!(
            errors.len(),
            1,
            "inclusive slice alias must be reported: {:?}",
            errors
        );
    }

    #[test]
    fn dynamic_array_slice_stays_conservative() {
        let errors = run_alias_check(
            "fn set_both(&mut int a, &mut int b) {} \
             fn caller(&mut int x, int hi) { \
                 let items = [x]; \
                 let selected = items[0..hi]; \
                 set_both(x, selected[0]); \
             }",
        );
        assert!(
            errors.is_empty(),
            "dynamic slice must stay opaque: {:?}",
            errors
        );
    }
}
