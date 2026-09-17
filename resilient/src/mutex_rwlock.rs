//! RES-2583: Mutex and RwLock synchronization primitives.
//!
//! Provides interpreter-level wrappers around shared-state concurrency
//! primitives. In the single-threaded interpreter these are value containers
//! with explicit lock/unlock; compiled backends map them to real OS primitives.
//!
//! ## API
//!
//!   mutex_new(v)          → Mutex  — create a mutex wrapping value `v`
//!   mutex_lock(m)         → value  — acquire lock, return wrapped value
//!   mutex_unlock(m)       → void   — release lock (no-op in interpreter)
//!   mutex_try_lock(m)     → Option — non-blocking; always Some in interpreter
//!   rwlock_new(v)         → RwLock — create an RwLock wrapping value `v`
//!   rwlock_read(l)        → value  — acquire shared read, return value
//!   rwlock_write(l)       → value  — acquire exclusive write, return value
//!   rwlock_unlock(l)      → void   — release lock (no-op in interpreter)
//!
//! ## Notes
//!
//! Both `Mutex` and `RwLock` are backed by a single-element `Value::Array`.
//! The lock/unlock operations are no-ops in the interpreter; the API matches
//! what a compiled backend would use with real OS primitives.

use std::collections::HashMap;

use crate::Value;

type RResult<T> = Result<T, String>;

// ---------------------------------------------------------------------------
// Mutex builtins
// ---------------------------------------------------------------------------

/// `mutex_new(v) → Mutex` — wrap `v` in a new mutex.
pub(crate) fn builtin_mutex_new(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Array(vec![v.clone()])),
        _ => Err(format!(
            "mutex_new: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `mutex_lock(m) → value` — acquire the mutex and return the wrapped value.
/// In the interpreter this always succeeds immediately.
pub(crate) fn builtin_mutex_lock(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(cells)] if !cells.is_empty() => Ok(cells[0].clone()),
        [_] => Err("mutex_lock: argument is not a mutex".to_string()),
        _ => Err(format!(
            "mutex_lock: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `mutex_unlock(m) → void` — release the mutex. No-op in the interpreter.
pub(crate) fn builtin_mutex_unlock(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(_)] => Ok(Value::Void),
        [_] => Err("mutex_unlock: argument is not a mutex".to_string()),
        _ => Err(format!(
            "mutex_unlock: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `mutex_try_lock(m) → Option<value>` — non-blocking acquire.
/// In the interpreter this always returns `Some(value)`.
pub(crate) fn builtin_mutex_try_lock(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(cells)] if !cells.is_empty() => {
            Ok(Value::Option(Some(Box::new(cells[0].clone()))))
        }
        [Value::Array(_)] => Ok(Value::Option(None)),
        [_] => Err("mutex_try_lock: argument is not a mutex".to_string()),
        _ => Err(format!(
            "mutex_try_lock: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// RwLock builtins
// ---------------------------------------------------------------------------

/// `rwlock_new(v) → RwLock` — wrap `v` in a new read-write lock.
pub(crate) fn builtin_rwlock_new(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Array(vec![v.clone()])),
        _ => Err(format!(
            "rwlock_new: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `rwlock_read(l) → value` — acquire a shared read guard.
pub(crate) fn builtin_rwlock_read(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(cells)] if !cells.is_empty() => Ok(cells[0].clone()),
        [_] => Err("rwlock_read: argument is not an rwlock".to_string()),
        _ => Err(format!(
            "rwlock_read: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `rwlock_write(l) → value` — acquire an exclusive write guard.
pub(crate) fn builtin_rwlock_write(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(cells)] if !cells.is_empty() => Ok(cells[0].clone()),
        [_] => Err("rwlock_write: argument is not an rwlock".to_string()),
        _ => Err(format!(
            "rwlock_write: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `rwlock_unlock(l) → void` — release a read or write guard. No-op in interpreter.
pub(crate) fn builtin_rwlock_unlock(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(_)] => Ok(Value::Void),
        [_] => Err("rwlock_unlock: argument is not an rwlock".to_string()),
        _ => Err(format!(
            "rwlock_unlock: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// Advisory type-check pass
// ---------------------------------------------------------------------------

/// RES-3133: Validate mutex/rwlock builtin arguments at compile time.
/// Rejects invalid argument types in mutex_lock/unlock/try_lock and rwlock_read/write/unlock calls.
pub(crate) fn check(program: &crate::Node, source_path: &str) -> Result<(), String> {
    let mut analyzer = LockAnalyzer::new(source_path);
    analyzer.walk(program);

    if analyzer.errors.is_empty() {
        Ok(())
    } else {
        Err(analyzer.errors.join("\n"))
    }
}

/// A binding's statically provable lock origin.
///
/// `Unknown` is deliberately different from `DefinitelyNonLock`: parameters,
/// arbitrary calls, indexing, and field access may carry a lock, while a
/// literal or a literal collection cannot be a lock under the source-level
/// API. Keeping those cases distinct lets this advisory pass reject proven
/// mistakes without guessing about dynamic values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingKind {
    Mutex,
    RwLock,
    DefinitelyNonLock,
    Unknown,
}

/// Scope-aware origin analysis for the lock builtins.
///
/// The typechecker intentionally registers these builtins as `Any -> Any`,
/// so this pass cannot ask the type environment for a lock type. Instead it
/// tracks only direct origins and invalidates a binding on every uncertain
/// reassignment. Branches and loops are joined conservatively: a value is
/// retained only when every path agrees on its origin.
struct LockAnalyzer<'a> {
    source_path: &'a str,
    scopes: Vec<HashMap<String, BindingKind>>,
    errors: Vec<String>,
}

impl<'a> LockAnalyzer<'a> {
    fn new(source_path: &'a str) -> Self {
        Self {
            source_path,
            scopes: vec![HashMap::new()],
            errors: Vec::new(),
        }
    }

    fn fork(&self, scopes: Vec<HashMap<String, BindingKind>>) -> Self {
        Self {
            source_path: self.source_path,
            scopes,
            errors: Vec::new(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        debug_assert!(self.scopes.len() > 1);
        self.scopes.pop();
    }

    fn bind(&mut self, name: &str, kind: BindingKind) {
        self.scopes
            .last_mut()
            .expect("lock analyzer always has a root scope")
            .insert(name.to_string(), kind);
    }

    fn assign(&mut self, name: &str, kind: BindingKind) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), kind);
                return;
            }
        }
        // The normal typechecker rejects assignment to an undeclared name.
        // Keeping an entry here makes this pass safe when called directly on
        // a hand-built AST and prevents a stale origin from appearing later.
        self.bind(name, BindingKind::Unknown);
    }

    fn lookup(&self, name: &str) -> BindingKind {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .unwrap_or(BindingKind::Unknown)
    }

    fn expression_kind(&self, node: &crate::Node) -> BindingKind {
        match node {
            crate::Node::Identifier { name, .. } => self.lookup(name),
            crate::Node::CallExpression { function, .. } => {
                let crate::Node::Identifier { name, .. } = function.as_ref() else {
                    return BindingKind::Unknown;
                };
                match name.as_str() {
                    "mutex_new" => BindingKind::Mutex,
                    "rwlock_new" => BindingKind::RwLock,
                    _ => BindingKind::Unknown,
                }
            }
            crate::Node::IntegerLiteral { .. }
            | crate::Node::FloatLiteral { .. }
            | crate::Node::StringLiteral { .. }
            | crate::Node::StringInternLiteral { .. }
            | crate::Node::BytesLiteral { .. }
            | crate::Node::CharLiteral { .. }
            | crate::Node::BooleanLiteral { .. }
            | crate::Node::ArrayLiteral { .. }
            | crate::Node::MapLiteral { .. }
            | crate::Node::SetLiteral { .. }
            | crate::Node::TupleLiteral { .. }
            | crate::Node::StructLiteral { .. }
            | crate::Node::FunctionLiteral { .. } => BindingKind::DefinitelyNonLock,
            _ => BindingKind::Unknown,
        }
    }

    fn reassignment_kind(&self, node: &crate::Node) -> BindingKind {
        match node {
            crate::Node::CallExpression { function, .. }
                if matches!(
                    function.as_ref(),
                    crate::Node::Identifier { name, .. }
                        if matches!(name.as_str(), "mutex_new" | "rwlock_new")
                ) =>
            {
                self.expression_kind(node)
            }
            _ => BindingKind::Unknown,
        }
    }

    fn walk(&mut self, node: &crate::Node) {
        use crate::Node;

        match node {
            Node::Program(items) => {
                for item in items {
                    self.walk(&item.node);
                }
            }
            Node::Extern { decls, .. } => {
                for decl in decls {
                    self.walk_signature(&decl.parameters, &decl.requires, &decl.ensures);
                }
            }
            Node::Function {
                parameters,
                defaults,
                body,
                requires,
                ensures,
                recovers_to,
                ..
            } => self.walk_function(
                parameters,
                defaults,
                requires,
                ensures,
                recovers_to.as_deref(),
                Some(body),
            ),
            Node::ImplBlock { methods, .. } | Node::BlanketImpl { methods, .. } => {
                for method in methods {
                    self.walk(method);
                }
            }
            Node::ModuleDecl { body, .. } => self.walk_statements(body),
            Node::Actor {
                state_init,
                concurrent_ensures,
                handlers,
                ..
            } => {
                self.walk(state_init);
                for ensure in concurrent_ensures {
                    self.walk(ensure);
                }
                for handler in handlers {
                    self.walk_function(
                        &[],
                        &[],
                        &[],
                        &handler.ensures,
                        None,
                        Some(handler.body.as_ref()),
                    );
                }
            }
            Node::ActorDecl {
                state_fields,
                always_clauses,
                eventually_clauses,
                receive_handlers,
                handlers,
                ..
            } => {
                for (_, _, initializer) in state_fields {
                    self.walk(initializer);
                }
                for clause in always_clauses {
                    self.walk(clause);
                }
                for clause in eventually_clauses {
                    self.walk(&clause.post);
                }
                for handler in receive_handlers {
                    self.walk_function(
                        &handler.parameters,
                        &[],
                        &handler.requires,
                        &handler.ensures,
                        None,
                        Some(&handler.body),
                    );
                }
                for handler in handlers {
                    self.walk_function(
                        &[],
                        &[],
                        &[],
                        &handler.ensures,
                        None,
                        Some(handler.body.as_ref()),
                    );
                }
            }
            Node::ClusterDecl { invariants, .. } => {
                for invariant in invariants {
                    self.walk(invariant);
                }
            }
            Node::Block { stmts, .. } => {
                self.push_scope();
                self.walk_statements(stmts);
                self.pop_scope();
            }
            Node::LetStatement { name, value, .. }
            | Node::StaticLet { name, value, .. }
            | Node::Const { name, value, .. } => {
                self.walk(value);
                self.bind(name, self.expression_kind(value));
            }
            Node::LetDestructureStruct { fields, value, .. } => {
                self.walk(value);
                for (_, local_name) in fields {
                    self.bind(local_name, BindingKind::Unknown);
                }
            }
            Node::LetTupleDestructure { names, value, .. } => {
                self.walk(value);
                for name in names {
                    self.bind(name, BindingKind::Unknown);
                }
            }
            Node::Assignment { name, value, .. } => {
                self.walk(value);
                self.assign(name, self.reassignment_kind(value));
            }
            Node::ReturnStatement {
                value: Some(value), ..
            }
            | Node::BreakWith { value, .. }
            | Node::DeferStatement { expr: value, .. }
            | Node::NewtypeConstruct { value, .. }
            | Node::NamedArg { value, .. } => self.walk(value),
            Node::ReturnStatement { value: None, .. }
            | Node::Break { .. }
            | Node::BreakLabel { .. }
            | Node::Continue { .. }
            | Node::ContinueLabel { .. }
            | Node::Identifier { .. }
            | Node::IntegerLiteral { .. }
            | Node::FloatLiteral { .. }
            | Node::StringLiteral { .. }
            | Node::StringInternLiteral { .. }
            | Node::BytesLiteral { .. }
            | Node::CharLiteral { .. }
            | Node::BooleanLiteral { .. }
            | Node::DurationLiteral { .. }
            | Node::Use { .. }
            | Node::StructDecl { .. }
            | Node::TraitDecl { .. }
            | Node::TypeAlias { .. }
            | Node::RegionDecl { .. }
            | Node::NewtypeDecl { .. }
            | Node::SupervisorDecl { .. }
            | Node::EnumDecl { .. }
            | Node::RegionParam { .. } => {}
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                self.walk(condition);
                let base = self.scopes.clone();
                let mut then_branch = self.fork(base.clone());
                then_branch.walk(consequence);
                let mut branches = vec![then_branch];
                if let Some(alternative) = alternative {
                    let mut else_branch = self.fork(base.clone());
                    else_branch.walk(alternative);
                    branches.push(else_branch);
                } else {
                    branches.push(self.fork(base.clone()));
                }
                self.merge_branches(&base, branches);
            }
            Node::WhileStatement {
                condition,
                body,
                invariants,
                ..
            } => {
                self.walk(condition);
                let base = self.scopes.clone();
                let mut loop_branch = self.fork(base.clone());
                loop_branch.push_scope();
                for invariant in invariants {
                    loop_branch.walk(invariant);
                }
                loop_branch.walk(body);
                loop_branch.pop_scope();
                let no_loop = self.fork(base.clone());
                self.merge_branches(&base, vec![loop_branch, no_loop]);
            }
            Node::ForInStatement {
                name,
                iterable,
                body,
                invariants,
                ..
            } => {
                self.walk(iterable);
                let base = self.scopes.clone();
                let mut loop_branch = self.fork(base.clone());
                loop_branch.push_scope();
                loop_branch.bind(name, BindingKind::Unknown);
                for invariant in invariants {
                    loop_branch.walk(invariant);
                }
                loop_branch.walk(body);
                loop_branch.pop_scope();
                let no_loop = self.fork(base.clone());
                self.merge_branches(&base, vec![loop_branch, no_loop]);
            }
            Node::ExpressionStatement { expr, .. }
            | Node::TryExpression { expr, .. }
            | Node::InvariantStatement { expr, .. } => self.walk(expr),
            Node::CallExpression {
                function,
                arguments,
                span,
            } => {
                self.check_call(function, arguments, span);
                self.walk(function);
                for argument in arguments {
                    self.walk(argument);
                }
            }
            Node::FieldAccess { target, .. } => self.walk(target),
            Node::FieldAssignment { target, value, .. } => {
                self.walk(target);
                self.walk(value);
            }
            Node::IndexExpression { target, index, .. } => {
                self.walk(target);
                self.walk(index);
            }
            Node::IndexAssignment {
                target,
                index,
                value,
                ..
            } => {
                self.walk(target);
                self.walk(index);
                self.walk(value);
            }
            Node::InfixExpression { left, right, .. } => {
                self.walk(left);
                self.walk(right);
            }
            Node::PrefixExpression { right, .. } => self.walk(right),
            Node::ArrayLiteral { items, .. }
            | Node::SetLiteral { items, .. }
            | Node::TupleLiteral { items, .. } => {
                for item in items {
                    self.walk(item);
                }
            }
            Node::Match {
                scrutinee, arms, ..
            } => {
                self.walk(scrutinee);
                let base = self.scopes.clone();
                let mut branches = Vec::with_capacity(arms.len().max(1));
                for (pattern, guard, body) in arms {
                    let mut arm = self.fork(base.clone());
                    arm.walk_pattern(pattern);
                    arm.push_scope();
                    arm.bind_pattern(pattern);
                    if let Some(guard) = guard {
                        arm.walk(guard);
                    }
                    arm.walk(body);
                    arm.pop_scope();
                    branches.push(arm);
                }
                if branches.is_empty() {
                    self.scopes = base;
                } else {
                    self.merge_branches(&base, branches);
                }
            }
            Node::FunctionLiteral {
                parameters,
                body,
                requires,
                ensures,
                recovers_to,
                ..
            } => self.walk_function(
                parameters,
                &[],
                requires,
                ensures,
                recovers_to.as_deref(),
                Some(body),
            ),
            Node::Assert {
                condition, message, ..
            }
            | Node::Assume {
                condition, message, ..
            } => {
                self.walk(condition);
                if let Some(message) = message {
                    self.walk(message);
                }
            }
            Node::MapLiteral { entries, .. } => {
                for (key, value) in entries {
                    self.walk(key);
                    self.walk(value);
                }
            }
            Node::StructLiteral { fields, base, .. } => {
                if let Some(base) = base {
                    self.walk(base);
                }
                for (_, value) in fields {
                    self.walk(value);
                }
            }
            Node::Slice { target, lo, hi, .. } => {
                self.walk(target);
                if let Some(lo) = lo {
                    self.walk(lo);
                }
                if let Some(hi) = hi {
                    self.walk(hi);
                }
            }
            Node::Range { lo, hi, .. } => {
                self.walk(lo);
                self.walk(hi);
            }
            Node::TupleIndex { tuple, .. } => self.walk(tuple),
            Node::InterpolatedString { parts, .. } => {
                for part in parts {
                    if let crate::string_interp::StringPart::Expr(expr) = part {
                        self.walk(expr);
                    }
                }
            }
            Node::TryCatch { body, handlers, .. } => {
                let base = self.scopes.clone();
                let mut branches = Vec::with_capacity(handlers.len() + 1);
                let mut body_branch = self.fork(base.clone());
                body_branch.walk_statements(body);
                branches.push(body_branch);
                for (_, handler_body) in handlers {
                    let mut handler_branch = self.fork(base.clone());
                    handler_branch.walk_statements(handler_body);
                    branches.push(handler_branch);
                }
                self.merge_branches(&base, branches);
            }
            Node::OptionalChain { object, access, .. } => {
                self.walk(object);
                if let crate::ChainAccess::Method(_, arguments) = access {
                    for argument in arguments {
                        self.walk(argument);
                    }
                }
            }
            Node::LiveBlock {
                body,
                invariants,
                timeout,
                ..
            } => {
                if let Some(timeout) = timeout {
                    self.walk(timeout);
                }
                let base = self.scopes.clone();
                let mut live_branch = self.fork(base.clone());
                for invariant in invariants {
                    live_branch.walk(invariant);
                }
                live_branch.walk(body);
                let no_retry = self.fork(base.clone());
                self.merge_branches(&base, vec![live_branch, no_retry]);
            }
            Node::Quantifier {
                range, var, body, ..
            } => {
                let mut quantified = self.fork(self.scopes.clone());
                match range {
                    crate::quantifiers::QuantRange::Range { lo, hi } => {
                        quantified.walk(lo);
                        quantified.walk(hi);
                    }
                    crate::quantifiers::QuantRange::Iterable(iterable) => {
                        quantified.walk(iterable);
                    }
                }
                quantified.push_scope();
                quantified.bind(var, BindingKind::Unknown);
                quantified.walk(body);
                quantified.pop_scope();
                self.errors.extend(quantified.errors);
            }
            Node::UnsafeBlock { body, .. }
            | Node::BenchBlock { body, .. }
            | Node::StaticAssert {
                condition: body, ..
            } => self.walk(body),
        }
    }

    fn walk_statements(&mut self, statements: &[crate::Node]) {
        for statement in statements {
            self.walk(statement);
        }
    }

    fn walk_function(
        &mut self,
        parameters: &[(String, String)],
        defaults: &[Option<Box<crate::Node>>],
        requires: &[crate::Node],
        ensures: &[crate::Node],
        recovers_to: Option<&crate::Node>,
        body: Option<&crate::Node>,
    ) {
        let mut function = self.fork(self.scopes.clone());
        function.push_scope();
        for (_, name) in parameters {
            function.bind(name, BindingKind::Unknown);
        }
        for default in defaults.iter().flatten() {
            function.walk(default);
        }
        for require in requires {
            function.walk(require);
        }
        if let Some(body) = body {
            function.walk(body);
        }
        for ensure in ensures {
            function.walk(ensure);
        }
        if let Some(recovers_to) = recovers_to {
            function.walk(recovers_to);
        }
        function.pop_scope();
        self.errors.extend(function.errors);
    }

    fn walk_signature(
        &mut self,
        parameters: &[(String, String)],
        requires: &[crate::Node],
        ensures: &[crate::Node],
    ) {
        self.walk_function(parameters, &[], requires, ensures, None, None);
    }

    fn walk_pattern(&mut self, pattern: &crate::Pattern) {
        match pattern {
            crate::Pattern::Literal(node) => self.walk(node),
            crate::Pattern::Or(branches) => {
                for branch in branches {
                    self.walk_pattern(branch);
                }
            }
            crate::Pattern::Bind(_, inner)
            | crate::Pattern::Some(inner)
            | crate::Pattern::Ok(inner)
            | crate::Pattern::Err(inner) => self.walk_pattern(inner),
            crate::Pattern::Struct { fields, .. } => {
                for (_, pattern) in fields {
                    self.walk_pattern(pattern);
                }
            }
            crate::Pattern::EnumVariant { payload, .. } => match payload {
                crate::EnumPatternPayload::None => {}
                crate::EnumPatternPayload::Named(fields) => {
                    for (_, pattern) in fields {
                        self.walk_pattern(pattern);
                    }
                }
                crate::EnumPatternPayload::Tuple(patterns) => {
                    for pattern in patterns {
                        self.walk_pattern(pattern);
                    }
                }
            },
            crate::Pattern::TupleStruct { fields, .. } | crate::Pattern::Tuple(fields) => {
                for pattern in fields {
                    self.walk_pattern(pattern);
                }
            }
            crate::Pattern::Identifier(_)
            | crate::Pattern::Wildcard
            | crate::Pattern::Range { .. }
            | crate::Pattern::None => {}
        }
    }

    fn bind_pattern(&mut self, pattern: &crate::Pattern) {
        match pattern {
            crate::Pattern::Identifier(name) => self.bind(name, BindingKind::Unknown),
            crate::Pattern::Or(branches) => {
                for branch in branches {
                    self.bind_pattern(branch);
                }
            }
            crate::Pattern::Bind(name, inner) => {
                self.bind(name, BindingKind::Unknown);
                self.bind_pattern(inner);
            }
            crate::Pattern::Struct { fields, .. } => {
                for (_, pattern) in fields {
                    self.bind_pattern(pattern);
                }
            }
            crate::Pattern::Some(inner)
            | crate::Pattern::Ok(inner)
            | crate::Pattern::Err(inner) => self.bind_pattern(inner),
            crate::Pattern::EnumVariant { payload, .. } => match payload {
                crate::EnumPatternPayload::None => {}
                crate::EnumPatternPayload::Named(fields) => {
                    for (_, pattern) in fields {
                        self.bind_pattern(pattern);
                    }
                }
                crate::EnumPatternPayload::Tuple(patterns) => {
                    for pattern in patterns {
                        self.bind_pattern(pattern);
                    }
                }
            },
            crate::Pattern::TupleStruct { fields, .. } | crate::Pattern::Tuple(fields) => {
                for pattern in fields {
                    self.bind_pattern(pattern);
                }
            }
            crate::Pattern::Literal(_)
            | crate::Pattern::Wildcard
            | crate::Pattern::Range { .. }
            | crate::Pattern::None => {}
        }
    }

    fn merge_branches(&mut self, base: &[HashMap<String, BindingKind>], branches: Vec<Self>) {
        let branch_scopes: Vec<&[HashMap<String, BindingKind>]> = branches
            .iter()
            .map(|branch| branch.scopes.as_slice())
            .collect();
        let merged = merge_scopes(base, &branch_scopes);
        for branch in branches {
            self.errors.extend(branch.errors);
        }
        self.scopes = merged;
    }

    fn check_call(
        &mut self,
        function: &crate::Node,
        arguments: &[crate::Node],
        span: &crate::span::Span,
    ) {
        let crate::Node::Identifier { name, .. } = function else {
            return;
        };
        let expected = match name.as_str() {
            "mutex_lock" | "mutex_unlock" | "mutex_try_lock" => BindingKind::Mutex,
            "rwlock_read" | "rwlock_write" | "rwlock_unlock" => BindingKind::RwLock,
            _ => return,
        };
        if arguments.len() != 1 {
            return;
        }

        let actual = self.expression_kind(&arguments[0]);
        let reason = match (expected, actual) {
            (BindingKind::Mutex, BindingKind::Mutex)
            | (BindingKind::RwLock, BindingKind::RwLock)
            | (_, BindingKind::Unknown) => return,
            (BindingKind::Mutex, BindingKind::RwLock) => {
                "the argument is a RwLock created with rwlock_new, not a Mutex"
            }
            (BindingKind::RwLock, BindingKind::Mutex) => {
                "the argument is a Mutex created with mutex_new, not an RwLock"
            }
            (BindingKind::Mutex, BindingKind::DefinitelyNonLock) => {
                "got type that cannot be a mutex"
            }
            (BindingKind::RwLock, BindingKind::DefinitelyNonLock) => {
                "got type that cannot be an rwlock"
            }
            _ => unreachable!("lock kind matching is exhaustive"),
        };
        let expected_name = match expected {
            BindingKind::Mutex => "Mutex",
            BindingKind::RwLock => "RwLock",
            _ => unreachable!(),
        };
        self.errors.push(format!(
            "{}:{}:{}: error[mutex]: `{}` expects a {} argument, but {}",
            self.source_path, span.start.line, span.start.column, name, expected_name, reason
        ));
    }
}

fn merge_scopes(
    base: &[HashMap<String, BindingKind>],
    branches: &[&[HashMap<String, BindingKind>]],
) -> Vec<HashMap<String, BindingKind>> {
    let mut merged = base.to_vec();
    for (scope_index, base_scope) in base.iter().enumerate() {
        for (name, base_kind) in base_scope {
            let mut kind = *base_kind;
            for branch in branches {
                let branch_kind = branch
                    .get(scope_index)
                    .and_then(|scope| scope.get(name))
                    .copied()
                    .unwrap_or(*base_kind);
                if branch_kind != kind {
                    kind = BindingKind::Unknown;
                    break;
                }
            }
            merged[scope_index].insert(name.clone(), kind);
        }
    }
    merged
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> String {
        let r = run_program(src);
        assert!(r.ok, "program failed: {:?}", r.errors);
        r.stdout
    }

    #[test]
    fn mutex_new_and_lock() {
        let out = run(r#"
let m = mutex_new(42);
let v = mutex_lock(m);
println(to_string(v));
mutex_unlock(m);
"#);
        assert!(out.contains("42"), "got: {out:?}");
    }

    #[test]
    fn mutex_try_lock_returns_some() {
        let out = run(r#"
let m = mutex_new("hello");
let opt = mutex_try_lock(m);
println(to_string(is_some(opt)));
"#);
        assert!(out.contains("true"), "got: {out:?}");
    }

    #[test]
    fn rwlock_read_and_write() {
        let out = run(r#"
let l = rwlock_new(100);
let r = rwlock_read(l);
println(to_string(r));
let w = rwlock_write(l);
println(to_string(w));
rwlock_unlock(l);
"#);
        assert!(out.contains("100"), "got: {out:?}");
    }

    #[test]
    fn mutex_wraps_different_types() {
        let out = run(r#"
let m1 = mutex_new(true);
let m2 = mutex_new("world");
let v1 = mutex_lock(m1);
let v2 = mutex_lock(m2);
println(to_string(v1));
println(v2);
"#);
        assert!(out.contains("true"), "got: {out:?}");
        assert!(out.contains("world"), "got: {out:?}");
    }

    // ── Malformed-input regression corpus (RES-3174) ──────────────
    #[test]
    fn corpus_mutex_lock_with_non_mutex() {
        let src = r#"
let x = 42;
let result = mutex_lock(x);
println(to_string(result));
"#;
        let r = crate::run_program(src);
        assert!(!r.ok, "mutex_lock with non-mutex should fail");
        assert!(
            r.errors.iter().any(|e| e.contains("mutex")),
            "error should mention mutex"
        );
    }

    #[test]
    fn corpus_rwlock_write_with_invalid_arg() {
        let src = r#"
let s = "invalid";
let result = rwlock_write(s);
"#;
        let r = crate::run_program(src);
        assert!(!r.ok, "rwlock_write with string should fail");
        assert!(
            r.errors.iter().any(|e| e.contains("rwlock")),
            "error should mention rwlock"
        );
    }

    #[test]
    fn corpus_mutex_try_lock_null_value() {
        let src = r#"
let not_mutex = false;
let opt = mutex_try_lock(not_mutex);
"#;
        let r = crate::run_program(src);
        assert!(!r.ok, "mutex_try_lock with non-mutex should fail");
        assert!(
            r.errors.iter().any(|e| e.contains("mutex")),
            "error should mention mutex"
        );
    }
}
