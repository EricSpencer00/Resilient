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
    let mut children = Vec::new();
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

            // Track child IDs to detect duplicates
            let mut seen_ids = std::collections::HashSet::new();

            // Validate each child
            for child in children {
                // Check for duplicate IDs
                if !seen_ids.insert(child.id.clone()) {
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

                // Validate that the referenced function exists
                if let Some(ty) = env.get(&child.fn_name) {
                    // Check if it's a function type
                    match ty {
                        crate::typechecker::Type::Function { .. } => {
                            // Valid — it's a function
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
