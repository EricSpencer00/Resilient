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
