//! RES-290: trait / interface system.
//!
//! Surface syntax (closely modelled on Rust):
//!
//! ```text
//! trait Printable {
//!     fn to_string(self) -> string;
//! }
//!
//! struct Point { x: int; y: int; }
//!
//! impl Printable for Point {
//!     fn to_string(self) -> string {
//!         "Point(" + self.x + ", " + self.y + ")"
//!     }
//! }
//!
//! fn announce<T: Printable>(item: T) { println(item.to_string()); }
//! ```
//!
//! Declarations are nominal-looking, but enforcement is structural —
//! we verify that every `impl Trait for Type` block exposes the methods
//! the trait declares (matching by name and arity), and that any type
//! used at a `<T: Trait>` call site has either an explicit impl or all
//! of the trait's methods bound directly to it via plain `impl Type {
//! ... }` blocks. Runtime dispatch reuses the existing
//! `<TypeName>$<method>` mangling produced by `parse_impl_block`; there
//! is no VTable.
//!
//! Out of scope here: VTable / `dyn Trait`, monomorphisation,
//! supertraits, default method bodies, blanket impls.
//!
//! ## RES-779: Associated Types Extension
//!
//! **Status**: In scope for phase 2. Not yet implemented.
//!
//! Associated types allow traits to declare type members that each impl
//! must define. For example:
//!
//! ```text
//! trait Transport {
//!     type Message;
//!     type Error;
//!
//!     fn send(self, msg: Self::Message) -> Result<(), Self::Error>;
//! }
//! ```
//!
//! This enables reusable embedded abstractions where the type relationships
//! are fixed per implementation:
//!
//! ```text
//! struct Serial { ... }
//!
//! impl Transport for Serial {
//!     type Message = [u8; 64];
//!     type Error = SerialError;
//!
//!     fn send(self, msg: Self::Message) -> Result<(), SerialError> { ... }
//! }
//! ```
//!
//! Implementation plan (RES-779):
//! 1. Parser: `type Name = ConcreteType;` in impl blocks
//! 2. Typechecker: validate associated type definitions, projection (`T::AssocType`)
//! 3. Generics: carry associated type constraints through monomorphization
//! 4. Examples: demonstrate embedded APIs using associated types

#[allow(unused_imports)]
use crate::Lexer;
use crate::span::Span;
use crate::{Node, Parser, Token};
use std::collections::{HashMap, HashSet};

/// Method signature on a trait declaration.
#[derive(Debug, Clone)]
pub(crate) struct TraitMethodSig {
    pub name: String,
    /// Number of parameters declared, including `self`.
    pub param_arity: usize,
    /// Whether the first parameter is `self` (always true for the
    /// MVP; surfaced explicitly so a future "free function in trait"
    /// extension can flip this without a schema change).
    pub takes_self: bool,
    pub span: Span,
    /// RES-2697: parameter names in declaration order (e.g. `["self", "x"]`).
    /// Used to bind arguments when dispatching a default method body.
    pub params: Vec<String>,
    /// RES-2697: optional default body. `None` means the method must be
    /// provided by every `impl` block; `Some` means the impl may omit it
    /// and the default fires instead.
    pub default_body: Option<Box<Node>>,
}

/// Associated type declaration in a trait.
/// RES-779: `type Name;` inside a trait declares a type member that each
/// impl must define.
#[derive(Debug, Clone)]
pub(crate) struct AssociatedTypeDecl {
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub span: Span,
}

/// Parse a `trait` declaration. Called from `parse_statement` when the
/// current token is `Token::Trait`. On entry, `current_token` is
/// `Token::Trait`; on a successful exit the cursor sits on the closing
/// `}` (matching the convention `parse_program` expects).
pub(crate) fn parse(parser: &mut Parser) -> Node {
    let trait_span = parser.span_at_current();
    parser.next_token(); // skip 'trait'

    let name = match &parser.current_token {
        Token::Identifier(n) => n.clone(),
        other => {
            let tok = other.clone();
            parser.record_error(format!("Expected identifier after 'trait', found {}", tok));
            String::new()
        }
    };
    parser.next_token(); // skip name

    // RES-2572: parse optional `extends A + B + C` super-trait list.
    let mut supers: Vec<String> = Vec::new();
    if let Token::Identifier(kw) = &parser.current_token
        && kw == "extends"
    {
        parser.next_token(); // skip "extends"
        loop {
            match &parser.current_token {
                Token::Identifier(super_name) => {
                    supers.push(super_name.clone());
                    parser.next_token();
                    if parser.current_token == Token::Plus {
                        parser.next_token(); // skip '+'
                    } else {
                        break;
                    }
                }
                other => {
                    let tok = other.clone();
                    parser.record_error(format!(
                        "Expected trait name after 'extends', found {}",
                        tok
                    ));
                    break;
                }
            }
        }
    }

    if parser.current_token != Token::LeftBrace {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected '{{' after 'trait {}', found {}",
            name, tok
        ));
    } else {
        parser.next_token(); // skip '{'
    }

    // RES-1802: pre-size to 4 — typical trait declares 1-4 methods
    // and 0-2 associated types. Same fixed-capacity shape as
    // RES-1768 / RES-1776 parser pre-sizes.
    let mut methods: Vec<TraitMethodSig> = Vec::with_capacity(4);
    let mut associated_types: Vec<AssociatedTypeDecl> = Vec::with_capacity(2);

    while parser.current_token != Token::RightBrace && parser.current_token != Token::Eof {
        match &parser.current_token {
            Token::Function => {
                if let Some(sig) = parse_method_sig(parser) {
                    methods.push(sig);
                }
            }
            Token::Type => {
                if let Some(assoc_type) = parse_assoc_type_decl(parser) {
                    associated_types.push(assoc_type);
                }
            }
            _ => {
                let tok = parser.current_token.clone();
                parser.record_error(format!(
                    "Expected 'fn' or 'type' inside trait body, found {}",
                    tok
                ));
                // Recover: jump to the closing brace.
                while parser.current_token != Token::RightBrace
                    && parser.current_token != Token::Eof
                {
                    parser.next_token();
                }
                break;
            }
        }
    }

    Node::TraitDecl {
        name,
        methods,
        associated_types,
        supers,
        span: trait_span,
    }
}

/// Parse a single method signature inside a trait body:
/// `fn name(self, param: Type, ...) -> RetType;` or `fn name(self) {}`
/// (the body is allowed but ignored — keeps the parser tolerant of
/// stub bodies that future tickets may grow into default methods).
fn parse_method_sig(parser: &mut Parser) -> Option<TraitMethodSig> {
    let sig_span = parser.span_at_current();
    parser.next_token(); // skip 'fn'

    let method_name = match &parser.current_token {
        Token::Identifier(n) => n.clone(),
        other => {
            let tok = other.clone();
            parser.record_error(format!("Expected method name in trait body, found {}", tok));
            return None;
        }
    };
    parser.next_token(); // skip method name

    if parser.current_token != Token::LeftParen {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected '(' after method name `{}` in trait body, found {}",
            method_name, tok
        ));
        return None;
    }
    parser.next_token(); // skip '('

    // RES-2697: walk parameter tokens until the matching `)`, collecting
    // param names for default-body dispatch. Arity still drives
    // typecheck coverage; names drive runtime binding.
    let mut depth = 0_i32;
    let mut takes_self = false;
    let mut param_count = 0_usize;
    let mut params: Vec<String> = Vec::new();
    let mut saw_any_token_in_param = false;
    let mut first_param = true;
    let mut at_param_start = true;
    while parser.current_token != Token::Eof {
        match &parser.current_token {
            Token::LeftParen | Token::Less | Token::LeftBracket | Token::LeftBrace => {
                depth += 1;
                saw_any_token_in_param = true;
                at_param_start = false;
                parser.next_token();
            }
            Token::RightParen if depth == 0 => {
                if saw_any_token_in_param {
                    param_count += 1;
                }
                parser.next_token(); // skip ')'
                break;
            }
            Token::RightParen | Token::Greater | Token::RightBracket | Token::RightBrace => {
                depth -= 1;
                saw_any_token_in_param = true;
                at_param_start = false;
                parser.next_token();
            }
            Token::Comma if depth == 0 => {
                if saw_any_token_in_param {
                    param_count += 1;
                }
                saw_any_token_in_param = false;
                first_param = false;
                at_param_start = true;
                parser.next_token();
            }
            Token::Identifier(ident)
                if first_param && !saw_any_token_in_param && ident == "self" =>
            {
                takes_self = true;
                saw_any_token_in_param = true;
                at_param_start = false;
                params.push("self".to_string());
                parser.next_token();
            }
            Token::Identifier(ident) if at_param_start && depth == 0 => {
                params.push(ident.clone());
                saw_any_token_in_param = true;
                at_param_start = false;
                parser.next_token();
            }
            _ => {
                saw_any_token_in_param = true;
                at_param_start = false;
                parser.next_token();
            }
        }
    }

    // Optional `-> ReturnType` — skip until `;` or `{` or `fn` or `}`.
    if parser.current_token == Token::Arrow {
        parser.next_token(); // skip '->'
        // Skip the return-type expression until a terminator.
        while !matches!(
            parser.current_token,
            Token::Semicolon | Token::LeftBrace | Token::RightBrace | Token::Function | Token::Eof
        ) {
            parser.next_token();
        }
    }

    // RES-2697: `;` means abstract signature; `{ ... }` means default body.
    // After parse_block_statement returns the cursor sits ON the closing `}`;
    // advance past it so the outer trait-body loop doesn't mistake it for
    // the trait's own closing brace.
    let default_body = if parser.current_token == Token::Semicolon {
        parser.next_token();
        None
    } else if parser.current_token == Token::LeftBrace {
        let block = parser.parse_block_statement();
        parser.next_token(); // step past the method body's closing `}`
        Some(Box::new(block))
    } else {
        None
    };

    Some(TraitMethodSig {
        name: method_name,
        param_arity: param_count,
        takes_self,
        span: sig_span,
        params,
        default_body,
    })
}

/// Parse a single associated type declaration inside a trait body:
/// `type Name;`
/// On entry, `current_token` is `Token::Type`; on exit, the cursor
/// sits after the semicolon (matching the convention of parse_method_sig).
fn parse_assoc_type_decl(parser: &mut Parser) -> Option<AssociatedTypeDecl> {
    let type_span = parser.span_at_current();
    parser.next_token(); // skip 'type'

    let type_name = match &parser.current_token {
        Token::Identifier(n) => n.clone(),
        other => {
            let tok = other.clone();
            parser.record_error(format!(
                "Expected type name after 'type' in trait body, found {}",
                tok
            ));
            return None;
        }
    };
    parser.next_token(); // skip type name

    // Expect a semicolon after the type name
    if parser.current_token != Token::Semicolon {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected ';' after 'type {}' in trait body, found {}",
            type_name, tok
        ));
    } else {
        parser.next_token(); // skip ';'
    }

    Some(AssociatedTypeDecl {
        name: type_name,
        span: type_span,
    })
}

/// Validate trait declarations, `impl Trait for Type` coverage, and
/// `<T: Trait>` bound usage. Walks the program once collecting traits,
/// then again validating impls and call sites.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    // RES-1391: complete fast-reject. The earlier RES-1230 draft
    // checked only `TraitDecl` and broke the unknown-trait
    // diagnostics, because the pass also validates trait *references*
    // in `impl Trait for Type` blocks and `<T: Trait>` bounds — both
    // of which can appear in programs without any TraitDecl.
    //
    // A correct fast-reject needs all three signals: any TraitDecl,
    // any ImplBlock carrying a `trait_name`, or any Function with
    // type-params / non-empty type-param bounds. Programs without
    // any of these have no trait-validation work to do — Pass 1's
    // collection is empty, Pass 2's impl scan finds nothing to
    // validate, Pass 3 has no bounds to check, and
    // `walk_call_sites` would early-return at
    // `if type_params.is_empty()` for every callee. Same
    // fast-reject pattern as RES-1281 / RES-1290 / RES-1294 /
    // RES-1297 / RES-1311 / RES-1316 / RES-1320 / RES-1376.
    let has_trait_work = stmts.iter().any(|s| match &s.node {
        Node::TraitDecl { .. } => true,
        Node::ImplBlock {
            trait_name: Some(_),
            ..
        } => true,
        Node::Function {
            type_params,
            type_param_bounds,
            ..
        } => !type_params.is_empty() || type_param_bounds.iter().any(|bs| !bs.is_empty()),
        _ => false,
    });
    if !has_trait_work {
        return Ok(());
    }

    // Pass 1: collect trait declarations.
    let mut traits: HashMap<String, (Vec<TraitMethodSig>, Vec<AssociatedTypeDecl>, Span)> =
        HashMap::new();
    for stmt in stmts {
        if let Node::TraitDecl {
            name,
            methods,
            span,
            associated_types,
            ..
        } = &stmt.node
        {
            if name.is_empty() {
                continue;
            }
            if traits.contains_key(name) {
                return Err(format_err(
                    source_path,
                    *span,
                    &format!("duplicate trait declaration `{}`", name),
                ));
            }
            // Within a single trait, method names must be unique.
            // RES-1782: pre-size to methods.len() — exactly one insert
            // per method on the happy path.
            let mut seen = HashSet::with_capacity(methods.len());
            for m in methods {
                if !seen.insert(m.name.as_str()) {
                    return Err(format_err(
                        source_path,
                        m.span,
                        &format!("duplicate method `{}` in trait `{}`", m.name, name),
                    ));
                }
            }
            // Within a single trait, associated type names must be unique.
            // RES-1782: pre-size to associated_types.len() — exactly
            // one insert per associated type on the happy path.
            let mut type_seen = HashSet::with_capacity(associated_types.len());
            for at in associated_types {
                if !type_seen.insert(at.name.as_str()) {
                    return Err(format_err(
                        source_path,
                        at.span,
                        &format!(
                            "duplicate associated type `{}` in trait `{}`",
                            at.name, name
                        ),
                    ));
                }
            }
            traits.insert(
                name.clone(),
                (methods.clone(), associated_types.clone(), *span),
            );
        }
    }
    // If no traits declared, there is nothing more to validate.

    // Index of "type T provides method M with arity A": collected from
    // every `impl T { ... }` block plus every `impl Trait for T { ... }`
    // block. Used both to validate trait-impl coverage and to validate
    // call-site bounds.
    // RES-1802: pre-size to stmts.len() — at most one entry per
    // top-level ImplBlock for each map.
    let mut type_methods: HashMap<String, HashMap<String, usize>> =
        HashMap::with_capacity(stmts.len());
    // Set of explicit `impl Trait for Type` declarations.
    let mut explicit_impls: HashSet<(String, String)> = HashSet::with_capacity(stmts.len());

    for stmt in stmts {
        if let Node::ImplBlock {
            trait_name,
            struct_name,
            methods,
            ..
        } = &stmt.node
        {
            let entry = type_methods.entry(struct_name.clone()).or_default();
            for m in methods {
                if let Node::Function {
                    name, parameters, ..
                } = m
                {
                    // Methods are mangled `<Type>$<method>` — strip the prefix.
                    let plain = name
                        .strip_prefix(&format!("{}$", struct_name))
                        .unwrap_or(name);
                    entry.insert(plain.to_string(), parameters.len());
                }
            }
            if let Some(t) = trait_name {
                explicit_impls.insert((t.clone(), struct_name.clone()));
            }
        }
    }

    // Pass 2: validate every `impl Trait for Type` covers the trait.
    for stmt in stmts {
        if let Node::ImplBlock {
            trait_name: Some(t),
            struct_name,
            methods,
            span,
            associated_type_impls,
        } = &stmt.node
        {
            let (trait_methods, trait_assoc_types, _trait_span) = match traits.get(t) {
                Some(v) => v,
                None => {
                    return Err(format_err(
                        source_path,
                        *span,
                        &format!("unknown trait `{}` in `impl {} for {}`", t, t, struct_name),
                    ));
                }
            };

            // Build the set of methods this impl block actually provides.
            // RES-1802: pre-size to methods.len() — one insert per method.
            let mut provided: HashMap<String, usize> = HashMap::with_capacity(methods.len());
            for m in methods {
                if let Node::Function {
                    name, parameters, ..
                } = m
                {
                    let plain = name
                        .strip_prefix(&format!("{}$", struct_name))
                        .unwrap_or(name);
                    provided.insert(plain.to_string(), parameters.len());
                }
            }

            // Validate methods.
            for sig in trait_methods {
                match provided.get(&sig.name) {
                    None => {
                        // RES-2697: a missing method is OK if the trait
                        // declares a default body for it.
                        if sig.default_body.is_some() {
                            continue;
                        }
                        return Err(format_err(
                            source_path,
                            *span,
                            &format!(
                                "impl `{}` for `{}` is missing method `{}` declared by trait `{}`",
                                t, struct_name, sig.name, t
                            ),
                        ));
                    }
                    Some(&arity) if arity != sig.param_arity => {
                        return Err(format_err(
                            source_path,
                            *span,
                            &format!(
                                "impl `{}` for `{}` method `{}` has {} parameter(s); trait `{}` declares {}",
                                t, struct_name, sig.name, arity, t, sig.param_arity
                            ),
                        ));
                    }
                    Some(_) => {}
                }
            }

            // Build the set of associated types this impl block defines.
            // RES-1802: pre-size to associated_type_impls.len() — one
            // insert per associated_type impl on the happy path.
            let mut provided_types: HashSet<String> =
                HashSet::with_capacity(associated_type_impls.len());
            for (type_name, _type_expr) in associated_type_impls {
                provided_types.insert(type_name.clone());
            }

            // Validate that all trait-declared associated types are provided.
            for at_decl in trait_assoc_types {
                if !provided_types.contains(&at_decl.name) {
                    return Err(format_err(
                        source_path,
                        *span,
                        &format!(
                            "impl `{}` for `{}` is missing associated type `{}` declared by trait `{}`",
                            t, struct_name, at_decl.name, t
                        ),
                    ));
                }
            }
        }
    }

    // Pass 3: validate generic-bound annotations refer to known traits,
    // and that direct calls passing a struct-typed literal as a bounded
    // type parameter satisfy the bound.
    // RES-1500: borrow each function name as `&str` instead of
    // cloning it into the HashMap key. The map's only consumer is
    // `walk_call_sites`, which performs `fns_by_name.get(callee_name)`
    // with a `&str` lookup (RES-1483). The owned `String` key was
    // pure overhead — same pattern applied to `power_contracts` /
    // `wcet_contracts` / `stack_contracts` in RES-1495.
    let fns_by_name: HashMap<&str, &Node> = stmts
        .iter()
        .filter_map(|s| {
            if let Node::Function { name, .. } = &s.node {
                Some((name.as_str(), &s.node as &Node))
            } else {
                None
            }
        })
        .collect();

    // Validate that every bound in every fn signature names a real trait.
    for stmt in stmts {
        if let Node::Function {
            name: fn_name,
            type_params,
            type_param_bounds,
            ..
        } = &stmt.node
        {
            for (i, tp) in type_params.iter().enumerate() {
                if let Some(bs) = type_param_bounds.get(i) {
                    for b in bs {
                        // RES-2695: projection bounds like "I::Item:Display" are
                        // not direct trait names — skip the existence check here
                        // and validate them in walk_call_sites instead.
                        if b.contains("::") {
                            continue;
                        }
                        if !traits.contains_key(b) {
                            return Err(format_err(
                                source_path,
                                stmt.span,
                                &format!(
                                    "unknown trait `{}` in bound `{}: {}` on fn `{}`",
                                    b, tp, b, fn_name
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }

    // RES-2695: build the associated-type map so projection bounds can be
    // validated at call sites: (trait_name, struct_name, assoc_type_name) → type.
    let assoc_type_map = build_assoc_type_map(program).unwrap_or_default();

    // Validate call sites where the argument is a struct literal — the
    // case where the type is concretely determinable. For other forms,
    // we skip silently (matching `generics.rs` philosophy: dynamic
    // interpreter, lightweight check).
    walk_call_sites(
        program,
        &fns_by_name,
        &traits,
        &type_methods,
        &explicit_impls,
        &assoc_type_map,
        source_path,
    )?;

    Ok(())
}

/// Recursively walk the program collecting `CallExpression` nodes
/// whose callee is a known generic function. For each, examine the
/// arguments: if an argument is a `StructLiteral { name, .. }`,
/// confirm that `name` satisfies any bound declared on the
/// corresponding type parameter.
fn walk_call_sites(
    node: &Node,
    fns_by_name: &HashMap<&str, &Node>,
    traits: &HashMap<String, (Vec<TraitMethodSig>, Vec<AssociatedTypeDecl>, Span)>,
    type_methods: &HashMap<String, HashMap<String, usize>>,
    explicit_impls: &HashSet<(String, String)>,
    assoc_type_map: &HashMap<(String, String, String), String>,
    source_path: &str,
) -> Result<(), String> {
    let ctx = CallSiteCheckCtx {
        fns_by_name,
        traits,
        type_methods,
        explicit_impls,
        assoc_type_map,
        source_path,
    };
    let mut bindings = vec![HashMap::new()];
    walk_call_sites_with_bindings(node, &ctx, &mut bindings)
}

struct CallSiteCheckCtx<'a> {
    fns_by_name: &'a HashMap<&'a str, &'a Node>,
    traits: &'a HashMap<String, (Vec<TraitMethodSig>, Vec<AssociatedTypeDecl>, Span)>,
    type_methods: &'a HashMap<String, HashMap<String, usize>>,
    explicit_impls: &'a HashSet<(String, String)>,
    assoc_type_map: &'a HashMap<(String, String, String), String>,
    source_path: &'a str,
}

fn walk_call_sites_with_bindings(
    node: &Node,
    ctx: &CallSiteCheckCtx<'_>,
    bindings: &mut Vec<HashMap<String, String>>,
) -> Result<(), String> {
    match node {
        Node::Program(stmts) => {
            bindings.push(HashMap::new());
            for s in stmts {
                walk_call_sites_with_bindings(&s.node, ctx, bindings)?;
            }
            bindings.pop();
        }
        Node::Block { stmts, .. } => {
            bindings.push(HashMap::new());
            for s in stmts {
                walk_call_sites_with_bindings(s, ctx, bindings)?;
            }
            bindings.pop();
        }
        Node::Function { body, .. } => {
            bindings.push(HashMap::new());
            walk_call_sites_with_bindings(body, ctx, bindings)?;
            bindings.pop();
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                walk_call_sites_with_bindings(m, ctx, bindings)?;
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            walk_call_sites_with_bindings(expr, ctx, bindings)?;
        }
        Node::LetStatement { name, value, .. } => {
            walk_call_sites_with_bindings(value, ctx, bindings)?;
            if let Some(concrete_type) = resolve_concrete_type(value, bindings) {
                bindings
                    .last_mut()
                    .expect("call-site binding scope should exist")
                    .insert(name.clone(), concrete_type);
            }
        }
        Node::Assignment { name, value, .. } => {
            walk_call_sites_with_bindings(value, ctx, bindings)?;
            if let Some(concrete_type) = resolve_concrete_type(value, bindings) {
                if let Some(scope) = bindings
                    .iter_mut()
                    .rev()
                    .find(|scope| scope.contains_key(name))
                {
                    scope.insert(name.clone(), concrete_type);
                } else {
                    bindings
                        .last_mut()
                        .expect("call-site binding scope should exist")
                        .insert(name.clone(), concrete_type);
                }
            }
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            walk_call_sites_with_bindings(v, ctx, bindings)?;
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_call_sites_with_bindings(condition, ctx, bindings)?;
            walk_call_sites_with_bindings(consequence, ctx, bindings)?;
            if let Some(alt) = alternative {
                walk_call_sites_with_bindings(alt, ctx, bindings)?;
            }
        }
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            // Recurse into nested calls / arguments first.
            walk_call_sites_with_bindings(function, ctx, bindings)?;
            for a in arguments {
                walk_call_sites_with_bindings(a, ctx, bindings)?;
            }

            // Identify the callee by name.
            // RES-1483: borrow the callee name as `&str` from the
            // call's `function` sub-node. The previous shape did
            // `name.clone()` to produce an owned `String`, then used
            // `&callee_name` for the `HashMap::get` lookup (which
            // accepts `&str` via `Borrow<str>`) and finally embedded
            // it in the rare error-message `format!`. `fns_by_name`
            // looks up natively against `&str`; `format!` accepts
            // any `Display`. The owned `String` was pure waste on
            // every call-expression walk.
            let callee_name: &str = match function.as_ref() {
                Node::Identifier { name, .. } => name.as_str(),
                _ => return Ok(()),
            };
            let callee = match ctx.fns_by_name.get(callee_name) {
                Some(c) => *c,
                None => return Ok(()),
            };
            let (type_params, type_param_bounds, parameters) = match callee {
                Node::Function {
                    type_params,
                    type_param_bounds,
                    parameters,
                    ..
                } => (type_params, type_param_bounds, parameters),
                _ => return Ok(()),
            };
            if type_params.is_empty() {
                return Ok(());
            }

            // For each type parameter with bounds, find any positional
            // argument whose declared type is the type parameter, and
            // — if the argument is a struct literal — confirm the bound.
            for (i, tp) in type_params.iter().enumerate() {
                let bounds = match type_param_bounds.get(i) {
                    Some(b) if !b.is_empty() => b,
                    _ => continue,
                };
                for (pi, (ptype, _pname)) in parameters.iter().enumerate() {
                    if ptype != tp {
                        continue;
                    }
                    let arg = match arguments.get(pi) {
                        Some(a) => a,
                        None => continue,
                    };
                    let concrete_type = resolve_concrete_type(arg, bindings);
                    if let Some(ct) = concrete_type {
                        for bound in bounds {
                            // RES-2695: projection bound like "I::Item:Display"
                            // — parse "Param::AssocType:Trait" and validate.
                            if bound.contains("::") {
                                // rsplit_once(':') splits at the LAST ':', giving
                                // projection="I::Item" and trait_bound="Display".
                                // split_once("::") on the projection gives the assoc name.
                                let parsed = bound.rsplit_once(':').and_then(|(proj, tb)| {
                                    proj.split_once("::").map(|(_, a)| (a, tb))
                                });
                                if let Some((assoc_name, trait_bound)) = parsed {
                                    let concrete_assoc = ctx
                                        .assoc_type_map
                                        .iter()
                                        .find(|((_, s, a), _)| {
                                            s.as_str() == ct && a.as_str() == assoc_name
                                        })
                                        .map(|(_, v)| v.as_str());
                                    match concrete_assoc {
                                        None => {
                                            return Err(format_err(
                                                ctx.source_path,
                                                *span,
                                                &format!(
                                                    "type `{}` does not define associated type `{}` required by bound `{}` at call to `{}`",
                                                    ct, assoc_name, bound, callee_name
                                                ),
                                            ));
                                        }
                                        Some(concrete_type) => {
                                            let satisfied = ctx.explicit_impls.contains(&(
                                                trait_bound.to_string(),
                                                concrete_type.to_string(),
                                            )) || trait_satisfied_structurally(
                                                trait_bound,
                                                concrete_type,
                                                ctx.traits,
                                                ctx.type_methods,
                                            );
                                            if !satisfied {
                                                return Err(format_err(
                                                    ctx.source_path,
                                                    *span,
                                                    &format!(
                                                        "associated type `{}::{}` = `{}` does not satisfy bound `{}` at call to `{}`",
                                                        ct,
                                                        assoc_name,
                                                        concrete_type,
                                                        trait_bound,
                                                        callee_name
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                }
                                continue; // projection bound handled above
                            }
                            let satisfied =
                                ctx.explicit_impls.contains(&(bound.clone(), ct.clone()))
                                    || trait_satisfied_structurally(
                                        bound,
                                        &ct,
                                        ctx.traits,
                                        ctx.type_methods,
                                    );
                            if !satisfied {
                                return Err(format_err(
                                    ctx.source_path,
                                    *span,
                                    &format!(
                                        "type `{}` does not satisfy bound `{}: {}` at call to `{}` (no `impl {} for {}` and required methods are missing)",
                                        ct, tp, bound, callee_name, bound, ct
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn resolve_concrete_type(node: &Node, bindings: &[HashMap<String, String>]) -> Option<String> {
    match node {
        Node::StructLiteral { name, .. } => Some(name.clone()),
        Node::Identifier { name, .. } => bindings
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned()),
        _ => None,
    }
}

fn trait_satisfied_structurally(
    trait_name: &str,
    type_name: &str,
    traits: &HashMap<String, (Vec<TraitMethodSig>, Vec<AssociatedTypeDecl>, Span)>,
    type_methods: &HashMap<String, HashMap<String, usize>>,
) -> bool {
    let methods = match traits.get(trait_name) {
        Some((m, _, _)) => m,
        None => return false,
    };
    let provided = match type_methods.get(type_name) {
        Some(p) => p,
        None => return false,
    };
    methods.iter().all(|sig| {
        provided
            .get(&sig.name)
            .is_some_and(|arity| *arity == sig.param_arity)
    })
}

/// RES-783 PR2: Build a map of concrete associated type definitions.
/// Maps (trait_name, struct_name, assoc_type_name) -> type_expr_string.
/// Returns (trait_name, struct_name) pairs with missing/invalid definitions as errors.
#[allow(dead_code)]
pub fn build_assoc_type_map(
    program: &Node,
) -> Result<HashMap<(String, String, String), String>, String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(HashMap::new()),
    };

    let mut assoc_type_map: HashMap<(String, String, String), String> = HashMap::new();

    // Walk through all impl blocks and collect their associated type definitions.
    for stmt in stmts {
        if let Node::ImplBlock {
            trait_name: Some(t),
            struct_name,
            associated_type_impls,
            ..
        } = &stmt.node
        {
            for (type_name, type_expr) in associated_type_impls {
                let key = (t.clone(), struct_name.clone(), type_name.clone());
                assoc_type_map.insert(key, normalize_assoc_type_expr(type_expr));
            }
        }
    }

    Ok(assoc_type_map)
}

fn normalize_assoc_type_expr(type_expr: &str) -> String {
    type_expr
        .strip_prefix("identifier `")
        .and_then(|rest| rest.strip_suffix('`'))
        .unwrap_or(type_expr)
        .to_string()
}

fn format_err(source_path: &str, span: Span, msg: &str) -> String {
    if span.start.line == 0 {
        msg.to_string()
    } else {
        format!(
            "{}:{}:{}: {}",
            source_path, span.start.line, span.start.column, msg
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn parse_program(src: &str) -> Node {
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    #[test]
    fn declares_trait_with_single_method() {
        let prog = parse_program(
            "trait Printable { fn to_string(self) -> string; }\nfn main(int dummy) {} main();",
        );
        let stmts = match &prog {
            Node::Program(s) => s,
            _ => panic!("expected program"),
        };
        let trait_decl = stmts
            .iter()
            .find_map(|s| match &s.node {
                Node::TraitDecl { name, methods, .. } => Some((name.clone(), methods.len())),
                _ => None,
            })
            .expect("trait decl");
        assert_eq!(trait_decl.0, "Printable");
        assert_eq!(trait_decl.1, 1);
    }

    #[test]
    fn impl_trait_for_type_records_trait_name() {
        let prog = parse_program(
            "trait Printable { fn to_string(self) -> string; }\n\
             struct Point { int x, int y, }\n\
             impl Printable for Point { fn to_string(self) -> string { return \"p\"; } }\n\
             fn main(int dummy) {} main();",
        );
        let stmts = match &prog {
            Node::Program(s) => s,
            _ => panic!("expected program"),
        };
        let found = stmts.iter().any(|s| {
            matches!(
                &s.node,
                Node::ImplBlock {
                    trait_name: Some(t),
                    struct_name,
                    ..
                } if t == "Printable" && struct_name == "Point"
            )
        });
        assert!(
            found,
            "expected ImplBlock with trait_name = Some(Printable)"
        );
    }

    #[test]
    fn missing_method_in_impl_yields_clear_error() {
        let prog = parse_program(
            "trait Printable { fn to_string(self) -> string; }\n\
             struct Point { int x, int y, }\n\
             impl Printable for Point { }\n\
             fn main(int dummy) {} main();",
        );
        let err = check(&prog, "test.rz").expect_err("expected missing-method error");
        assert!(err.contains("missing method `to_string`"), "got: {err}");
        assert!(err.contains("Printable"), "got: {err}");
        assert!(err.contains("Point"), "got: {err}");
    }

    #[test]
    fn unknown_trait_in_impl_errors() {
        let prog = parse_program(
            "struct Point { int x, }\n\
             impl Drawable for Point { fn draw(self) -> int { return 0; } }\n\
             fn main(int dummy) {} main();",
        );
        let err = check(&prog, "test.rz").expect_err("expected unknown-trait error");
        assert!(err.contains("unknown trait `Drawable`"), "got: {err}");
    }

    #[test]
    fn unknown_trait_in_bound_errors() {
        let prog = parse_program(
            "fn pick<T: Comparable>(int a) -> int { return a; }\n\
             fn main(int dummy) {} main();",
        );
        let err = check(&prog, "test.rz").expect_err("expected unknown-bound error");
        assert!(err.contains("unknown trait `Comparable`"), "got: {err}");
    }

    #[test]
    fn duplicate_trait_decl_errors() {
        let prog = parse_program(
            "trait Eq { fn eq(self, int other) -> bool; }\n\
             trait Eq { fn eq(self, int other) -> bool; }\n\
             fn main(int dummy) {} main();",
        );
        let err = check(&prog, "test.rz").expect_err("expected duplicate-trait error");
        assert!(
            err.contains("duplicate trait declaration `Eq`"),
            "got: {err}"
        );
    }

    #[test]
    fn trait_method_arity_mismatch_errors() {
        let prog = parse_program(
            "trait Cmp { fn cmp(self, int other) -> int; }\n\
             struct S { int x, }\n\
             impl Cmp for S { fn cmp(self) -> int { return 0; } }\n\
             fn main(int dummy) {} main();",
        );
        let err = check(&prog, "test.rz").expect_err("expected arity-mismatch error");
        assert!(err.contains("`cmp`"), "got: {err}");
        assert!(err.contains("parameter"), "got: {err}");
    }

    #[test]
    fn generic_call_with_satisfied_bound_passes() {
        let prog = parse_program(
            "trait Tag { fn tag(self) -> string; }\n\
             struct S { int x, }\n\
             impl Tag for S { fn tag(self) -> string { return \"s\"; } }\n\
             fn announce<T: Tag>(T item) -> string { return \"x\"; }\n\
             fn main(int dummy) { announce(new S { x: 1 }); } main();",
        );
        check(&prog, "test.rz").expect("bound should be satisfied");
    }

    #[test]
    fn generic_call_with_unsatisfied_bound_errors() {
        let prog = parse_program(
            "trait Tag { fn tag(self) -> string; }\n\
             struct S { int x, }\n\
             fn announce<T: Tag>(T item) -> string { return \"x\"; }\n\
             fn main(int dummy) { announce(new S { x: 1 }); } main();",
        );
        let err = check(&prog, "test.rz").expect_err("expected unsatisfied-bound error");
        assert!(err.contains("does not satisfy bound"), "got: {err}");
        assert!(err.contains("Tag"), "got: {err}");
        assert!(err.contains("S"), "got: {err}");
    }

    #[test]
    fn empty_program_passes() {
        let prog = Node::Program(Vec::new());
        check(&prog, "test.rz").expect("empty program should pass");
    }

    #[test]
    fn span_is_passed_through_in_error_messages() {
        let prog = parse_program(
            "fn pick<T: Nope>(int a) -> int { return a; }\nfn main(int dummy) {} main();",
        );
        let err = check(&prog, "src.rz").expect_err("expected error");
        assert!(
            err.contains("src.rz:"),
            "expected source-prefixed span, got: {err}"
        );
    }

    #[test]
    fn trait_method_sig_default_span_does_not_panic() {
        // Sanity: format_err with line == 0 returns the message verbatim.
        let s = format_err("x.rz", Span::default(), "oops");
        assert_eq!(s, "oops");
    }

    #[test]
    fn trait_with_associated_type_parses() {
        let prog = parse_program(
            "trait Transport { type Message; fn send(self) -> int; }\nfn main(int dummy) {} main();",
        );
        let stmts = match &prog {
            Node::Program(s) => s,
            _ => panic!("expected program"),
        };
        let trait_decl = stmts
            .iter()
            .find_map(|s| match &s.node {
                Node::TraitDecl {
                    name,
                    methods,
                    associated_types,
                    ..
                } => Some((name.clone(), methods.len(), associated_types.len())),
                _ => None,
            })
            .expect("trait decl");
        assert_eq!(trait_decl.0, "Transport");
        assert_eq!(trait_decl.1, 1);
        assert_eq!(trait_decl.2, 1);
    }

    #[test]
    fn trait_with_multiple_associated_types_parses() {
        let prog = parse_program(
            "trait Transport { type Message; type Error; fn send(self) -> int; }\nfn main(int dummy) {} main();",
        );
        let stmts = match &prog {
            Node::Program(s) => s,
            _ => panic!("expected program"),
        };
        let trait_decl = stmts
            .iter()
            .find_map(|s| match &s.node {
                Node::TraitDecl {
                    associated_types, ..
                } => Some(associated_types.len()),
                _ => None,
            })
            .expect("trait decl");
        assert_eq!(trait_decl, 2);
    }

    #[test]
    fn duplicate_associated_type_in_trait_errors() {
        let prog = parse_program("trait T { type X; type X; }\nfn main(int dummy) {} main();");
        let err = check(&prog, "test.rz").expect_err("expected duplicate-assoc-type error");
        assert!(err.contains("duplicate associated type `X`"), "got: {err}");
    }

    #[test]
    fn impl_missing_associated_type_errors() {
        let prog = parse_program(
            "trait Transport { type Message; fn send(self) -> int; }\n\
             struct Serial { int x, }\n\
             impl Transport for Serial { fn send(self) -> int { return 0; } }\n\
             fn main(int dummy) {} main();",
        );
        let err = check(&prog, "test.rz").expect_err("expected missing-assoc-type error");
        assert!(
            err.contains("missing associated type `Message`"),
            "got: {err}"
        );
        assert!(err.contains("Transport"), "got: {err}");
        assert!(err.contains("Serial"), "got: {err}");
    }

    #[test]
    fn build_assoc_type_map_collects_definitions() {
        let prog = parse_program(
            "trait Transport { type Message; type Error; fn send(self) -> int; }\n\
             struct Serial { int x, }\n\
             impl Transport for Serial {\n\
                 type Message = [u8; 64];\n\
                 type Error = int;\n\
                 fn send(self) -> int { return 0; }\n\
             }\n\
             fn main(int dummy) {} main();",
        );
        let map = build_assoc_type_map(&prog).expect("should build map");
        let msg_key = (
            "Transport".to_string(),
            "Serial".to_string(),
            "Message".to_string(),
        );
        let err_key = (
            "Transport".to_string(),
            "Serial".to_string(),
            "Error".to_string(),
        );
        assert!(map.contains_key(&msg_key), "should have Message type");
        assert!(map.contains_key(&err_key), "should have Error type");
    }

    // RES-2695: projection bound checking tests.

    #[test]
    fn projection_bound_no_false_positive_for_valid_program() {
        // A function with `where I::Item: Show` should typecheck cleanly
        // when the concrete struct's Item implements Show.
        let src = "trait Show { fn show(self) -> string; }\n\
             trait Iter { type Item; fn next(self) -> int; }\n\
             struct Num { int v }\n\
             struct List { int n }\n\
             impl Show for Num { fn show(self) -> string { return \"n\"; } }\n\
             impl Iter for List {\n\
                 type Item = Num;\n\
                 fn next(self) -> int { return self.n; }\n\
             }\n\
             fn<I> collect(I it) where I::Item: Show { println(it.next()); }\n\
             let sl = new List { n: 0 };\n\
             collect(sl);\n";
        let prog = parse_program(src);
        check(&prog, "test.rz").unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn projection_bound_errors_when_assoc_type_missing() {
        // Struct's impl doesn't define the associated type — should error.
        // Pass a struct literal so walk_call_sites sees the concrete type.
        let src = "trait Show { fn show(self) -> string; }\n\
             trait Iter { type Item; fn next(self) -> int; }\n\
             struct Widget { int x }\n\
             impl Iter for Widget { fn next(self) -> int { return self.x; } }\n\
             fn<I> collect(I it) where I::Item: Show { println(it.next()); }\n\
             collect(new Widget { x: 1 });\n";
        let prog = parse_program(src);
        let e = check(&prog, "test.rz").expect_err("expected missing-assoc-type error");
        assert!(e.contains("Item"), "got: {e}");
        assert!(e.contains("Widget"), "got: {e}");
    }

    #[test]
    fn projection_bound_errors_when_assoc_type_does_not_satisfy_bound() {
        // Struct defines Item = BadType, but BadType doesn't implement Show.
        // Pass a struct literal directly so walk_call_sites can see the concrete type.
        let src = "trait Show { fn show(self) -> string; }\n\
             trait Iter { type Item; fn next(self) -> int; }\n\
             struct BadType { int x }\n\
             struct List { int n }\n\
             impl Iter for List {\n\
                 type Item = BadType;\n\
                 fn next(self) -> int { return self.n; }\n\
             }\n\
             fn<I> collect(I it) where I::Item: Show { println(it.next()); }\n\
             collect(new List { n: 0 });\n";
        let prog = parse_program(src);
        let e = check(&prog, "test.rz").expect_err("expected bound-violation error");
        assert!(e.contains("BadType"), "got: {e}");
        assert!(e.contains("Show"), "got: {e}");
    }

    // RES-2697: default trait method body tests.

    #[test]
    fn default_method_parses_body() {
        let src = "trait Greet {\
             fn greet(self) -> string;\
             fn greet_loudly(self) -> string { return \"loud\"; }\
             }\
             struct A { int x }\
             impl Greet for A { fn greet(self) -> string { return \"hi\"; } }\
             fn main(int d) {} main();";
        let prog = parse_program(src);
        let stmts = match &prog {
            Node::Program(s) => s,
            _ => panic!("expected program"),
        };
        let loudly_has_default = stmts
            .iter()
            .find_map(|s| match &s.node {
                Node::TraitDecl { methods, .. } => methods
                    .iter()
                    .find(|m| m.name == "greet_loudly")
                    .map(|m| m.default_body.is_some()),
                _ => None,
            })
            .expect("trait decl with greet_loudly");
        assert!(
            loudly_has_default,
            "greet_loudly should have a default body"
        );
    }

    #[test]
    fn default_method_allows_missing_impl() {
        // Impl omits greet_loudly; trait has a default — should not error.
        let src = "trait Greet {\
             fn greet(self) -> string;\
             fn greet_loudly(self) -> string { return \"loud\"; }\
             }\
             struct A { int x }\
             impl Greet for A { fn greet(self) -> string { return \"hi\"; } }\
             fn main(int d) {} main();";
        let prog = parse_program(src);
        check(&prog, "test.rz").unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn abstract_method_without_default_still_required() {
        // `greet` has no default — impl must provide it.
        let src = "trait Greet {\
             fn greet(self) -> string;\
             fn greet_loudly(self) -> string { return \"loud\"; }\
             }\
             struct A { int x }\
             impl Greet for A { fn greet_loudly(self) -> string { return \"HI\"; } }\
             fn main(int d) {} main();";
        let prog = parse_program(src);
        let e = check(&prog, "test.rz").expect_err("expected missing-method error");
        assert!(e.contains("greet"), "got: {e}");
    }

    #[test]
    fn default_method_fires_at_runtime() {
        // The interpreter must dispatch greet_loudly via the default body.
        let src = "trait Greet {\
             fn greet(self) -> string;\
             fn greet_loudly(self) -> string { return self.greet() + \"!!!\"; }\
             }\
             struct Alice { string name }\
             impl Greet for Alice {\
             fn greet(self) -> string { return \"Hi \" + self.name; }\
             }\
             let a = new Alice { name: \"Alice\" };\
             a.greet_loudly();\
             ";
        let result = crate::run_program(src);
        assert!(result.ok, "runtime failed: {:?}", result.errors);
    }

    #[test]
    fn explicit_override_replaces_default() {
        // Bob overrides greet_loudly; his version must be used, not the default.
        let src = "trait Greet {\
             fn greet(self) -> string;\
             fn greet_loudly(self) -> string { return self.greet() + \"!!!\"; }\
             }\
             struct Bob { string name }\
             impl Greet for Bob {\
             fn greet(self) -> string { return \"Hi \" + self.name; }\
             fn greet_loudly(self) -> string { return \"HEY \" + self.name; }\
             }\
             let b = new Bob { name: \"Bob\" };\
             println(b.greet_loudly());";
        let result = crate::run_program(src);
        assert!(result.ok, "runtime failed: {:?}", result.errors);
        assert!(
            result.stdout.contains("HEY Bob"),
            "got: {:?}",
            result.stdout
        );
    }
}
