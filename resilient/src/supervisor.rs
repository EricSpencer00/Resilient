use crate::typechecker::TypeEnvironment;
/// RES-333: supervisor tree parsing and support.
///
/// Supervisor declarations provide restart policies for actor failures.
/// Syntax:
/// ```
/// supervisor {
///     strategy: one_for_one,
///     children: [
///         { id: "name", fn: handler_fn, restart: permanent },
///         { id: "name2", fn: handler_fn2, restart: transient },
///     ]
/// }
/// ```
///
/// This module handles parsing the supervisor block and its children.
use crate::{Node, SupervisorChild, Token};

/// Parse a supervisor declaration: `supervisor { strategy, children }`.
/// Called when the `supervisor` keyword has been matched; consumes it first.
pub(crate) fn parse(parser: &mut crate::Parser) -> Node {
    let span = parser.span_at_current();
    parser.next_token(); // skip `supervisor`

    // Consume opening brace
    if parser.current_token != Token::LeftBrace {
        parser.record_error(format!(
            "expected `{{` after `supervisor`, found {}",
            parser.current_token
        ));
        return Node::SupervisorDecl {
            strategy: String::new(),
            children: Vec::new(),
            span,
        };
    }
    parser.next_token();

    // Parse strategy field
    if !matches!(parser.current_token, Token::Identifier(ref n) if n == "strategy") {
        parser.record_error("expected `strategy` field in supervisor block".to_string());
        return Node::SupervisorDecl {
            strategy: String::new(),
            children: Vec::new(),
            span,
        };
    }
    parser.next_token();

    if parser.current_token != Token::Colon {
        parser.record_error("expected `:` after `strategy`".to_string());
        return Node::SupervisorDecl {
            strategy: String::new(),
            children: Vec::new(),
            span,
        };
    }
    parser.next_token();

    let strategy = match &parser.current_token {
        Token::Identifier(s) => s.clone(),
        _ => {
            parser.record_error(format!(
                "expected strategy identifier, found {}",
                parser.current_token
            ));
            String::new()
        }
    };

    // Validate strategy
    if strategy != "one_for_one" && strategy != "one_for_all" && !strategy.is_empty() {
        parser.record_error(format!(
            "invalid strategy `{}`; must be `one_for_one` or `one_for_all`",
            strategy
        ));
    }

    parser.next_token();

    if parser.current_token != Token::Comma {
        parser.record_error("expected `,` after strategy value".to_string());
        return Node::SupervisorDecl {
            strategy,
            children: Vec::new(),
            span,
        };
    }
    parser.next_token();

    // Parse children field
    if !matches!(parser.current_token, Token::Identifier(ref n) if n == "children") {
        parser.record_error("expected `children` field in supervisor block".to_string());
        return Node::SupervisorDecl {
            strategy,
            children: Vec::new(),
            span,
        };
    }
    parser.next_token();

    if parser.current_token != Token::Colon {
        parser.record_error("expected `:` after `children`".to_string());
        return Node::SupervisorDecl {
            strategy,
            children: Vec::new(),
            span,
        };
    }
    parser.next_token();

    if parser.current_token != Token::LeftBracket {
        parser.record_error(format!(
            "expected `[` for children array, found {}",
            parser.current_token
        ));
        return Node::SupervisorDecl {
            strategy,
            children: Vec::new(),
            span,
        };
    }
    parser.next_token();

    // Parse children array
    // RES-1770: pre-size to 4 — supervisor trees typically have 2-8
    // children. Same fixed-capacity shape as RES-1768's parser
    // pre-sizes (no upstream count available).
    let mut children = Vec::with_capacity(4);
    while parser.current_token != Token::RightBracket && parser.current_token != Token::Eof {
        // Parse child object: { id: "name", fn: fn_name, restart: restart_type }
        if parser.current_token != Token::LeftBrace {
            parser.record_error(format!(
                "expected `{{` for child spec, found {}",
                parser.current_token
            ));
            break;
        }
        parser.next_token();

        if let Some(child) = parse_child(parser) {
            children.push(child);
        }

        if parser.current_token != Token::RightBrace {
            parser.record_error("expected `}}` to close child spec".to_string());
            break;
        }
        parser.next_token();

        // Check for comma or end
        if parser.current_token == Token::Comma {
            parser.next_token();
        } else if parser.current_token != Token::RightBracket {
            parser.record_error("expected `,` or `]` after child spec".to_string());
            break;
        }
    }

    if parser.current_token != Token::RightBracket {
        parser.record_error("expected `]` to close children array".to_string());
    }
    parser.next_token();

    if parser.current_token != Token::RightBrace {
        parser.record_error("expected `}}` to close supervisor block".to_string());
    }
    parser.next_token();

    Node::SupervisorDecl {
        strategy,
        children,
        span,
    }
}

/// Parse a single child specification: { id: "name", fn: fn_name, restart: type }
fn parse_child(parser: &mut crate::Parser) -> Option<SupervisorChild> {
    // Parse id field
    if !matches!(parser.current_token, Token::Identifier(ref n) if n == "id") {
        parser.record_error("expected `id` field in child spec".to_string());
        return None;
    }
    parser.next_token();

    if parser.current_token != Token::Colon {
        parser.record_error("expected `:` after `id`".to_string());
        return None;
    }
    parser.next_token();

    let id = match &parser.current_token {
        Token::StringLiteral(s) => s.clone(),
        _ => {
            parser.record_error(format!(
                "expected string literal for id, found {}",
                parser.current_token
            ));
            return None;
        }
    };

    parser.next_token();

    if parser.current_token != Token::Comma {
        parser.record_error("expected `,` after id".to_string());
        return None;
    }
    parser.next_token();

    // Parse fn field
    if !matches!(parser.current_token, Token::Identifier(ref n) if n == "fn") {
        parser.record_error("expected `fn` field in child spec".to_string());
        return None;
    }
    parser.next_token();

    if parser.current_token != Token::Colon {
        parser.record_error("expected `:` after `fn`".to_string());
        return None;
    }
    parser.next_token();

    let fn_name = match &parser.current_token {
        Token::Identifier(s) => s.clone(),
        _ => {
            parser.record_error(format!(
                "expected identifier for fn, found {}",
                parser.current_token
            ));
            return None;
        }
    };

    parser.next_token();

    if parser.current_token != Token::Comma {
        parser.record_error("expected `,` after fn".to_string());
        return None;
    }
    parser.next_token();

    // Parse restart field
    if !matches!(parser.current_token, Token::Identifier(ref n) if n == "restart") {
        parser.record_error("expected `restart` field in child spec".to_string());
        return None;
    }
    parser.next_token();

    if parser.current_token != Token::Colon {
        parser.record_error("expected `:` after `restart`".to_string());
        return None;
    }
    parser.next_token();

    let restart = match &parser.current_token {
        Token::Identifier(s) => s.clone(),
        _ => {
            parser.record_error(format!(
                "expected restart type identifier, found {}",
                parser.current_token
            ));
            return None;
        }
    };

    // Validate restart type
    if restart != "permanent" && restart != "transient" && restart != "temporary" {
        parser.record_error(format!(
            "invalid restart type `{}`; must be `permanent`, `transient`, or `temporary`",
            restart
        ));
    }

    parser.next_token();

    Some(SupervisorChild {
        id,
        fn_name,
        restart,
    })
}

/// RES-333: Phase 3 typechecker validation for supervisor declarations.
/// Validates strategy, child specifications, and referenced functions.
pub(crate) fn check(node: &Node, env: &TypeEnvironment) -> Result<(), String> {
    match node {
        Node::SupervisorDecl {
            strategy, children, ..
        } => {
            // Validate strategy
            if strategy != "one_for_one" && strategy != "one_for_all" {
                return Err(format!(
                    "invalid supervisor strategy `{}`; must be `one_for_one` or `one_for_all`",
                    strategy
                ));
            }

            // Validate that we have children
            if children.is_empty() {
                return Err("supervisor must have at least one child".to_string());
            }

            // Track child IDs to detect duplicates.
            //
            // RES-1467: store `&str` borrows from the AST instead of
            // cloning each child id into an owned `String`. Lifetime
            // is tied to `children` (owned by the AST node passed
            // in). Same shape as RES-1431 (pattern_bindings) /
            // RES-1439 (info_flow walk_calls).
            // RES-1740: pre-size to children.len() — exact upper bound.
            let mut seen_ids: std::collections::HashSet<&str> =
                std::collections::HashSet::with_capacity(children.len());

            // Validate each child
            for child in children {
                // Check for duplicate IDs
                if !seen_ids.insert(child.id.as_str()) {
                    return Err(format!("duplicate child id `{}` in supervisor", child.id));
                }

                // Validate restart type
                if child.restart != "permanent"
                    && child.restart != "transient"
                    && child.restart != "temporary"
                {
                    return Err(format!(
                        "invalid restart type `{}` for child `{}`; must be `permanent`, `transient`, or `temporary`",
                        child.restart, child.id
                    ));
                }

                // Validate that the referenced function exists and has a
                // compatible signature for use as an actor spawn function.
                //
                // RES-776 Phase 2: actor spawn functions are called via
                // `spawn(fn_name)` with no arguments — they must be
                // zero-parameter functions. A non-zero-parameter function
                // can't be spawned directly and would panic at runtime.
                // The return type is also constrained to Void: a supervisor
                // child runs in a loop and never returns a meaningful value
                // to the supervisor (exit is signalled via the crash channel,
                // not a return value).
                if let Some(ty) = env.get(&child.fn_name) {
                    match ty {
                        crate::typechecker::Type::Function {
                            params,
                            return_type,
                        } => {
                            // Phase 2 check: spawn function must take no params.
                            if !params.is_empty() {
                                return Err(format!(
                                    "supervisor child `{}` references `{}`, which takes {} parameter(s); \
                                     actor spawn functions must take no parameters",
                                    child.id,
                                    child.fn_name,
                                    params.len()
                                ));
                            }
                            // Phase 2 check: spawn function must return Void.
                            if !matches!(return_type.as_ref(), crate::typechecker::Type::Void) {
                                return Err(format!(
                                    "supervisor child `{}` references `{}`, which returns `{}`; \
                                     actor spawn functions must return void",
                                    child.id, child.fn_name, return_type
                                ));
                            }
                        }
                        _ => {
                            return Err(format!(
                                "child `{}` references `{}`, which is not a function",
                                child.id, child.fn_name
                            ));
                        }
                    }
                } else {
                    return Err(format!(
                        "child `{}` references undefined function `{}`",
                        child.id, child.fn_name
                    ));
                }
            }

            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typechecker::TypeEnvironment;

    fn make_fn_env(names: &[&str]) -> TypeEnvironment {
        let mut env = TypeEnvironment::new();
        for name in names {
            // Register each as a function type
            env.set(
                name.to_string(),
                crate::typechecker::Type::Function {
                    params: vec![],
                    return_type: Box::new(crate::typechecker::Type::Void),
                },
            );
        }
        env
    }

    #[test]
    fn supervisor_valid_one_for_one() {
        let node = Node::SupervisorDecl {
            strategy: "one_for_one".to_string(),
            children: vec![SupervisorChild {
                id: "w1".to_string(),
                fn_name: "worker".to_string(),
                restart: "permanent".to_string(),
            }],
            span: crate::span::Span::default(),
        };
        let env = make_fn_env(&["worker"]);
        assert!(check(&node, &env).is_ok());
    }

    #[test]
    fn supervisor_valid_one_for_all() {
        let node = Node::SupervisorDecl {
            strategy: "one_for_all".to_string(),
            children: vec![SupervisorChild {
                id: "w1".to_string(),
                fn_name: "worker".to_string(),
                restart: "permanent".to_string(),
            }],
            span: crate::span::Span::default(),
        };
        let env = make_fn_env(&["worker"]);
        assert!(check(&node, &env).is_ok());
    }

    #[test]
    fn supervisor_invalid_strategy() {
        let node = Node::SupervisorDecl {
            strategy: "invalid_strategy".to_string(),
            children: vec![SupervisorChild {
                id: "w1".to_string(),
                fn_name: "worker".to_string(),
                restart: "permanent".to_string(),
            }],
            span: crate::span::Span::default(),
        };
        let env = make_fn_env(&["worker"]);
        let err = check(&node, &env);
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("invalid supervisor strategy"));
    }

    #[test]
    fn supervisor_empty_children() {
        let node = Node::SupervisorDecl {
            strategy: "one_for_one".to_string(),
            children: vec![],
            span: crate::span::Span::default(),
        };
        let env = TypeEnvironment::new();
        let err = check(&node, &env);
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("at least one child"));
    }

    #[test]
    fn supervisor_duplicate_ids() {
        let node = Node::SupervisorDecl {
            strategy: "one_for_one".to_string(),
            children: vec![
                SupervisorChild {
                    id: "worker".to_string(),
                    fn_name: "w1".to_string(),
                    restart: "permanent".to_string(),
                },
                SupervisorChild {
                    id: "worker".to_string(),
                    fn_name: "w2".to_string(),
                    restart: "permanent".to_string(),
                },
            ],
            span: crate::span::Span::default(),
        };
        let env = make_fn_env(&["w1", "w2"]);
        let err = check(&node, &env);
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("duplicate child id"));
    }

    #[test]
    fn supervisor_invalid_restart_type() {
        let node = Node::SupervisorDecl {
            strategy: "one_for_one".to_string(),
            children: vec![SupervisorChild {
                id: "w1".to_string(),
                fn_name: "worker".to_string(),
                restart: "invalid".to_string(),
            }],
            span: crate::span::Span::default(),
        };
        let env = make_fn_env(&["worker"]);
        let err = check(&node, &env);
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("invalid restart type"));
    }

    #[test]
    fn supervisor_undefined_function() {
        let node = Node::SupervisorDecl {
            strategy: "one_for_one".to_string(),
            children: vec![SupervisorChild {
                id: "w1".to_string(),
                fn_name: "undefined".to_string(),
                restart: "permanent".to_string(),
            }],
            span: crate::span::Span::default(),
        };
        let env = TypeEnvironment::new();
        let err = check(&node, &env);
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("undefined function"));
    }

    #[test]
    fn supervisor_all_valid_restart_types() {
        for restart_type in &["permanent", "transient", "temporary"] {
            let node = Node::SupervisorDecl {
                strategy: "one_for_one".to_string(),
                children: vec![SupervisorChild {
                    id: "w1".to_string(),
                    fn_name: "worker".to_string(),
                    restart: restart_type.to_string(),
                }],
                span: crate::span::Span::default(),
            };
            let env = make_fn_env(&["worker"]);
            assert!(
                check(&node, &env).is_ok(),
                "restart type {} should be valid",
                restart_type
            );
        }
    }
}
