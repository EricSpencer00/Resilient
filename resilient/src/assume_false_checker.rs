// RES-133c: Detect assume(false) and warn about dead-code regions.
//
// When assume(false) is used, the following statements are unreachable
// in any valid model. This pass walks the AST to detect assume(false)
// predicates and warns at the next statement boundary.

use crate::Node;

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let mut checker = AssumeChecker::new(source_path);
    checker.walk(program);
    Ok(())
}

struct AssumeChecker<'a> {
    source_path: &'a str,
}

impl<'a> AssumeChecker<'a> {
    fn new(source_path: &'a str) -> Self {
        AssumeChecker { source_path }
    }

    fn walk(&mut self, node: &Node) {
        match node {
            Node::Program(statements) => {
                self.check_statement_sequence(statements);
            }
            Node::Block { stmts, .. } => {
                self.check_block(stmts);
            }
            _ => self.walk_children(node),
        }
    }

    fn check_statement_sequence(&mut self, statements: &[crate::span::Spanned<Node>]) {
        for (i, stmt) in statements.iter().enumerate() {
            self.walk(&stmt.node);

            // Check if this statement is assume(false)
            if self.is_assume_false(&stmt.node) {
                // Warn about statements after assume(false)
                if i + 1 < statements.len() {
                    let next_stmt = &statements[i + 1];
                    eprintln!(
                        "{}:{}  warning: dead-code region following assume(false)",
                        self.source_path, next_stmt.span.start
                    );
                }
            }
        }
    }

    fn check_block(&mut self, stmts: &[Node]) {
        for (i, stmt) in stmts.iter().enumerate() {
            self.walk(stmt);

            // Check if this statement is assume(false)
            if self.is_assume_false(stmt) {
                // Warn about statements after assume(false)
                if i + 1 < stmts.len() {
                    eprintln!(
                        "{}:  warning: dead-code region following assume(false)",
                        self.source_path
                    );
                }
            }
        }
    }

    fn is_assume_false(&self, node: &Node) -> bool {
        match node {
            Node::Assume { condition, .. } => {
                // Check if condition is literally false
                matches!(**condition, Node::BooleanLiteral { value: false, .. })
            }
            _ => false,
        }
    }

    fn walk_children(&mut self, node: &Node) {
        match node {
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                self.walk(condition);
                self.walk(consequence);
                if let Some(alt) = alternative {
                    self.walk(alt);
                }
            }
            Node::WhileStatement {
                condition, body, ..
            } => {
                self.walk(condition);
                self.walk(body);
            }
            Node::ForInStatement { iterable, body, .. } => {
                self.walk(iterable);
                self.walk(body);
            }
            Node::Function { body, .. } => {
                self.walk(body);
            }
            _ => {}
        }
    }
}
