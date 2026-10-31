//! Token-level validation prevents safety bounds from disappearing during metadata parsing.
use crate::{Node, Parser, Token};

pub(crate) fn parse(parser: &mut Parser, line: usize, column: usize) -> Option<Node> {
    parser.next_token();
    let mut bound = None;
    if matches!(parser.current_token, Token::LeftParen) {
        parser.next_token();
        if let Token::IntLiteral(value) = parser.current_token
            && let Ok(value) = u32::try_from(value)
            && value > 0
        {
            bound = Some(value);
            parser.next_token();
        }
        if matches!(parser.current_token, Token::RightParen) {
            parser.next_token();
        } else {
            bound = None;
        }
    }
    if !matches!(parser.current_token, Token::RightBracket) {
        bound = None;
    }
    if bound.is_none() {
        parser.record_error_at(
            line,
            column,
            "#[loop_bound] requires exactly one integer literal in 1..=4294967295".into(),
        );
        while !matches!(parser.current_token, Token::RightBracket | Token::Eof) {
            parser.next_token();
        }
    }
    if matches!(parser.current_token, Token::RightBracket) {
        parser.next_token();
    }
    if bound.is_some() && matches!(parser.current_token, Token::Eof) {
        parser.record_error_at(
            line,
            column,
            "#[loop_bound] requires a named function declaration".into(),
        );
        return None;
    }
    let item = parser.parse_statement();
    if let (Some(bound), Some(node)) = (bound, item.as_ref()) {
        if let Node::Function { name, .. } = node {
            if crate::loop_bound_metadata::record(node, bound) {
                crate::feature_attrs::record(
                    name,
                    crate::feature_attrs::AttrRecord {
                        name: "loop_bound".into(),
                        args: bound.to_string(),
                        line,
                    },
                );
            } else {
                parser.record_error_at(
                    line,
                    column,
                    "conflicting #[loop_bound] attributes on the same function".into(),
                );
            }
        } else {
            parser.record_error_at(
                line,
                column,
                "#[loop_bound] requires a named function declaration".into(),
            );
        }
    }
    item
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostics_point_to_attribute_and_recovery_keeps_following_items() {
        let (node, errors) = crate::parse("\n  #[loop_bound(1.0)]\nfn bad(){}\nfn good(){}");
        assert!(errors.iter().any(|e| e.starts_with("2:3:")), "{errors:?}");
        assert!(format!("{node:?}").contains("good"));
        let (_, errors) = crate::parse("\n  #[loop_bound(1)]\n#[loop_bound(2)]\nfn f(){}");
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("2:3:") && e.contains("conflicting")),
            "{errors:?}"
        );
    }
}
