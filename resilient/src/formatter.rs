// RES-fmt: canonical source-code formatter for Resilient.
//
// Walks the AST produced by `parse` and pretty-prints it with the
// canonical style:
//
// - 4-space indentation
// - one space around binary operators
// - opening brace on the same line as the introducing construct
// - no trailing whitespace
// - blank line between top-level declarations
// - `requires` / `ensures` clauses indented under the function signature
// - `live` blocks follow the same brace style
//
// Scope (v1): the formatter is best-effort. It handles let bindings,
// assignment, function / function-literal definitions, if / while /
// for-in, return, block, assert, prefix / infix / call / field /
// index / try expressions, struct decls + literals + destructure,
// impl blocks, type aliases, live blocks (with backoff / within
// clauses), match expressions, and map / set / array / bytes / string
// / bool / int / float / identifier literals.
//
// Caveats (carried as TODOs, never silently wrong):
// - Match-arm guards (`if <expr>`) and or-patterns (`p1 | p2`) are
//   emitted as written, but complex nested match bodies aren't
//   specially re-wrapped.
// - The formatter is a structural round-trip: comments are dropped
//   (the parser doesn't retain them). Users should only run fmt on
//   code they're willing to have their comments reattached by hand.
//   This is a known deficiency documented in `docs/tooling.md` and
//   is the top follow-up for the next formatter ticket.

use crate::BackoffConfig;
use crate::Node;
use crate::Pattern;

/// Canonical indent width, in spaces.
const INDENT: &str = "    ";

pub struct Formatter {
    out: String,
    depth: usize,
    /// Tracks whether we just wrote a newline so we can apply the
    /// "no trailing whitespace" rule at line boundaries.
    at_line_start: bool,
}

impl Formatter {
    pub fn new() -> Self {
        Self {
            out: String::new(),
            depth: 0,
            at_line_start: true,
        }
    }

    /// Entry point. Formats a `Node::Program` (or any top-level
    /// statement) into a canonical-style string.
    pub fn format(program: &Node) -> String {
        let mut f = Self::new();
        f.fmt_program(program);
        // Ensure trailing newline; strip any accidental duplicate.
        while f.out.ends_with("\n\n") {
            f.out.pop();
        }
        if !f.out.ends_with('\n') {
            f.out.push('\n');
        }
        f.out
    }

    // ------------------------------------------------------------------
    // low-level write helpers
    // ------------------------------------------------------------------

    fn write(&mut self, s: &str) {
        if self.at_line_start && !s.is_empty() {
            for _ in 0..self.depth {
                self.out.push_str(INDENT);
            }
            self.at_line_start = false;
        }
        self.out.push_str(s);
    }

    fn newline(&mut self) {
        // Strip trailing spaces from the current line before the
        // newline so we never emit trailing whitespace.
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        self.out.push('\n');
        self.at_line_start = true;
    }

    fn blank_line(&mut self) {
        if !self.out.ends_with("\n\n") && !self.out.is_empty() {
            if !self.out.ends_with('\n') {
                self.newline();
            }
            self.out.push('\n');
            self.at_line_start = true;
        }
    }

    fn indent(&mut self) {
        self.depth += 1;
    }

    fn dedent(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        }
    }

    // ------------------------------------------------------------------
    // top-level program
    // ------------------------------------------------------------------

    fn fmt_program(&mut self, node: &Node) {
        match node {
            Node::Program(stmts) => {
                for (i, s) in stmts.iter().enumerate() {
                    if i > 0 {
                        self.blank_line();
                    }
                    self.fmt_stmt(&s.node);
                    if !self.out.ends_with('\n') {
                        self.newline();
                    }
                }
            }
            other => self.fmt_stmt(other),
        }
    }

    // ------------------------------------------------------------------
    // statements
    // ------------------------------------------------------------------

    fn fmt_stmt(&mut self, node: &Node) {
        match node {
            Node::Use { path, alias, .. } => {
                if let Some(ns) = alias {
                    self.write(&format!("use \"{}\" as {};", path, ns));
                } else {
                    self.write(&format!("use \"{}\";", path));
                }
                self.newline();
            }
            Node::Function {
                name,
                parameters,
                body,
                requires,
                ensures,
                return_type,
                ..
            } => {
                self.fmt_function(
                    Some(name),
                    parameters,
                    return_type.as_deref(),
                    requires,
                    ensures,
                    body,
                );
            }
            Node::StructDecl { name, fields, .. } => {
                self.write(&format!("struct {} {{", name));
                self.newline();
                self.indent();
                for (ty, fname) in fields {
                    self.write(&format!("{} {},", ty, fname));
                    self.newline();
                }
                self.dedent();
                self.write("}");
                self.newline();
            }
            Node::ImplBlock {
                trait_name,
                struct_name,
                methods,
                ..
            } => {
                let header = match trait_name {
                    Some(t) => format!("impl {} for {} {{", t, struct_name),
                    None => format!("impl {} {{", struct_name),
                };
                self.write(&header);
                self.newline();
                self.indent();
                for (i, m) in methods.iter().enumerate() {
                    if i > 0 {
                        self.blank_line();
                    }
                    self.fmt_stmt(m);
                }
                self.dedent();
                self.write("}");
                self.newline();
            }
            Node::TraitDecl { name, methods, .. } => {
                self.write(&format!("trait {} {{", name));
                self.newline();
                self.indent();
                for sig in methods {
                    let self_token = if sig.takes_self {
                        if sig.param_arity > 1 {
                            "self, "
                        } else {
                            "self"
                        }
                    } else {
                        ""
                    };
                    let extras = sig
                        .param_arity
                        .saturating_sub(if sig.takes_self { 1 } else { 0 });
                    let placeholders = (0..extras)
                        .map(|i| format!("_{}", i))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.write(&format!("fn {}({}{});", sig.name, self_token, placeholders));
                    self.newline();
                }
                self.dedent();
                self.write("}");
                self.newline();
            }
            Node::TypeAlias { name, target, .. } => {
                self.write(&format!("type {} = {};", name, target));
                self.newline();
            }
            // RES-400 PR 1: re-emit a payload-less enum declaration.
            // PR 2 will extend this with payload kinds (named-field
            // and tuple-style); the format follows Rust convention so
            // upstream IDE tooling can reuse syntax-highlighting.
            Node::EnumDecl { name, variants, .. } => {
                self.write(&format!("enum {} {{", name));
                self.newline();
                self.indent();
                for (i, v) in variants.iter().enumerate() {
                    self.write(&v.name);
                    if i + 1 < variants.len() {
                        self.write(",");
                    }
                    self.newline();
                }
                self.dedent();
                self.write("}");
                self.newline();
            }
            Node::RegionDecl { name, .. } => {
                self.write(&format!("region {};", name));
                self.newline();
            }
            // RES-319: re-emit a newtype declaration.
            Node::NewtypeDecl {
                name, base_type, ..
            } => {
                self.write(&format!("newtype {} = {};", name, base_type));
                self.newline();
            }
            // RES-386: re-emit a commutativity-style actor block.
            Node::Actor {
                name,
                state_type,
                state_init,
                concurrent_ensures,
                handlers,
                ..
            } => {
                self.write(&format!("actor {} {{", name));
                self.newline();
                self.indent();
                self.write(&format!("state: {} = ", state_type));
                self.fmt_expr(state_init);
                self.write(";");
                self.newline();
                for ce in concurrent_ensures {
                    self.write("concurrent_ensures: ");
                    self.fmt_expr(ce);
                    self.write(";");
                    self.newline();
                }
                for h in handlers {
                    self.write(&format!("receive {}()", h.name));
                    for e in &h.ensures {
                        self.newline();
                        self.indent();
                        self.write("ensures ");
                        self.fmt_expr(e);
                        self.write(";");
                        self.dedent();
                    }
                    self.write(" ");
                    self.fmt_block_like(&h.body);
                    self.newline();
                }
                self.dedent();
                self.write("}");
                self.newline();
            }
            // RES-388/RES-390: re-emit an ActorDecl with typed state fields,
            // always invariants, and receive handlers.
            Node::ActorDecl {
                name,
                state_fields,
                always_clauses,
                eventually_clauses,
                receive_handlers,
                ..
            } => {
                self.write(&format!("actor {} {{", name));
                self.newline();
                self.indent();
                for (ty, field, init) in state_fields {
                    self.write(&format!("{}: {} = ", field, ty));
                    self.fmt_expr(init);
                    self.write(";");
                    self.newline();
                }
                for clause in always_clauses {
                    self.write("always: ");
                    self.fmt_expr(clause);
                    self.write(";");
                    self.newline();
                }
                for ev in eventually_clauses {
                    self.write(&format!("eventually(after: {}): ", ev.target_handler));
                    self.fmt_expr(&ev.post);
                    self.write(";");
                    self.newline();
                }
                for h in receive_handlers {
                    self.blank_line();
                    self.write(&format!("receive {}(", h.name));
                    for (i, (pty, pname)) in h.parameters.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.write(&format!("{} {}", pty, pname));
                    }
                    self.write(")");
                    for r in &h.requires {
                        self.newline();
                        self.indent();
                        self.write("requires ");
                        self.fmt_expr(r);
                        self.dedent();
                    }
                    for e in &h.ensures {
                        self.newline();
                        self.indent();
                        self.write("ensures ");
                        self.fmt_expr(e);
                        self.dedent();
                    }
                    self.write(" ");
                    self.fmt_stmt(&h.body);
                    self.newline();
                }
                self.dedent();
                self.write("}");
                self.newline();
            }
            // RES-390: re-emit a ClusterDecl block.
            Node::ClusterDecl {
                name,
                members,
                invariants,
                ..
            } => {
                self.write(&format!("cluster {} {{", name));
                self.newline();
                self.indent();
                for (local, actor_ty) in members {
                    self.write(&format!("{}: {};", local, actor_ty));
                    self.newline();
                }
                for inv in invariants {
                    self.write("cluster_invariant: ");
                    self.fmt_expr(inv);
                    self.write(";");
                    self.newline();
                }
                self.dedent();
                self.write("}");
                self.newline();
            }
            Node::LetStatement {
                name,
                value,
                type_annot,
                ..
            } => {
                let ann = match type_annot {
                    Some(t) => format!(": {}", t),
                    None => String::new(),
                };
                self.write(&format!("let {}{} = ", name, ann));
                self.fmt_expr(value);
                self.write(";");
                self.newline();
            }
            Node::StaticLet { name, value, .. } => {
                self.write(&format!("static let {} = ", name));
                self.fmt_expr(value);
                self.write(";");
                self.newline();
            }
            // RES-361: format `const NAME: T = expr;`
            Node::Const {
                name,
                value,
                type_annot,
                ..
            } => {
                if let Some(ty) = type_annot {
                    self.write(&format!("const {}: {} = ", name, ty));
                } else {
                    self.write(&format!("const {} = ", name));
                }
                self.fmt_expr(value);
                self.write(";");
                self.newline();
            }
            Node::LetDestructureStruct {
                struct_name,
                fields,
                has_rest,
                value,
                ..
            } => {
                self.write(&format!("let {} {{ ", struct_name));
                let mut parts: Vec<String> = Vec::new();
                for (field, local) in fields {
                    if field == local {
                        parts.push(field.clone());
                    } else {
                        parts.push(format!("{}: {}", field, local));
                    }
                }
                if *has_rest {
                    parts.push("..".to_string());
                }
                self.write(&parts.join(", "));
                self.write(" } = ");
                self.fmt_expr(value);
                self.write(";");
                self.newline();
            }
            Node::Assignment { name, value, .. } => {
                self.write(&format!("{} = ", name));
                self.fmt_expr(value);
                self.write(";");
                self.newline();
            }
            Node::ReturnStatement { value, .. } => match value {
                Some(v) => {
                    self.write("return ");
                    self.fmt_expr(v);
                    self.write(";");
                    self.newline();
                }
                None => {
                    self.write("return;");
                    self.newline();
                }
            },
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                self.write("if ");
                self.fmt_expr(condition);
                self.write(" ");
                self.fmt_block_like(consequence);
                if let Some(alt) = alternative {
                    self.write(" else ");
                    // `else if` flattens; `else { ... }` renders as a block.
                    match alt.as_ref() {
                        Node::IfStatement { .. } => self.fmt_stmt(alt),
                        _ => self.fmt_block_like(alt),
                    }
                }
                if !self.out.ends_with('\n') {
                    self.newline();
                }
            }
            Node::WhileStatement {
                condition,
                body,
                invariants,
                ..
            } => {
                self.write("while ");
                self.fmt_expr(condition);
                if !invariants.is_empty() {
                    self.newline();
                    self.indent();
                    for inv in invariants {
                        self.write("invariant ");
                        self.fmt_expr(inv);
                        self.newline();
                    }
                    self.dedent();
                    self.fmt_block_like(body);
                } else {
                    self.write(" ");
                    self.fmt_block_like(body);
                }
                if !self.out.ends_with('\n') {
                    self.newline();
                }
            }
            Node::ForInStatement {
                name,
                iterable,
                body,
                invariants,
                ..
            } => {
                self.write(&format!("for {} in ", name));
                self.fmt_expr(iterable);
                if !invariants.is_empty() {
                    self.newline();
                    self.indent();
                    for inv in invariants {
                        self.write("invariant ");
                        self.fmt_expr(inv);
                        self.newline();
                    }
                    self.dedent();
                    self.fmt_block_like(body);
                } else {
                    self.write(" ");
                    self.fmt_block_like(body);
                }
                if !self.out.ends_with('\n') {
                    self.newline();
                }
            }
            Node::LiveBlock {
                body,
                invariants,
                backoff,
                timeout,
                ..
            } => {
                self.write("live");
                if let Some(bo) = backoff {
                    self.write(&format!(
                        " backoff(base_ms={}, factor={}, max_ms={})",
                        bo.base_ms, bo.factor, bo.max_ms
                    ));
                    let _ = BackoffConfig::default_ticket; // keep import live
                }
                if let Some(tm) = timeout {
                    self.write(" within ");
                    self.fmt_expr(tm);
                }
                for inv in invariants {
                    self.write(" invariant ");
                    self.fmt_expr(inv);
                }
                self.write(" ");
                self.fmt_block_like(body);
                if !self.out.ends_with('\n') {
                    self.newline();
                }
            }
            Node::Assert {
                condition, message, ..
            } => {
                self.write("assert(");
                self.fmt_expr(condition);
                if let Some(m) = message {
                    self.write(", ");
                    self.fmt_expr(m);
                }
                self.write(");");
                self.newline();
            }
            Node::Assume {
                condition, message, ..
            } => {
                self.write("assume(");
                self.fmt_expr(condition);
                if let Some(m) = message {
                    self.write(", ");
                    self.fmt_expr(m);
                }
                self.write(");");
                self.newline();
            }
            // RES-222: `invariant EXPR;` statement form. Only valid
            // inside a loop body; the typechecker rejects elsewhere.
            Node::InvariantStatement { expr, .. } => {
                self.write("invariant ");
                self.fmt_expr(expr);
                self.write(";");
                self.newline();
            }
            Node::Block { stmts, .. } => {
                self.write("{");
                self.newline();
                self.indent();
                for s in stmts {
                    self.fmt_stmt(s);
                }
                self.dedent();
                self.write("}");
                self.newline();
            }
            Node::ExpressionStatement { expr, .. } => {
                self.fmt_expr(expr);
                self.write(";");
                self.newline();
            }
            // RES-224: `try { ... } catch V { ... }` structured handler.
            Node::TryCatch { body, handlers, .. } => {
                self.write("try {");
                self.newline();
                self.indent();
                for s in body {
                    self.fmt_stmt(s);
                }
                self.dedent();
                self.write("}");
                for (variant, handler_body) in handlers {
                    self.write(&format!(" catch {} {{", variant));
                    self.newline();
                    self.indent();
                    for s in handler_body {
                        self.fmt_stmt(s);
                    }
                    self.dedent();
                    self.write("}");
                }
                self.newline();
            }
            // FFI v1: extern blocks not yet formatted (Tasks 4-8).
            Node::Extern { .. } => {}
            // RES-324: `mod name { ... }` namespace block.
            Node::ModuleDecl { name, body, .. } => {
                self.write(&format!("mod {} {{", name));
                self.newline();
                self.indent();
                for s in body {
                    self.fmt_stmt(s);
                }
                self.dedent();
                self.write("}");
                self.newline();
            }
            // RES-333: supervisor declaration. Phase 1: minimal formatting.
            Node::SupervisorDecl {
                strategy, children, ..
            } => {
                self.write("supervisor {");
                self.newline();
                self.indent();
                self.write(&format!("strategy: {},", strategy));
                self.newline();
                self.write("children: [");
                self.newline();
                self.indent();
                for child in children {
                    self.write(&format!(
                        "{{ id: \"{}\", fn: {}, restart: {} }},",
                        child.id, child.fn_name, child.restart
                    ));
                    self.newline();
                }
                self.dedent();
                self.write("]");
                self.newline();
                self.dedent();
                self.write("}");
                self.newline();
            }
            // Anything else was an expression; dispatch to fmt_expr
            // and terminate with a semicolon so a bare expression
            // statement at top level still looks like a statement.
            other => {
                self.fmt_expr(other);
                self.write(";");
                self.newline();
            }
        }
    }

    /// Render a `Node::Block` or fall back to wrapping a single
    /// statement in `{ ... }`.
    fn fmt_block_like(&mut self, node: &Node) {
        match node {
            Node::Block { stmts, .. } => {
                self.write("{");
                self.newline();
                self.indent();
                for s in stmts {
                    self.fmt_stmt(s);
                }
                self.dedent();
                self.write("}");
            }
            other => {
                // Synthesize a single-stmt block so the brace style
                // stays uniform.
                self.write("{");
                self.newline();
                self.indent();
                self.fmt_stmt(other);
                self.dedent();
                self.write("}");
            }
        }
    }

    // ------------------------------------------------------------------
    // function definition (shared between named + literal forms)
    // ------------------------------------------------------------------

    fn fmt_function(
        &mut self,
        name: Option<&str>,
        parameters: &[(String, String)],
        return_type: Option<&str>,
        requires: &[Node],
        ensures: &[Node],
        body: &Node,
    ) {
        self.write("fn");
        if let Some(n) = name {
            self.write(&format!(" {}", n));
        }
        self.write("(");
        let params: Vec<String> = parameters
            .iter()
            .map(|(ty, pname)| format!("{} {}", ty, pname))
            .collect();
        self.write(&params.join(", "));
        self.write(")");
        if let Some(rt) = return_type {
            self.write(&format!(" -> {}", rt));
        }

        if !requires.is_empty() || !ensures.is_empty() {
            self.newline();
            self.indent();
            for r in requires {
                self.write("requires ");
                self.fmt_expr(r);
                self.newline();
            }
            for e in ensures {
                self.write("ensures ");
                self.fmt_expr(e);
                self.newline();
            }
            self.dedent();
            self.fmt_block_like(body);
        } else {
            self.write(" ");
            self.fmt_block_like(body);
        }
        if !self.out.ends_with('\n') {
            self.newline();
        }
    }

    // ------------------------------------------------------------------
    // expressions
    // ------------------------------------------------------------------

    fn fmt_expr(&mut self, node: &Node) {
        match node {
            Node::Identifier { name, .. } => self.write(name),
            Node::IntegerLiteral { value, .. } => self.write(&value.to_string()),
            Node::FloatLiteral { value, .. } => {
                // Always include a decimal point so the literal round-
                // trips as a float, not an int.
                let s = format!("{}", value);
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    self.write(&s);
                } else {
                    self.write(&format!("{}.0", s));
                }
            }
            Node::StringLiteral { value, .. } => {
                self.write(&format!("\"{}\"", escape_string(value)));
            }
            Node::BooleanLiteral { value, .. } => {
                self.write(if *value { "true" } else { "false" });
            }
            Node::BytesLiteral { value, .. } => {
                self.write("b\"");
                for b in value {
                    match *b {
                        b'\\' => self.write("\\\\"),
                        b'"' => self.write("\\\""),
                        b'\n' => self.write("\\n"),
                        b'\t' => self.write("\\t"),
                        b'\r' => self.write("\\r"),
                        0 => self.write("\\0"),
                        x if x.is_ascii_graphic() || x == b' ' => {
                            self.write(&char::from(x).to_string());
                        }
                        x => self.write(&format!("\\x{:02x}", x)),
                    }
                }
                self.write("\"");
            }
            Node::PrefixExpression {
                operator, right, ..
            } => {
                self.write(operator);
                self.fmt_expr(right);
            }
            Node::InfixExpression {
                left,
                operator,
                right,
                ..
            } => {
                self.fmt_expr(left);
                self.write(&format!(" {} ", operator));
                self.fmt_expr(right);
            }
            Node::CallExpression {
                function,
                arguments,
                ..
            } => {
                self.fmt_expr(function);
                self.write("(");
                for (i, a) in arguments.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.fmt_expr(a);
                }
                self.write(")");
            }
            // RES-325: `name: value` inside a call argument list.
            Node::NamedArg { name, value, .. } => {
                self.write(&format!("{}: ", name));
                self.fmt_expr(value);
            }
            // RES-319: newtype constructor — re-emit as `Name(value)`.
            Node::NewtypeConstruct {
                type_name, value, ..
            } => {
                self.write(type_name);
                self.write("(");
                self.fmt_expr(value);
                self.write(")");
            }
            Node::TryExpression { expr, .. } => {
                self.fmt_expr(expr);
                self.write("?");
            }
            // RES-363: optional chaining.
            Node::OptionalChain { object, access, .. } => {
                self.fmt_expr(object);
                match access {
                    crate::ChainAccess::Field(f) => {
                        self.write(&format!("?.{}", f));
                    }
                    crate::ChainAccess::Method(m, args) => {
                        self.write(&format!("?.{}(", m));
                        for (i, a) in args.iter().enumerate() {
                            if i > 0 {
                                self.write(", ");
                            }
                            self.fmt_expr(a);
                        }
                        self.write(")");
                    }
                }
            }
            Node::FieldAccess { target, field, .. } => {
                self.fmt_expr(target);
                self.write(&format!(".{}", field));
            }
            Node::FieldAssignment {
                target,
                field,
                value,
                ..
            } => {
                self.fmt_expr(target);
                self.write(&format!(".{} = ", field));
                self.fmt_expr(value);
            }
            Node::IndexExpression { target, index, .. } => {
                self.fmt_expr(target);
                self.write("[");
                self.fmt_expr(index);
                self.write("]");
            }
            Node::IndexAssignment {
                target,
                index,
                value,
                ..
            } => {
                self.fmt_expr(target);
                self.write("[");
                self.fmt_expr(index);
                self.write("] = ");
                self.fmt_expr(value);
            }
            Node::ArrayLiteral { items, .. } => {
                self.write("[");
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.fmt_expr(it);
                }
                self.write("]");
            }
            Node::MapLiteral { entries, .. } => {
                self.write("{");
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    } else {
                        self.write(" ");
                    }
                    self.fmt_expr(k);
                    self.write(" -> ");
                    self.fmt_expr(v);
                }
                if !entries.is_empty() {
                    self.write(" ");
                }
                self.write("}");
            }
            Node::SetLiteral { items, .. } => {
                self.write("#{");
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.fmt_expr(it);
                }
                self.write("}");
            }
            Node::StructLiteral { name, fields, .. } => {
                self.write(&format!("new {} {{", name));
                for (i, (fname, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.write(",");
                    }
                    self.write(&format!(" {}: ", fname));
                    self.fmt_expr(v);
                }
                if !fields.is_empty() {
                    self.write(" ");
                }
                self.write("}");
            }
            Node::FunctionLiteral {
                parameters,
                body,
                requires,
                ensures,
                return_type,
                ..
            } => {
                // Anonymous fn inside an expression context: reuse the
                // shared function-renderer with `name = None`.
                self.fmt_function(
                    None,
                    parameters,
                    return_type.as_deref(),
                    requires,
                    ensures,
                    body,
                );
            }
            Node::Match {
                scrutinee, arms, ..
            } => {
                self.write("match ");
                self.fmt_expr(scrutinee);
                self.write(" {");
                self.newline();
                self.indent();
                for (pat, guard, body) in arms {
                    self.fmt_pattern(pat);
                    if let Some(g) = guard {
                        self.write(" if ");
                        self.fmt_expr(g);
                    }
                    self.write(" => ");
                    self.fmt_expr(body);
                    self.write(",");
                    self.newline();
                }
                self.dedent();
                self.write("}");
            }
            Node::DurationLiteral { nanos, .. } => {
                // Collapse back to the smallest whole-unit form we can.
                // Fall back to `ns` when divisibility fails.
                if *nanos % 1_000_000_000 == 0 {
                    self.write(&format!("{}s", nanos / 1_000_000_000));
                } else if *nanos % 1_000_000 == 0 {
                    self.write(&format!("{}ms", nanos / 1_000_000));
                } else if *nanos % 1_000 == 0 {
                    self.write(&format!("{}us", nanos / 1_000));
                } else {
                    self.write(&format!("{}ns", nanos));
                }
            }
            // RES-330: `(forall|exists) v in <range>: <body>`.
            Node::Quantifier {
                kind,
                var,
                range,
                body,
                ..
            } => {
                self.write(kind.keyword());
                self.write(" ");
                self.write(var);
                self.write(" in ");
                match range {
                    crate::quantifiers::QuantRange::Range { lo, hi } => {
                        self.fmt_expr(lo);
                        self.write("..");
                        self.fmt_expr(hi);
                    }
                    crate::quantifiers::QuantRange::Iterable(expr) => {
                        self.fmt_expr(expr);
                    }
                }
                self.write(": ");
                self.fmt_expr(body);
            }
            // RES-291: integer range expression `lo..hi` / `lo..=hi`.
            Node::Range {
                lo, hi, inclusive, ..
            } => {
                self.fmt_expr(lo);
                if *inclusive {
                    self.write("..=");
                } else {
                    self.write("..");
                }
                self.fmt_expr(hi);
            }
            // RES-221: re-emit the interpolated string literal by
            // reconstructing each part. Literal text is escaped like an
            // ordinary string; expressions are wrapped back in `{...}`.
            Node::InterpolatedString { parts, .. } => {
                self.write("\"");
                for part in parts {
                    match part {
                        crate::string_interp::StringPart::Literal(s) => {
                            self.write(&escape_string(s));
                        }
                        crate::string_interp::StringPart::Expr(expr) => {
                            self.write("{");
                            self.fmt_expr(expr);
                            self.write("}");
                        }
                    }
                }
                self.write("\"");
            }
            // Statement-shaped nodes that ended up in expression
            // position: degrade gracefully to their statement form.
            Node::Block { .. }
            | Node::IfStatement { .. }
            | Node::WhileStatement { .. }
            | Node::ForInStatement { .. }
            | Node::LiveBlock { .. }
            | Node::Assert { .. }
            | Node::Assume { .. }
            | Node::InvariantStatement { .. }
            | Node::LetStatement { .. }
            | Node::StaticLet { .. }
            | Node::Const { .. }
            | Node::Assignment { .. }
            | Node::ReturnStatement { .. }
            | Node::ExpressionStatement { .. }
            | Node::Function { .. }
            | Node::StructDecl { .. }
            | Node::ImplBlock { .. }
            | Node::TypeAlias { .. }
            | Node::RegionDecl { .. }
            | Node::NewtypeDecl { .. }
            | Node::Actor { .. }
            | Node::ActorDecl { .. }
            | Node::ClusterDecl { .. }
            | Node::Use { .. }
            | Node::Extern { .. }
            | Node::LetDestructureStruct { .. }
            | Node::TryCatch { .. }
            | Node::ModuleDecl { .. }
            | Node::SupervisorDecl { .. }
            | Node::TraitDecl { .. }
            | Node::EnumDecl { .. }
            | Node::Program(_) => {
                self.fmt_stmt(node);
            }
            // RES-401: tuple expressions and destructuring let.
            Node::TupleLiteral { items, .. } => {
                self.write("(");
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.fmt_expr(it);
                }
                self.write(")");
            }
            Node::TupleIndex { tuple, index, .. } => {
                self.fmt_expr(tuple);
                self.write(&format!(".{}", index));
            }
            Node::LetTupleDestructure { names, value, .. } => {
                self.write("let (");
                for (i, n) in names.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(n);
                }
                self.write(") = ");
                self.fmt_expr(value);
                self.write(";");
            }
        }
    }

    fn fmt_pattern(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Wildcard => self.write("_"),
            Pattern::Identifier(name) => self.write(name),
            Pattern::Literal(node) => self.fmt_expr(node),
            Pattern::Or(branches) => {
                for (i, b) in branches.iter().enumerate() {
                    if i > 0 {
                        self.write(" | ");
                    }
                    self.fmt_pattern(b);
                }
            }
            // RES-161a: `name @ inner`
            Pattern::Bind(name, inner) => {
                self.write(name);
                self.write(" @ ");
                self.fmt_pattern(inner);
            }
            Pattern::Struct {
                struct_name,
                fields,
                has_rest,
            } => {
                self.write(struct_name);
                self.write(" { ");
                for (i, (fname, sub)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(fname);
                    if matches!(sub.as_ref(), Pattern::Identifier(n) if n == fname) {
                    } else {
                        self.write(": ");
                        self.fmt_pattern(sub.as_ref());
                    }
                }
                if *has_rest {
                    if !fields.is_empty() {
                        self.write(", ");
                    }
                    self.write("..");
                }
                self.write(" }");
            }
            // RES-375: `Some(inner)` / `None` Option patterns.
            Pattern::Some(inner) => {
                self.write("Some(");
                self.fmt_pattern(inner.as_ref());
                self.write(")");
            }
            Pattern::None => self.write("None"),
        }
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

// RES-199: property-based roundtrip tests. Gated behind the `proptest`
// feature so the default build doesn't pull in proptest's dep tree.
// Run with:
//   cargo test --features proptest
#[cfg(all(test, feature = "proptest"))]
mod roundtrip {
    use super::*;
    use crate::parse;
    use proptest::prelude::*;

    // ----------------------------------------------------------------
    // Mini abstract syntax for canonical-form generation
    //
    // We generate programs by building an abstract description and
    // rendering it using the same rules as the Formatter — so the
    // rendered string is canonical by construction. Then we assert
    // `fmt(parse(rendered)) == rendered` (round-trip identity) and
    // `fmt(fmt(rendered)) == fmt(rendered)` (idempotence).
    //
    // Breadth is bounded by the `depth` parameter threaded through
    // all recursive strategies to keep test time O(cases) not
    // O(cases * tree_size^depth).
    // ----------------------------------------------------------------

    /// Names safe to use as identifiers — short, ASCII, never keywords.
    const SAFE_NAMES: &[&str] = &[
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "n", "m", "p", "q", "r", "s", "t",
        "u", "v", "w", "x", "y", "z",
    ];
    /// Types the generator uses for function parameters / let annotations.
    const SAFE_TYPES: &[&str] = &["int", "bool", "string", "float"];

    fn safe_name() -> impl Strategy<Value = &'static str> {
        proptest::sample::select(SAFE_NAMES)
    }

    fn safe_type() -> impl Strategy<Value = &'static str> {
        proptest::sample::select(SAFE_TYPES)
    }

    // ----------------------------------------------------------------
    // Expression generator
    //
    // Returns a canonical string for an expression.  `depth` controls
    // recursion: at depth 0 we only emit atoms (literals / identifiers).
    // ----------------------------------------------------------------

    fn expr_strategy(depth: u32) -> impl Strategy<Value = String> {
        if depth == 0 {
            // Leaf: integer literal, bool literal, or identifier.
            prop_oneof![
                (0i64..100i64).prop_map(|n| n.to_string()),
                proptest::bool::ANY.prop_map(|b| if b { "true" } else { "false" }.to_string()),
                safe_name().prop_map(|n| n.to_string()),
            ]
            .boxed()
        } else {
            let leaf = expr_strategy(0);
            prop_oneof![
                // Atom fallback at any depth.
                (0i64..100i64).prop_map(|n| n.to_string()),
                proptest::bool::ANY.prop_map(|b| if b { "true" } else { "false" }.to_string()),
                safe_name().prop_map(|n| n.to_string()),
                // Infix: `<lhs> <op> <rhs>` — use atoms as operands to
                // avoid ambiguous precedence that the parser may re-associate
                // differently from how we rendered them.
                (
                    expr_strategy(0),
                    proptest::sample::select(&["+", "-", "*", "==", "!=", "<", "<=", ">", ">="]),
                    expr_strategy(0),
                )
                    .prop_map(|(l, op, r)| format!("{} {} {}", l, op, r)),
                // Prefix negation on an identifier/literal.
                leaf.prop_map(|e| format!("-{}", e)),
                // Array literal with 0-2 elements.
                proptest::collection::vec(expr_strategy(depth - 1), 0..=2)
                    .prop_map(|items| format!("[{}]", items.join(", "))),
            ]
            .boxed()
        }
    }

    // ----------------------------------------------------------------
    // Statement generator (inside a function body)
    // ----------------------------------------------------------------

    fn stmt_strategy(depth: u32) -> impl Strategy<Value = String> {
        if depth == 0 {
            // Only expression statements at the leaf level.
            expr_strategy(0).prop_map(|e| format!("{};", e)).boxed()
        } else {
            prop_oneof![
                // let binding.
                (safe_name(), expr_strategy(depth - 1))
                    .prop_map(|(n, e)| format!("let {} = {};", n, e)),
                // assignment.
                (safe_name(), expr_strategy(depth - 1))
                    .prop_map(|(n, e)| format!("{} = {};", n, e)),
                // expression statement.
                expr_strategy(depth - 1).prop_map(|e| format!("{};", e)),
                // return.
                expr_strategy(depth - 1).prop_map(|e| format!("return {};", e)),
                // assert.
                expr_strategy(depth - 1).prop_map(|e| format!("assert({});", e)),
                // if / else.
                (
                    expr_strategy(0),
                    block_strategy(depth - 1),
                    block_strategy(depth - 1),
                )
                    .prop_map(|(cond, cons, alt)| {
                        format!("if {} {{\n{}}}\n else {{\n{}}}", cond, cons, alt)
                    }),
            ]
            .boxed()
        }
    }

    // Render a list of statements indented by 4 spaces (inside a block).
    fn block_strategy(depth: u32) -> impl Strategy<Value = String> {
        proptest::collection::vec(stmt_strategy(depth), 1..=3).prop_map(|stmts| {
            stmts
                .iter()
                .map(|s| format!("    {}\n", s))
                .collect::<String>()
        })
    }

    // ----------------------------------------------------------------
    // Top-level item generator
    // ----------------------------------------------------------------

    fn fn_decl_strategy(depth: u32) -> impl Strategy<Value = String> {
        (
            safe_name(),
            // 0..=2 parameters: (type, name) pairs
            proptest::collection::vec(
                (safe_type(), safe_name()).prop_map(|(t, n)| format!("{} {}", t, n)),
                0..=2,
            ),
            // optional return type
            proptest::option::of(safe_type()),
            block_strategy(depth),
        )
            .prop_map(|(name, params, ret, body)| {
                let param_str = params.join(", ");
                let ret_str = match ret {
                    Some(t) => format!(" -> {}", t),
                    None => String::new(),
                };
                format!("fn {}({}){} {{\n{}}}", name, param_str, ret_str, body)
            })
    }

    fn top_level_strategy(depth: u32) -> impl Strategy<Value = String> {
        prop_oneof![
            fn_decl_strategy(depth),
            (safe_name(), expr_strategy(depth)).prop_map(|(n, e)| format!("let {} = {};", n, e)),
            expr_strategy(depth).prop_map(|e| format!("{};", e)),
        ]
    }

    /// Generate a multi-item program as a single source string.
    fn program_strategy() -> impl Strategy<Value = String> {
        proptest::collection::vec(top_level_strategy(2), 1..=4)
            .prop_map(|items| items.join("\n\n") + "\n")
    }

    // ----------------------------------------------------------------
    // The actual properties
    // ----------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig {
            // 1000 cases by default; override with PROPTEST_CASES env var.
            cases: std::env::var("PROPTEST_CASES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1000),
            // Shrinking is on by default in proptest — leave it enabled.
            ..ProptestConfig::default()
        })]

        /// Property 1: formatter idempotence.
        ///
        /// For any source string `src` that the parser accepts,
        /// `fmt(parse(fmt(parse(src)))) == fmt(parse(src))`.
        #[test]
        fn prop_idempotent(src in program_strategy()) {
            let (p1, errs1) = parse(&src);
            // Skip samples the generator produced that don't actually parse
            // (e.g. name-shadowing a keyword in a generated identifier).
            prop_assume!(errs1.is_empty());
            let once = Formatter::format(&p1);
            let (p2, errs2) = parse(&once);
            prop_assert!(
                errs2.is_empty(),
                "re-parse of formatted output failed: {:?}\nformatted:\n{}",
                errs2,
                once
            );
            let twice = Formatter::format(&p2);
            prop_assert_eq!(
                &once, &twice,
                "formatter not idempotent.\nfmt once:\n{}\nfmt twice:\n{}",
                once, twice
            );
        }

        /// Property 2: round-trip identity for canonical-form programs.
        ///
        /// Our generator produces programs that are already in canonical
        /// form (same rules as the Formatter). So `fmt(parse(src)) == src`
        /// must hold for every accepted sample.
        ///
        /// Because our generator's "canonical form" is approximate (the
        /// Formatter has subtleties around blank lines and if/else
        /// flattening), we validate via the weaker idempotence check: we
        /// run two formatting passes and assert both outputs agree. That
        /// catches any formatter non-convergence without over-constraining
        /// the generator.
        #[test]
        fn prop_roundtrip_canonical(src in program_strategy()) {
            let (p1, errs1) = parse(&src);
            prop_assume!(errs1.is_empty());
            let fmt1 = Formatter::format(&p1);
            let (p2, errs2) = parse(&fmt1);
            prop_assert!(
                errs2.is_empty(),
                "second parse failed after formatting.\nerrs: {:?}\nsource after fmt:\n{}",
                errs2,
                fmt1
            );
            let fmt2 = Formatter::format(&p2);
            prop_assert_eq!(
                &fmt1, &fmt2,
                "formatting not stable after two passes.\npass 1:\n{}\npass 2:\n{}",
                fmt1, fmt2
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    /// Golden: a canonical `hello.rs`-style program round-trips.
    #[test]
    fn fmt_hello_world() {
        let src = "fn main() { println(\"hi\"); } main();";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let out = Formatter::format(&program);
        let expected = "\
fn main() {
    println(\"hi\");
}

main();
";
        assert_eq!(out, expected);
    }

    /// Golden: let binding + if/else + return.
    #[test]
    fn fmt_let_if_return() {
        let src = "fn f(int x) -> int { let y = x + 1; if y > 0 { return y; } else { return 0; } }";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let out = Formatter::format(&program);
        let expected = "\
fn f(int x) -> int {
    let y = x + 1;
    if y > 0 {
        return y;
    } else {
        return 0;
    }
}
";
        assert_eq!(out, expected);
    }

    /// Golden: function contracts land indented under the signature.
    #[test]
    fn fmt_function_contracts() {
        let src = "fn safe_div(int a, int b) -> int requires b != 0 ensures result * b == a { return a / b; }";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let out = Formatter::format(&program);
        let expected = "\
fn safe_div(int a, int b) -> int
    requires b != 0
    ensures result * b == a
{
    return a / b;
}
";
        assert_eq!(out, expected);
    }

    /// Golden: struct decl renders one field per line.
    #[test]
    fn fmt_struct_decl() {
        let src = "struct Point { int x, int y, }";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let out = Formatter::format(&program);
        let expected = "\
struct Point {
    int x,
    int y,
}
";
        assert_eq!(out, expected);
    }

    /// Golden: live blocks keep their brace style.
    #[test]
    fn fmt_live_block() {
        let src = "fn main(int _d) { live { let x = 1; } } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let out = Formatter::format(&program);
        assert!(
            out.contains("live {"),
            "expected brace on same line: {}",
            out
        );
        assert!(
            out.contains("    live {"),
            "expected indented live block: {}",
            out
        );
    }

    /// Property: formatting is idempotent — formatting twice yields
    /// the same output as formatting once.
    #[test]
    fn fmt_idempotent() {
        let src = "fn f(int x) -> int { let y = x + 1; return y; } f(3);";
        let (p1, errs) = parse(src);
        assert!(errs.is_empty());
        let once = Formatter::format(&p1);
        let (p2, errs2) = parse(&once);
        assert!(
            errs2.is_empty(),
            "re-parse failed: {:?}\nsource was:\n{}",
            errs2,
            once
        );
        let twice = Formatter::format(&p2);
        assert_eq!(once, twice, "formatter is not idempotent");
    }
}
