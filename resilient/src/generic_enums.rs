//! RES-2575: generic enum declarations — `enum Name<T> { Variant(T), ... }`.
//!
//! This module owns all generic-enum logic. The existing enum machinery
//! in `sum_types.rs` keeps the non-generic surface; this module adds the
//! type-parameter validation, substitution, and monomorphization that
//! generic enums need.
//!
//! ## Surface syntax
//!
//! ```text
//! enum Either<L, R> {
//!     Left(L),
//!     Right(R),
//! }
//!
//! enum Option<T> {
//!     None,
//!     Some(T),
//! }
//!
//! enum Result<T, E> {
//!     Ok(T),
//!     Err(E),
//! }
//!
//! enum Box<T> {
//!     Wrap(T),
//! }
//! ```
//!
//! ## What ships in this PR
//!
//! - **Parser**: `parse_enum_decl` in `sum_types.rs` consumes an optional
//!   `<T, U>` segment after the enum name. Type-parameter names land in
//!   `EnumDecl::type_params`.
//! - **Typechecker validation**: this module's `check` pass runs after
//!   the existing sum-type registration and enforces:
//!     1. Type-parameter names are unique within the enum.
//!     2. Every type referenced inside a variant payload either names a
//!        declared type parameter or is a resolvable concrete type.
//!     3. Type-parameter shadowing of concrete types is forbidden
//!        (e.g. you cannot declare `enum Foo<int> { ... }`).
//! - **Substitution helpers**: `substitute_payload_type` rewrites a
//!   payload-type string by replacing free type-parameter names with
//!   the concrete types from a `Subst` map. The lowering pass (next
//!   PR) calls this to produce specialized variants per concrete
//!   instantiation.
//! - **Monomorphization registry**: `MonoRegistry` collects the set of
//!   `(EnumName, TypeArgs)` pairs observed at type-positions in the
//!   program (e.g. `Either<int, string>` in a `let` annotation or a
//!   parameter type). The bytecode VM (RES-2576) and the JIT
//!   (RES-2577) consume the registry to emit one specialized payload
//!   layout per concrete use.
//!
//! The existing `Option` / `Result` builtin entries continue to work
//! exactly as before; this module's check pass is a strict superset
//! that recognises generic syntax without disturbing the monomorphic
//! path. Users who declare their own `Option<T>` or `Result<T, E>`
//! get an error noting that those names are reserved by the stdlib
//! — that keeps the post-substitution `Type::Option` / `Type::Result`
//! shortcuts in the typechecker unambiguous.
//!
//! ## Design notes
//!
//! - The construction site (`new EnumName::Variant(...)`) infers
//!   type arguments from the payload's actual types. Where the enum
//!   has variants that don't carry every type parameter (e.g.
//!   `Option<T>::None`), inference falls back to the let-binding
//!   annotation when one is present. Today, an unbound type
//!   parameter at a `None`-style construction site without a type
//!   annotation surfaces as a diagnostic asking the user to spell
//!   the annotation out — same shape as `Option::<int>::None`
//!   would, just expressed differently.
//! - Pattern-matching deconstructs payloads by position (tuple) or
//!   name (named). The bound payload's type is the type-parameter
//!   resolved against the matched expression's static type at the
//!   match site.
//! - Trait bounds on enum type parameters are accepted at parse
//!   time but not enforced until RES-2578 lands the bound-checker
//!   pass; that PR shares `traits::check`'s plumbing.

// PR 1 lays the surface; downstream PRs consume `MonoRegistry`,
// `substitute_payload_type`, and `register_monomorphization`.
#![allow(dead_code)]

use crate::span::Span;
use crate::{EnumPayload, EnumVariant, Node};
use std::collections::{BTreeMap, HashSet};

// ---------------------------------------------------------------------------
// Substitution machinery
// ---------------------------------------------------------------------------

/// Map from a type-parameter name (e.g. `"T"`) to the concrete type
/// name it was instantiated with at a construction or annotation
/// site. Stored as a string because the rest of the AST keeps
/// payload-type annotations as strings (`EnumPayload::Tuple(Vec<String>)`,
/// `EnumField::ty: String`). The typechecker resolves the string
/// against its type environment when consuming the substituted
/// payload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Subst {
    map: BTreeMap<String, String>,
}

impl Subst {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a substitution from parallel `(param_name, concrete_type)`
    /// pairs. The arity must match — callers handle the arity-mismatch
    /// diagnostic before invoking this.
    pub fn from_pairs(params: &[String], concretes: &[String]) -> Self {
        let mut map = BTreeMap::new();
        for (p, c) in params.iter().zip(concretes.iter()) {
            map.insert(p.clone(), c.clone());
        }
        Self { map }
    }

    pub fn bind(&mut self, param: &str, concrete: &str) -> Result<(), String> {
        match self.map.get(param) {
            Some(existing) if existing != concrete => Err(format!(
                "type parameter `{}` is inferred as both `{}` and `{}` — they must agree",
                param, existing, concrete
            )),
            _ => {
                self.map.insert(param.to_string(), concrete.to_string());
                Ok(())
            }
        }
    }

    pub fn get(&self, param: &str) -> Option<&String> {
        self.map.get(param)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.map.iter()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

/// Replace every free type-parameter name in `ty` with its concrete
/// binding. Unbound names pass through unchanged so concrete types
/// (`int`, `string`, user-defined struct names) are preserved.
///
/// This is intentionally string-level rather than `Type`-level — the
/// AST stores payload annotations as strings and the typechecker
/// resolves them lazily when the substituted variant is consumed.
pub fn substitute_payload_type(ty: &str, subst: &Subst) -> String {
    match subst.get(ty) {
        Some(replaced) => replaced.clone(),
        None => ty.to_string(),
    }
}

/// Apply a substitution to an entire variant payload, returning a
/// fresh `EnumPayload` whose payload-type strings have been rewritten
/// in place.
pub fn substitute_payload(payload: &EnumPayload, subst: &Subst) -> EnumPayload {
    match payload {
        EnumPayload::None => EnumPayload::None,
        EnumPayload::Tuple(tys) => EnumPayload::Tuple(
            tys.iter()
                .map(|t| substitute_payload_type(t, subst))
                .collect(),
        ),
        EnumPayload::Named(fields) => EnumPayload::Named(
            fields
                .iter()
                .map(|f| crate::EnumField {
                    name: f.name.clone(),
                    ty: substitute_payload_type(&f.ty, subst),
                    span: f.span,
                })
                .collect(),
        ),
    }
}

/// Specialize all variants of a generic enum under a given substitution.
/// Returns a fresh `Vec<EnumVariant>` with payload types rewritten —
/// callers register the result under the mangled monomorphized name
/// (`Either<int,string>`).
pub fn specialize_variants(variants: &[EnumVariant], subst: &Subst) -> Vec<EnumVariant> {
    variants
        .iter()
        .map(|v| EnumVariant {
            name: v.name.clone(),
            span: v.span,
            payload: substitute_payload(&v.payload, subst),
        })
        .collect()
}

/// Mangle a generic enum reference into a flat type name suitable for
/// use as a key in the typechecker's enum table. Order of concrete
/// types matches the declaration order of the enum's type parameters.
///
/// Example: `mangle_mono_name("Either", &["int", "string"])` →
/// `"Either<int,string>"`. The format is `Name<arg1,arg2,...>` with no
/// internal whitespace so equality lookups are byte-exact.
pub fn mangle_mono_name(name: &str, type_args: &[String]) -> String {
    if type_args.is_empty() {
        return name.to_string();
    }
    let mut out = String::with_capacity(
        name.len() + type_args.iter().map(|t| t.len() + 1).sum::<usize>() + 2,
    );
    out.push_str(name);
    out.push('<');
    let mut first = true;
    for arg in type_args {
        if !first {
            out.push(',');
        }
        out.push_str(arg);
        first = false;
    }
    out.push('>');
    out
}

/// Inverse of `mangle_mono_name`. Splits `"Either<int,string>"` into
/// `("Either", ["int", "string"])`. Returns `None` if the input is
/// not a mangled name (no `<>` brackets).
pub fn unmangle_mono_name(name: &str) -> Option<(&str, Vec<&str>)> {
    let lt = name.find('<')?;
    if !name.ends_with('>') {
        return None;
    }
    let base = &name[..lt];
    let inside = &name[lt + 1..name.len() - 1];
    let args: Vec<&str> = if inside.is_empty() {
        Vec::new()
    } else {
        inside.split(',').collect()
    };
    Some((base, args))
}

// ---------------------------------------------------------------------------
// Monomorphization registry
// ---------------------------------------------------------------------------

/// Records every concrete instantiation of a generic enum observed
/// during a typecheck. Downstream backends (bytecode VM, JIT, Lean
/// export) consume the registry to emit one specialized payload
/// layout per (enum, type-args) tuple. Keyed by the mangled name so
/// equality lookups are O(name length).
#[derive(Debug, Clone, Default)]
pub struct MonoRegistry {
    entries: BTreeMap<String, MonoEntry>,
}

#[derive(Debug, Clone)]
pub struct MonoEntry {
    pub base_name: String,
    pub type_args: Vec<String>,
    /// Specialized variants with payload types substituted.
    pub variants: Vec<EnumVariant>,
}

impl MonoRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a monomorphization. Idempotent — re-registering the same
    /// `(base, args)` is a no-op so caller paths that walk the AST
    /// multiple times don't double-emit.
    pub fn register(
        &mut self,
        base_name: &str,
        type_params: &[String],
        type_args: &[String],
        variants: &[EnumVariant],
    ) -> Result<(), String> {
        if type_params.len() != type_args.len() {
            return Err(format!(
                "enum `{}` takes {} type argument(s), got {}",
                base_name,
                type_params.len(),
                type_args.len()
            ));
        }
        let mangled = mangle_mono_name(base_name, type_args);
        if self.entries.contains_key(&mangled) {
            return Ok(());
        }
        let subst = Subst::from_pairs(type_params, type_args);
        let specialized = specialize_variants(variants, &subst);
        self.entries.insert(
            mangled,
            MonoEntry {
                base_name: base_name.to_string(),
                type_args: type_args.to_vec(),
                variants: specialized,
            },
        );
        Ok(())
    }

    pub fn get(&self, mangled: &str) -> Option<&MonoEntry> {
        self.entries.get(mangled)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &MonoEntry)> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Validation pass
// ---------------------------------------------------------------------------

/// Validate every generic enum declaration in `program`. Returns the
/// first error encountered, or `Ok(())` if all generic enums pass.
///
/// Checks performed:
/// 1. Type-parameter names within a single enum are unique.
/// 2. Type-parameter names don't shadow built-in primitive types
///    (`int`, `float`, `string`, `bool`, `char`, `byte`, `bytes`,
///    `void`, `any`, `Unit`, signed / unsigned width-pinned ints).
/// 3. Every payload type either names a declared type parameter or
///    is plausibly a concrete type (non-empty identifier). The
///    actual type resolution happens later in the typechecker's
///    `parse_type_name` pass — here we only catch obvious mistakes.
///
/// Programs that declare zero generic enums short-circuit to `Ok`
/// after a single scan, so non-generic code pays nothing.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    let has_generic_enum = stmts.iter().any(|s| {
        matches!(
            &s.node,
            Node::EnumDecl { type_params, .. } if !type_params.is_empty()
        )
    });
    if !has_generic_enum {
        return Ok(());
    }

    for stmt in stmts {
        if let Node::EnumDecl {
            name,
            type_params,
            variants,
            span,
        } = &stmt.node
        {
            if type_params.is_empty() {
                continue;
            }
            validate_generic_enum(name, type_params, variants, *span, source_path)?;
        }
    }
    Ok(())
}

fn validate_generic_enum(
    name: &str,
    type_params: &[String],
    variants: &[EnumVariant],
    span: Span,
    source_path: &str,
) -> Result<(), String> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(type_params.len());
    for tp in type_params {
        if !seen.insert(tp.as_str()) {
            return Err(format!(
                "{}:{}:{}: error: duplicate type parameter `{}` in enum `{}`",
                source_path, span.start.line, span.start.column, tp, name
            ));
        }
        if is_reserved_type_name(tp) {
            return Err(format!(
                "{}:{}:{}: error: type parameter `{}` of enum `{}` shadows a built-in type — pick a different name (convention: single uppercase letter, e.g. `T`)",
                source_path, span.start.line, span.start.column, tp, name
            ));
        }
    }

    let tp_set: HashSet<&str> = type_params.iter().map(String::as_str).collect();
    for v in variants {
        match &v.payload {
            EnumPayload::None => {}
            EnumPayload::Tuple(tys) => {
                for ty in tys {
                    validate_payload_type_name(name, &v.name, ty, &tp_set, span, source_path)?;
                }
            }
            EnumPayload::Named(fields) => {
                let mut field_seen: HashSet<&str> = HashSet::with_capacity(fields.len());
                for f in fields {
                    if !field_seen.insert(f.name.as_str()) {
                        return Err(format!(
                            "{}:{}:{}: error: duplicate field `{}` in variant `{}::{}`",
                            source_path,
                            f.span.start.line,
                            f.span.start.column,
                            f.name,
                            name,
                            v.name
                        ));
                    }
                    validate_payload_type_name(name, &v.name, &f.ty, &tp_set, span, source_path)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_payload_type_name(
    enum_name: &str,
    variant_name: &str,
    ty: &str,
    tp_set: &HashSet<&str>,
    span: Span,
    source_path: &str,
) -> Result<(), String> {
    if ty.is_empty() {
        return Err(format!(
            "{}:{}:{}: error: empty type in payload of variant `{}::{}`",
            source_path, span.start.line, span.start.column, enum_name, variant_name
        ));
    }
    if tp_set.contains(ty) {
        return Ok(());
    }
    // Concrete type — the typechecker's `parse_type_name` validates
    // it when the variant is consumed; here we only insist the
    // string looks like a type identifier.
    if !ty
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_')
        .unwrap_or(false)
    {
        return Err(format!(
            "{}:{}:{}: error: payload type `{}` in variant `{}::{}` is not a valid type identifier",
            source_path, span.start.line, span.start.column, ty, enum_name, variant_name
        ));
    }
    Ok(())
}

pub fn is_reserved_type_name(name: &str) -> bool {
    matches!(
        name,
        "int"
            | "Int"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "float"
            | "Float"
            | "string"
            | "String"
            | "bool"
            | "Bool"
            | "char"
            | "Char"
            | "byte"
            | "Byte"
            | "bytes"
            | "Bytes"
            | "void"
            | "Void"
            | "any"
            | "Any"
            | "Unit"
            | "Option"
            | "Result"
    )
}

// ---------------------------------------------------------------------------
// Convenience helpers used by the typechecker / interpreter / VM
// ---------------------------------------------------------------------------

/// Collect every `EnumDecl` with a non-empty `type_params` from a
/// program. Cheap O(N) pre-scan that the substitution lowering pass
/// uses to enumerate work without rewalking the full AST.
pub fn collect_generic_enums(program: &Node) -> Vec<(&str, &Vec<String>, &Vec<EnumVariant>)> {
    let mut out = Vec::new();
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::EnumDecl {
                name,
                type_params,
                variants,
                ..
            } = &s.node
                && !type_params.is_empty()
            {
                out.push((name.as_str(), type_params, variants));
            }
        }
    }
    out
}

/// True if `name` is the name of a generic enum declared in `program`.
/// Used by the type-name resolver to detect `Either<int, string>`
/// references and dispatch to monomorphization.
pub fn is_generic_enum(program: &Node, name: &str) -> bool {
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::EnumDecl {
                name: en,
                type_params,
                ..
            } = &s.node
                && en == name
                && !type_params.is_empty()
            {
                return true;
            }
        }
    }
    false
}

/// Find the generic enum named `name` and return its `(type_params, variants)`.
/// Returns `None` when the name doesn't refer to a generic enum declared
/// in `program`.
pub fn find_generic_enum<'a>(
    program: &'a Node,
    name: &str,
) -> Option<(&'a Vec<String>, &'a Vec<EnumVariant>)> {
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::EnumDecl {
                name: en,
                type_params,
                variants,
                ..
            } = &s.node
                && en == name
                && !type_params.is_empty()
            {
                return Some((type_params, variants));
            }
        }
    }
    None
}

/// Scan a program for places where a generic enum is referenced with
/// explicit concrete type arguments (e.g. `Either<int, string>`).
/// Returns one (base_name, type_args) per reference; the registry
/// caller deduplicates.
///
/// Today the scan looks at:
/// - Parameter type annotations (`Function::parameters`)
/// - Let-binding type annotations (`Node::Let::type_annotation`)
/// - Return type annotations (`Function::return_type`)
///
/// Anything else passes silently; future PRs extend the scan as
/// the surface grows (struct fields, generic-fn return positions, etc).
pub fn collect_concrete_references(program: &Node) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let strs = collect_type_annotation_strings(program);
    let mut seen: HashSet<String> = HashSet::new();
    for s in strs {
        if !seen.insert(s.clone()) {
            continue;
        }
        if let Some((base, args)) = parse_generic_reference(&s) {
            out.push((
                base.to_string(),
                args.into_iter().map(String::from).collect(),
            ));
        }
    }
    out
}

/// Parse a type-annotation string like `"Either<int, string>"` into
/// `("Either", ["int", "string"])`. Returns `None` if the string is
/// not a generic reference. Trims whitespace around the type
/// arguments so `Either< int , string >` is accepted.
pub fn parse_generic_reference(s: &str) -> Option<(&str, Vec<&str>)> {
    let lt = s.find('<')?;
    if !s.ends_with('>') {
        return None;
    }
    let base = s[..lt].trim();
    let inside = &s[lt + 1..s.len() - 1];
    // Split on commas at depth 0 so nested generics like
    // `Option<Result<int, string>>` parse correctly.
    let mut args: Vec<&str> = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    for (i, c) in inside.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                args.push(inside[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = inside[start..].trim();
    if !last.is_empty() || !args.is_empty() {
        args.push(last);
    }
    Some((base, args))
}

fn collect_type_annotation_strings(program: &Node) -> Vec<String> {
    let mut out = Vec::new();
    walk_for_type_strings(program, &mut out);
    out
}

fn walk_for_type_strings(node: &Node, out: &mut Vec<String>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_for_type_strings(&s.node, out);
            }
        }
        Node::Function {
            parameters,
            return_type,
            body,
            ..
        } => {
            for (ty, _name) in parameters {
                out.push(ty.clone());
            }
            if let Some(rt) = return_type.as_ref() {
                out.push(rt.clone());
            }
            walk_for_type_strings(body, out);
        }
        Node::LetStatement {
            type_annot: Some(ann),
            value,
            ..
        } => {
            out.push(ann.clone());
            walk_for_type_strings(value, out);
        }
        Node::LetStatement { value, .. } => {
            walk_for_type_strings(value, out);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_for_type_strings(s, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_for_type_strings(condition, out);
            walk_for_type_strings(consequence, out);
            if let Some(alt) = alternative {
                walk_for_type_strings(alt, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => walk_for_type_strings(expr, out),
        Node::ReturnStatement { value: Some(v), .. } => walk_for_type_strings(v, out),
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_for_type_strings(condition, out);
            walk_for_type_strings(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_for_type_strings(iterable, out);
            walk_for_type_strings(body, out);
        }
        Node::InfixExpression { left, right, .. } => {
            walk_for_type_strings(left, out);
            walk_for_type_strings(right, out);
        }
        Node::CallExpression { arguments, .. } => {
            for a in arguments {
                walk_for_type_strings(a, out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        check(&prog, "<t>")
    }

    fn enum_decls(prog: &Node) -> Vec<&Node> {
        match prog {
            Node::Program(stmts) => stmts
                .iter()
                .map(|s| &s.node)
                .filter(|n| matches!(n, Node::EnumDecl { .. }))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn first_generic_enum(prog: &Node) -> Option<(&String, &Vec<String>, &Vec<EnumVariant>)> {
        if let Node::Program(stmts) = prog {
            for s in stmts {
                if let Node::EnumDecl {
                    name,
                    type_params,
                    variants,
                    ..
                } = &s.node
                    && !type_params.is_empty()
                {
                    return Some((name, type_params, variants));
                }
            }
        }
        None
    }

    // ---- parser smoke tests ----

    #[test]
    fn parses_single_type_param_enum() {
        let (prog, errs) = parse("enum Box<T> { Wrap(T) }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let decls = enum_decls(&prog);
        assert_eq!(decls.len(), 1);
        let (name, tps, _) = first_generic_enum(&prog).expect("generic enum");
        assert_eq!(name, "Box");
        assert_eq!(tps, &vec!["T".to_string()]);
    }

    #[test]
    fn parses_two_type_param_enum() {
        let (prog, errs) = parse("enum Either<L, R> { Left(L), Right(R) }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let (name, tps, variants) = first_generic_enum(&prog).expect("generic enum");
        assert_eq!(name, "Either");
        assert_eq!(tps, &vec!["L".to_string(), "R".to_string()]);
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].name, "Left");
        match &variants[0].payload {
            EnumPayload::Tuple(t) => assert_eq!(t, &vec!["L".to_string()]),
            other => panic!("expected tuple payload, got {:?}", other),
        }
        match &variants[1].payload {
            EnumPayload::Tuple(t) => assert_eq!(t, &vec!["R".to_string()]),
            other => panic!("expected tuple payload, got {:?}", other),
        }
    }

    #[test]
    fn parses_named_payload_with_type_param() {
        let (prog, errs) = parse("enum Box<T> { Wrap { value: T } }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let (_, _, variants) = first_generic_enum(&prog).expect("generic enum");
        match &variants[0].payload {
            EnumPayload::Named(fields) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "value");
                assert_eq!(fields[0].ty, "T");
            }
            other => panic!("expected named payload, got {:?}", other),
        }
    }

    #[test]
    fn parses_payload_less_variant_in_generic_enum() {
        let (prog, errs) = parse("enum MyOption<T> { Nothing, Just(T) }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let (_, _, variants) = first_generic_enum(&prog).expect("generic enum");
        assert_eq!(variants.len(), 2);
        assert!(matches!(variants[0].payload, EnumPayload::None));
        assert!(matches!(variants[1].payload, EnumPayload::Tuple(_)));
    }

    // ---- validation pass ----

    #[test]
    fn duplicate_type_parameter_is_rejected() {
        let err = check_src("enum Bad<T, T> { A(T) }").expect_err("dup type param should fail");
        assert!(err.contains("duplicate type parameter"), "got: {}", err);
    }

    #[test]
    fn type_param_shadowing_builtin_is_rejected() {
        let err =
            check_src("enum Bad<int> { A(int) }").expect_err("`int` as type param should fail");
        assert!(err.contains("shadows a built-in"), "got: {}", err);
    }

    #[test]
    fn reserved_option_name_as_type_param_is_rejected() {
        let err =
            check_src("enum Bad<Option> { A(Option) }").expect_err("`Option` as param should fail");
        assert!(err.contains("shadows a built-in"), "got: {}", err);
    }

    #[test]
    fn well_formed_either_passes() {
        check_src("enum Either<L, R> { Left(L), Right(R) }").expect("Either should typecheck");
    }

    #[test]
    fn user_defined_option_passes_validation() {
        // The check pass should accept a user-defined Option<T> shape
        // — the conflict with the builtin lives in the typechecker
        // registration pass, not here.
        check_src("enum MyOption<T> { None, Some(T) }")
            .expect("user MyOption<T> should pass validation");
    }

    #[test]
    fn user_defined_result_passes_validation() {
        check_src("enum MyResult<T, E> { Ok(T), Err(E) }")
            .expect("user MyResult<T, E> should pass validation");
    }

    #[test]
    fn duplicate_field_in_named_generic_payload_is_rejected() {
        // Duplicate fields are caught by the parser before the check pass runs.
        let (_, errs) = parse("enum Bad<T> { V { x: T, x: T } }");
        assert!(
            !errs.is_empty(),
            "parser should have caught duplicate field 'x'"
        );
        let combined = errs.join(" ");
        assert!(
            combined.to_lowercase().contains("duplicate"),
            "expected 'Duplicate' in error, got: {combined}"
        );
    }

    #[test]
    fn non_generic_enums_are_unaffected() {
        // Pass should short-circuit when no generic enums are present.
        check_src("enum Color { Red, Green, Blue }").expect("non-generic enum should pass");
    }

    // ---- substitution helpers ----

    #[test]
    fn subst_from_pairs_records_in_order() {
        let s = Subst::from_pairs(
            &["T".to_string(), "U".to_string()],
            &["int".to_string(), "string".to_string()],
        );
        assert_eq!(s.get("T"), Some(&"int".to_string()));
        assert_eq!(s.get("U"), Some(&"string".to_string()));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn subst_bind_consistent_is_ok() {
        let mut s = Subst::new();
        s.bind("T", "int").unwrap();
        s.bind("T", "int")
            .expect("re-binding to the same type should succeed");
    }

    #[test]
    fn subst_bind_inconsistent_errors() {
        let mut s = Subst::new();
        s.bind("T", "int").unwrap();
        let err = s
            .bind("T", "string")
            .expect_err("inconsistent bind should fail");
        assert!(err.contains("inferred as both"), "got: {}", err);
    }

    #[test]
    fn substitute_payload_type_replaces_param() {
        let s = Subst::from_pairs(&["T".to_string()], &["int".to_string()]);
        assert_eq!(substitute_payload_type("T", &s), "int");
    }

    #[test]
    fn substitute_payload_type_leaves_concrete_alone() {
        let s = Subst::from_pairs(&["T".to_string()], &["int".to_string()]);
        assert_eq!(substitute_payload_type("string", &s), "string");
    }

    #[test]
    fn substitute_payload_rewrites_tuple() {
        let s = Subst::from_pairs(
            &["L".to_string(), "R".to_string()],
            &["int".to_string(), "string".to_string()],
        );
        let pl = EnumPayload::Tuple(vec!["L".to_string(), "R".to_string()]);
        match substitute_payload(&pl, &s) {
            EnumPayload::Tuple(tys) => {
                assert_eq!(tys, vec!["int".to_string(), "string".to_string()])
            }
            other => panic!("expected tuple, got {:?}", other),
        }
    }

    #[test]
    fn substitute_payload_leaves_none_alone() {
        let s = Subst::from_pairs(&["T".to_string()], &["int".to_string()]);
        match substitute_payload(&EnumPayload::None, &s) {
            EnumPayload::None => {}
            other => panic!("expected None, got {:?}", other),
        }
    }

    // ---- mangling / unmangling ----

    #[test]
    fn mangle_single_type_arg() {
        assert_eq!(mangle_mono_name("Box", &["int".to_string()]), "Box<int>");
    }

    #[test]
    fn mangle_two_type_args() {
        assert_eq!(
            mangle_mono_name("Either", &["int".to_string(), "string".to_string()]),
            "Either<int,string>"
        );
    }

    #[test]
    fn mangle_no_args_passes_through() {
        assert_eq!(mangle_mono_name("Color", &[]), "Color");
    }

    #[test]
    fn unmangle_round_trips() {
        let mangled = mangle_mono_name("Either", &["int".to_string(), "string".to_string()]);
        let (base, args) = unmangle_mono_name(&mangled).expect("unmangle");
        assert_eq!(base, "Either");
        assert_eq!(args, vec!["int", "string"]);
    }

    #[test]
    fn unmangle_returns_none_for_non_generic() {
        assert_eq!(unmangle_mono_name("Color"), None);
    }

    // ---- monomorphization registry ----

    #[test]
    fn mono_registry_records_specialization() {
        let (prog, errs) = parse("enum Either<L, R> { Left(L), Right(R) }");
        assert!(errs.is_empty());
        let (_, tps, variants) = first_generic_enum(&prog).unwrap();
        let mut reg = MonoRegistry::new();
        reg.register(
            "Either",
            tps,
            &["int".to_string(), "string".to_string()],
            variants,
        )
        .unwrap();
        let entry = reg.get("Either<int,string>").expect("mono entry");
        assert_eq!(entry.base_name, "Either");
        assert_eq!(entry.type_args, vec!["int", "string"]);
        match &entry.variants[0].payload {
            EnumPayload::Tuple(t) => assert_eq!(t, &vec!["int".to_string()]),
            other => panic!("expected tuple, got {:?}", other),
        }
        match &entry.variants[1].payload {
            EnumPayload::Tuple(t) => assert_eq!(t, &vec!["string".to_string()]),
            other => panic!("expected tuple, got {:?}", other),
        }
    }

    #[test]
    fn mono_registry_arity_mismatch_errors() {
        let (prog, errs) = parse("enum Either<L, R> { Left(L), Right(R) }");
        assert!(errs.is_empty());
        let (_, tps, variants) = first_generic_enum(&prog).unwrap();
        let mut reg = MonoRegistry::new();
        let err = reg
            .register("Either", tps, &["int".to_string()], variants)
            .expect_err("arity mismatch should fail");
        assert!(err.contains("takes 2 type argument"), "got: {}", err);
    }

    #[test]
    fn mono_registry_dedupes_repeat_registration() {
        let (prog, _) = parse("enum Box<T> { Wrap(T) }");
        let (_, tps, variants) = first_generic_enum(&prog).unwrap();
        let mut reg = MonoRegistry::new();
        reg.register("Box", tps, &["int".to_string()], variants)
            .unwrap();
        reg.register("Box", tps, &["int".to_string()], variants)
            .unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn mono_registry_handles_distinct_concretes() {
        let (prog, _) = parse("enum Box<T> { Wrap(T) }");
        let (_, tps, variants) = first_generic_enum(&prog).unwrap();
        let mut reg = MonoRegistry::new();
        reg.register("Box", tps, &["int".to_string()], variants)
            .unwrap();
        reg.register("Box", tps, &["string".to_string()], variants)
            .unwrap();
        assert_eq!(reg.len(), 2);
        assert!(reg.get("Box<int>").is_some());
        assert!(reg.get("Box<string>").is_some());
    }

    // ---- generic reference parsing ----

    #[test]
    fn parse_generic_reference_single_arg() {
        assert_eq!(
            parse_generic_reference("Box<int>"),
            Some(("Box", vec!["int"]))
        );
    }

    #[test]
    fn parse_generic_reference_two_args() {
        assert_eq!(
            parse_generic_reference("Either<int, string>"),
            Some(("Either", vec!["int", "string"]))
        );
    }

    #[test]
    fn parse_generic_reference_nested() {
        // Nested generic Option<Result<int, string>>.
        assert_eq!(
            parse_generic_reference("Option<Result<int,string>>"),
            Some(("Option", vec!["Result<int,string>"]))
        );
    }

    #[test]
    fn parse_generic_reference_rejects_non_generic() {
        assert_eq!(parse_generic_reference("Color"), None);
        assert_eq!(parse_generic_reference("int"), None);
    }

    // ---- collect_concrete_references ----

    #[test]
    fn collect_concrete_references_finds_let_annotation() {
        // Parser may not produce a Let::type_annotation everywhere
        // but if it does, we should pick it up. Construct a synthetic
        // program for the helper unit test.
        use crate::span::Span;
        let prog = Node::Program(vec![crate::span::Spanned::new(
            Node::LetStatement {
                name: "x".to_string(),
                value: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: Span::default(),
                }),
                type_annot: Some("Either<int,string>".to_string()),
                span: Span::default(),
            },
            Span::default(),
        )]);
        let refs = collect_concrete_references(&prog);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, "Either");
        assert_eq!(refs[0].1, vec!["int", "string"]);
    }

    // ---- collect_generic_enums / is_generic_enum / find_generic_enum ----

    #[test]
    fn collect_generic_enums_lists_each_declaration() {
        let (prog, _) = parse(
            "enum Box<T> { Wrap(T) } enum Either<L, R> { Left(L), Right(R) } enum Color { Red }",
        );
        let g = collect_generic_enums(&prog);
        assert_eq!(g.len(), 2);
        let names: Vec<&str> = g.iter().map(|(n, _, _)| *n).collect();
        assert!(names.contains(&"Box"));
        assert!(names.contains(&"Either"));
    }

    #[test]
    fn is_generic_enum_finds_declared() {
        let (prog, _) = parse("enum Box<T> { Wrap(T) } enum Color { Red }");
        assert!(is_generic_enum(&prog, "Box"));
        assert!(!is_generic_enum(&prog, "Color"));
        assert!(!is_generic_enum(&prog, "Missing"));
    }

    #[test]
    fn find_generic_enum_returns_decl() {
        let (prog, _) = parse("enum Either<L, R> { Left(L), Right(R) }");
        let (tps, variants) = find_generic_enum(&prog, "Either").expect("find Either");
        assert_eq!(tps, &vec!["L".to_string(), "R".to_string()]);
        assert_eq!(variants.len(), 2);
    }

    // ---- nested generic monomorphization smoke test ----

    #[test]
    fn nested_generic_mono_round_trip() {
        // Synthetic verification: we can monomorphize Option<T> with
        // T = "Result<int,string>" and read the substituted payload
        // back out — proves the substitution machinery doesn't get
        // confused by nested generic syntax in the type-arg string.
        let (prog, _) = parse("enum MyOption<T> { None, Some(T) }");
        let (_, tps, variants) = first_generic_enum(&prog).unwrap();
        let mut reg = MonoRegistry::new();
        reg.register(
            "MyOption",
            tps,
            &["Result<int,string>".to_string()],
            variants,
        )
        .unwrap();
        let entry = reg.get("MyOption<Result<int,string>>").expect("mono");
        assert_eq!(entry.variants.len(), 2);
        // First variant is `None` — no payload, no substitution needed.
        assert!(matches!(entry.variants[0].payload, EnumPayload::None));
        // Second variant is `Some(T)` → `Some(Result<int,string>)`.
        match &entry.variants[1].payload {
            EnumPayload::Tuple(t) => assert_eq!(t, &vec!["Result<int,string>".to_string()]),
            other => panic!("expected tuple, got {:?}", other),
        }
    }
}
