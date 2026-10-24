//! The implementation proof profile is deliberately narrower than the language.
//! It describes bytecode VM arithmetic, independently of a selected execution backend.
use crate::Node;
pub(crate) const PROFILE: &str = "vm-i64-wrap-v1";
pub(crate) fn wrap_mode() -> bool {
    crate::vm::OverflowMode::from_env() == crate::vm::OverflowMode::Wrap
}
pub(crate) fn signature(parameters: &[(String, String)], result: Option<&str>) -> bool {
    fn integer(ty: &str) -> bool {
        matches!(ty, "int" | "Int" | "Int64" | "i64" | "long")
    }
    wrap_mode() && result.is_some_and(integer) && parameters.iter().all(|(ty, _)| integer(ty))
}
pub(crate) fn supported(node: &Node) -> bool {
    fn visit(node: &Node, remaining: &mut usize, depth: usize) -> bool {
        if *remaining == 0 || depth > 128 {
            return false;
        }
        *remaining -= 1;
        match node {
            Node::IntegerLiteral { .. } | Node::BooleanLiteral { .. } | Node::Identifier { .. } => {
                true
            }
            Node::PrefixExpression {
                operator, right, ..
            } if matches!(*operator, "-" | "!") => visit(right, remaining, depth + 1),
            Node::InfixExpression {
                left,
                operator,
                right,
                ..
            } if matches!(
                *operator,
                "+" | "-"
                    | "*"
                    | "/"
                    | "%"
                    | "=="
                    | "!="
                    | "<"
                    | ">"
                    | "<="
                    | ">="
                    | "&&"
                    | "||"
                    | "&"
                    | "|"
                    | "^"
            ) =>
            {
                visit(left, remaining, depth + 1) && visit(right, remaining, depth + 1)
            }
            _ => false,
        }
    }
    visit(node, &mut 32768, 0)
}

/// Clause-only arithmetic proofs are not authority to erase an executable guard.
pub(crate) fn runtime_elision_safe(parameters: &[(String, String)], node: &Node) -> bool {
    fn visit(parameters: &[(String, String)], node: &Node, depth: usize) -> bool {
        if depth > 128 {
            return false;
        }
        match node {
            Node::IntegerLiteral { .. } | Node::BooleanLiteral { .. } => true,
            Node::Identifier { name, .. } => parameters.iter().any(|(ty, parameter)| {
                parameter == name && matches!(ty.as_str(), "int" | "Int" | "Int64" | "i64" | "long")
            }),
            Node::PrefixExpression {
                operator: "!",
                right,
                ..
            } => visit(parameters, right, depth + 1),
            Node::InfixExpression {
                left,
                operator,
                right,
                ..
            } if matches!(
                *operator,
                "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||"
            ) =>
            {
                visit(parameters, left, depth + 1) && visit(parameters, right, depth + 1)
            }
            _ => false,
        }
    }
    visit(parameters, node, 0)
}
