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
    ///
    /// RES-2212: walk the union-find chain via `&Region` borrows from
    /// the `parent` HashMap. The previous shape did `r = parent.clone()`
    /// at every step — a fresh `Region` allocation per chain link
    /// (including a `String` clone whenever a `Named(_)` was bumped
    /// out by an intermediate `Var(_)`). For chain depth D with a
    /// `Named` target, the new loop clones the final `String` exactly
    /// once and otherwise just walks `u32` indices.
    pub fn resolve(&self, r: Region) -> Region {
        let mut current_var = match r {
            Region::Var(v) => v.0,
            Region::Named(_) => return r,
        };
        loop {
            match self.parent.get(&current_var) {
                None => return Region::Var(RegionVar(current_var)),
                Some(Region::Var(v)) => current_var = v.0,
                Some(Region::Named(name)) => return Region::Named(name.clone()),
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

/// EXTENSION_PASSES entry point — runs after type-checking.
///
/// RES-1202: this pass was originally a placeholder slot for the D2/D5
/// inference work that landed in `build_region_map`. The function body
/// historically called `build_region_map(program)` and immediately
/// *discarded* the returned `RegionMap`, then returned `Ok(())`.
///
/// The actual consumer of the region map (the alias-aliasing check at
/// `lib.rs:check_region_aliasing`) builds its own copy via
/// `build_region_map(program)` when it needs one, so the work here was
/// unobservable: no thread-local, no global, no I/O — just an
/// allocation and a tree walk whose result was dropped on function
/// exit. For a single type-check that meant walking the AST twice for
/// region inference (once here, once at the consumer) instead of once.
///
/// The entry point is kept (so the `EXTENSION_PASSES` block in
/// `typechecker.rs` is undisturbed and a future use can flow data
/// here) but the body is now empty.
pub fn infer(_program: &crate::Node, _source_path: &str) -> Result<(), String> {
    Ok(())
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
}
