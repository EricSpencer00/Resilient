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

    if parser.current_token != Token::LeftBrace {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected '{{' after 'trait {}', found {}",
            name, tok
        ));
    } else {
        parser.next_token(); // skip '{'
    }

    let mut methods: Vec<TraitMethodSig> = Vec::new();
    let mut associated_types: Vec<AssociatedTypeDecl> = Vec::new();
    while parser.current_token != Token::RightBrace && parser.current_token != Token::Eof {
        match parser.current_token {
            Token::Function => {
                if let Some(sig) = parse_method_sig(parser) {
                    methods.push(sig);
                }
            }
            Token::Type => {
                if let Some(assoc_ty) = parse_associated_type_decl(parser) {
                    associated_types.push(assoc_ty);
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

    // Walk parameter tokens until the matching `)`. We don't need full
    // type-checking here; arity is what matters for dispatch
    // validation. We split on commas at depth 0 so that nested generic
    // args / parens don't confuse the count.
    let mut depth = 0_i32;
    let mut takes_self = false;
    let mut param_count = 0_usize;
    let mut saw_any_token_in_param = false;
    let mut first_param = true;
    while parser.current_token != Token::Eof {
        match &parser.current_token {
            Token::LeftParen | Token::Less | Token::LeftBracket | Token::LeftBrace => {
                depth += 1;
                saw_any_token_in_param = true;
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
                parser.next_token();
            }
            Token::Comma if depth == 0 => {
                if saw_any_token_in_param {
                    param_count += 1;
                }
                saw_any_token_in_param = false;
                first_param = false;
                parser.next_token();
            }
            Token::Identifier(ident)
                if first_param && !saw_any_token_in_param && ident == "self" =>
            {
                takes_self = true;
                saw_any_token_in_param = true;
                parser.next_token();
            }
            _ => {
                saw_any_token_in_param = true;
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

    // Tolerate either `;` (signature only) or `{ ... }` (stub body — ignored).
    if parser.current_token == Token::Semicolon {
        parser.next_token();
    } else if parser.current_token == Token::LeftBrace {
        // Skip the body to the matching closing brace.
        let mut brace_depth = 1_i32;
        parser.next_token(); // skip '{'
        while brace_depth > 0 && parser.current_token != Token::Eof {
            match parser.current_token {
                Token::LeftBrace => brace_depth += 1,
                Token::RightBrace => brace_depth -= 1,
                _ => {}
            }
            parser.next_token();
        }
    }

    Some(TraitMethodSig {
        name: method_name,
        param_arity: param_count,
        takes_self,
        span: sig_span,
    })
}

/// Parse a single associated type declaration inside a trait body:
/// `type Name;` RES-779
fn parse_associated_type_decl(parser: &mut Parser) -> Option<AssociatedTypeDecl> {
    let decl_span = parser.span_at_current();
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

    // Expect semicolon
    if parser.current_token != Token::Semicolon {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected ';' after associated type `{}` in trait body, found {}",
            type_name, tok
        ));
    } else {
        parser.next_token(); // skip ';'
    }

    Some(AssociatedTypeDecl {
        name: type_name,
        span: decl_span,
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

    // Pass 1: collect trait declarations.
    let mut traits: HashMap<String, (Vec<TraitMethodSig>, Span)> = HashMap::new();
    for stmt in stmts {
        if let Node::TraitDecl {
            name,
            methods,
            span,
            associated_types: _,
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
            let mut seen = HashSet::new();
            for m in methods {
                if !seen.insert(m.name.as_str()) {
                    return Err(format_err(
                        source_path,
                        m.span,
                        &format!("duplicate method `{}` in trait `{}`", m.name, name),
                    ));
                }
            }
            traits.insert(name.clone(), (methods.clone(), *span));
        }
    }
    // If no traits declared, there is nothing more to validate.

    // Index of "type T provides method M with arity A": collected from
    // every `impl T { ... }` block plus every `impl Trait for T { ... }`
    // block. Used both to validate trait-impl coverage and to validate
    // call-site bounds.
    let mut type_methods: HashMap<String, HashMap<String, usize>> = HashMap::new();
    // Set of explicit `impl Trait for Type` declarations.
    let mut explicit_impls: HashSet<(String, String)> = HashSet::new();

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
            associated_type_impls: _,
        } = &stmt.node
        {
            let (trait_methods, _trait_span) = match traits.get(t) {
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
            let mut provided: HashMap<String, usize> = HashMap::new();
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

            for sig in trait_methods {
                match provided.get(&sig.name) {
                    None => {
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
        }
    }

    // Pass 3: validate generic-bound annotations refer to known traits,
    // and that direct calls passing a struct-typed literal as a bounded
    // type parameter satisfy the bound.
    let fns_by_name: HashMap<String, &Node> = stmts
        .iter()
        .filter_map(|s| {
            if let Node::Function { name, .. } = &s.node {
                Some((name.clone(), &s.node as &Node))
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
    fns_by_name: &HashMap<String, &Node>,
    traits: &HashMap<String, (Vec<TraitMethodSig>, Span)>,
    type_methods: &HashMap<String, HashMap<String, usize>>,
    explicit_impls: &HashSet<(String, String)>,
    source_path: &str,
) -> Result<(), String> {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_call_sites(
                    &s.node,
                    fns_by_name,
                    traits,
                    type_methods,
                    explicit_impls,
                    source_path,
                )?;
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_call_sites(
                    s,
                    fns_by_name,
                    traits,
                    type_methods,
                    explicit_impls,
                    source_path,
                )?;
            }
        }
        Node::Function { body, .. } => {
            walk_call_sites(
                body,
                fns_by_name,
                traits,
                type_methods,
                explicit_impls,
                source_path,
            )?;
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                walk_call_sites(
                    m,
                    fns_by_name,
                    traits,
                    type_methods,
                    explicit_impls,
                    source_path,
                )?;
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            walk_call_sites(
                expr,
                fns_by_name,
                traits,
                type_methods,
                explicit_impls,
                source_path,
            )?;
        }
        Node::LetStatement { value, .. } => {
            walk_call_sites(
                value,
                fns_by_name,
                traits,
                type_methods,
                explicit_impls,
                source_path,
            )?;
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            walk_call_sites(
                v,
                fns_by_name,
                traits,
                type_methods,
                explicit_impls,
                source_path,
            )?;
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_call_sites(
                condition,
                fns_by_name,
                traits,
                type_methods,
                explicit_impls,
                source_path,
            )?;
            walk_call_sites(
                consequence,
                fns_by_name,
                traits,
                type_methods,
                explicit_impls,
                source_path,
            )?;
            if let Some(alt) = alternative {
                walk_call_sites(
                    alt,
                    fns_by_name,
                    traits,
                    type_methods,
                    explicit_impls,
                    source_path,
                )?;
            }
        }
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            // Recurse into nested calls / arguments first.
            walk_call_sites(
                function,
                fns_by_name,
                traits,
                type_methods,
                explicit_impls,
                source_path,
            )?;
            for a in arguments {
                walk_call_sites(
                    a,
                    fns_by_name,
                    traits,
                    type_methods,
                    explicit_impls,
                    source_path,
                )?;
            }

            // Identify the callee by name.
            let callee_name = match function.as_ref() {
                Node::Identifier { name, .. } => name.clone(),
                _ => return Ok(()),
            };
            let callee = match fns_by_name.get(&callee_name) {
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
                    let concrete_type = match arg {
                        Node::StructLiteral { name, .. } => Some(name.clone()),
                        _ => None,
                    };
                    if let Some(ct) = concrete_type {
                        for bound in bounds {
                            let satisfied = explicit_impls.contains(&(bound.clone(), ct.clone()))
                                || trait_satisfied_structurally(bound, &ct, traits, type_methods);
                            if !satisfied {
                                return Err(format_err(
                                    source_path,
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

fn trait_satisfied_structurally(
    trait_name: &str,
    type_name: &str,
    traits: &HashMap<String, (Vec<TraitMethodSig>, Span)>,
    type_methods: &HashMap<String, HashMap<String, usize>>,
) -> bool {
    let methods = match traits.get(trait_name) {
        Some((m, _)) => m,
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
        let lexer = Lexer::new(src.to_string());
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
}
