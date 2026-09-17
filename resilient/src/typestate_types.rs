//! Feature 13/50 - Temporal Type States.
//!
//! `#[typestate(states = "Closed Open Flushed", transitions = "Closed:open->Open Open:flush->Flushed Open:close->Closed Flushed:close->Closed")]`
//! attached struct turns it into typestate type: value's
//! state evolves across method calls, and calls that violate the
//! state machine are rejected.
//!
//! This module applies to stateful objects such as file handles,
//! MMIO peripherals, lock guards, and parser cursors.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct TypestateSpec {
    pub struct_name: String,
    pub states: Vec<String>,
    /// Map of (current_state, method) -> next_state.
    pub transitions: HashMap<(String, String), String>,
}

static SPECS: RwLock<Vec<TypestateSpec>> = RwLock::new(Vec::new());

fn diag(source_path: &str, line: usize, msg: impl AsRef<str>) -> String {
    format!("{source_path}:{}:0: error: {}", line, msg.as_ref())
}

fn parse_quoted_string(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return None;
    }
    Some(&value[1..value.len() - 1])
}

fn parse_states(spec: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for state in spec.split_whitespace() {
        if state.is_empty() {
            return Err("empty state name".to_string());
        }
        out.push(state.to_string());
    }
    if out.is_empty() {
        return Err("no states declared".to_string());
    }
    Ok(out)
}

fn parse_transitions(spec: &str) -> Result<HashMap<(String, String), String>, String> {
    let mut out = HashMap::new();
    for step in spec.split_whitespace() {
        let Some((lhs, next)) = step.split_once("->") else {
            return Err(format!("malformed transition `{step}`"));
        };
        let Some((state, method)) = lhs.split_once(':') else {
            return Err(format!("malformed transition `{step}`"));
        };
        let state = state.trim();
        let method = method.trim();
        let next = next.trim();
        if state.is_empty() || method.is_empty() || next.is_empty() {
            return Err(format!("malformed transition `{step}`"));
        }
        out.insert((state.to_string(), method.to_string()), next.to_string());
    }
    if out.is_empty() {
        return Err("no transitions declared".to_string());
    }
    Ok(out)
}

fn validate_transition_semantics(
    item: &str,
    source_path: &str,
    line: usize,
    states: &[String],
    transitions: &HashMap<(String, String), String>,
) -> Result<(), String> {
    let state_set: std::collections::HashSet<_> = states.iter().cloned().collect();

    for ((src_state, _method), dst_state) in transitions.iter() {
        if !state_set.contains(src_state) {
            return Err(diag(
                source_path,
                line,
                format!(
                    "typestate `{item}`: transition references undefined source state `{src_state}`"
                ),
            ));
        }
        if !state_set.contains(dst_state) {
            return Err(diag(
                source_path,
                line,
                format!(
                    "typestate `{item}`: transition references undefined target state `{dst_state}`"
                ),
            ));
        }
    }

    if !states.is_empty() {
        let first_state = &states[0];
        if !transitions.iter().any(|((src, _), _)| src == first_state) {
            return Err(diag(
                source_path,
                line,
                format!(
                    "typestate `{item}`: initial state `{first_state}` has no outgoing transitions"
                ),
            ));
        }
    }

    Ok(())
}

fn parse_spec(
    source_path: &str,
    line: usize,
    item: &str,
    args: &str,
) -> Result<TypestateSpec, String> {
    let mut states = None;
    let mut transitions = None;

    for chunk in args.split(',') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            return Err(diag(
                source_path,
                line,
                format!("malformed typestate attribute on `{item}`: empty argument"),
            ));
        }
        let Some((key, value)) = chunk.split_once('=') else {
            return Err(diag(
                source_path,
                line,
                format!("malformed typestate attribute on `{item}`: expected `key = value`"),
            ));
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "states" => {
                if states.is_some() {
                    return Err(diag(
                        source_path,
                        line,
                        format!("duplicate `states` argument on `{item}`"),
                    ));
                }
                let Some(parsed) = parse_quoted_string(value) else {
                    return Err(diag(
                        source_path,
                        line,
                        format!("typestate attribute on `{item}` requires quoted `states` string"),
                    ));
                };
                states = Some(parse_states(parsed).map_err(|msg| {
                    diag(
                        source_path,
                        line,
                        format!("typestate attribute on `{item}`: {msg}"),
                    )
                })?);
            }
            "transitions" => {
                if transitions.is_some() {
                    return Err(diag(
                        source_path,
                        line,
                        format!("duplicate `transitions` argument on `{item}`"),
                    ));
                }
                let Some(parsed) = parse_quoted_string(value) else {
                    return Err(diag(
                        source_path,
                        line,
                        format!(
                            "typestate attribute on `{item}` requires quoted `transitions` string"
                        ),
                    ));
                };
                transitions = Some(parse_transitions(parsed).map_err(|msg| {
                    diag(
                        source_path,
                        line,
                        format!("typestate attribute on `{item}`: {msg}"),
                    )
                })?);
            }
            _ => {
                return Err(diag(
                    source_path,
                    line,
                    format!("unknown typestate argument `{key}` on `{item}`"),
                ));
            }
        }
    }

    let Some(states) = states else {
        return Err(diag(
            source_path,
            line,
            format!("typestate attribute on `{item}` missing `states`"),
        ));
    };
    let Some(transitions) = transitions else {
        return Err(diag(
            source_path,
            line,
            format!("typestate attribute on `{item}` missing `transitions`"),
        ));
    };

    validate_transition_semantics(item, source_path, line, &states, &transitions)?;

    Ok(TypestateSpec {
        struct_name: item.to_string(),
        states,
        transitions,
    })
}

pub fn collect() -> Vec<TypestateSpec> {
    let attrs = crate::feature_attrs::find_kind("typestate");
    let mut out = Vec::with_capacity(attrs.len());

    for (item, rec) in attrs {
        let mut states = Vec::new();
        let mut transitions = HashMap::new();
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((key, value)) = chunk.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                if let Some(parsed) = parse_quoted_string(value) {
                    match key {
                        "states" => {
                            if let Ok(parsed_states) = parse_states(parsed) {
                                states = parsed_states;
                            }
                        }
                        "transitions" => {
                            if let Ok(parsed_transitions) = parse_transitions(parsed) {
                                transitions = parsed_transitions;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        out.push(TypestateSpec {
            struct_name: item,
            states,
            transitions,
        });
    }

    out
}

pub fn install(specs: Vec<TypestateSpec>) {
    if let Ok(mut g) = SPECS.write() {
        *g = specs;
    }
}

pub fn validate_call(
    struct_name: &str,
    current_state: &str,
    method: &str,
) -> Result<String, String> {
    let g = SPECS
        .read()
        .map_err(|_| format!("no typestate for {struct_name}"))?;
    let spec = g
        .iter()
        .find(|s| s.struct_name == struct_name)
        .ok_or_else(|| format!("no typestate for {struct_name}"))?;
    spec.transitions
        .get(&(current_state.to_string(), method.to_string()))
        .cloned()
        .ok_or_else(|| {
            format!(
                "typestate violation: cannot call `{}` on `{}` in state `{}`",
                method, struct_name, current_state
            )
        })
}

#[derive(Debug, Clone)]
struct TrackedValue {
    struct_name: String,
    state: String,
}

struct CallSiteAnalyzer<'a> {
    specs: &'a [TypestateSpec],
    bindings: HashMap<String, TrackedValue>,
    source_path: &'a str,
}

impl<'a> CallSiteAnalyzer<'a> {
    fn new(specs: &'a [TypestateSpec], params: &[(String, String)], source_path: &'a str) -> Self {
        let mut analyzer = Self {
            specs,
            bindings: HashMap::with_capacity(params.len()),
            source_path,
        };
        for (type_name, name) in params {
            if let Some(spec) = analyzer.spec_for_type(type_name) {
                analyzer.bindings.insert(
                    name.clone(),
                    TrackedValue {
                        struct_name: spec.struct_name.clone(),
                        state: initial_state(spec),
                    },
                );
            }
        }
        analyzer
    }

    fn spec_for_type(&self, type_name: &str) -> Option<&TypestateSpec> {
        let type_name = normalize_type_name(type_name);
        self.specs.iter().find(|spec| spec.struct_name == type_name)
    }

    fn scan_body(&mut self, body: &Node) -> Result<(), String> {
        match body {
            Node::Block { stmts, .. } => {
                for stmt in stmts {
                    self.scan_stmt(stmt)?;
                }
            }
            other => self.scan_stmt(other)?,
        }
        Ok(())
    }

    fn scan_stmt(&mut self, node: &Node) -> Result<(), String> {
        match node {
            Node::Block { stmts, .. } => {
                for stmt in stmts {
                    self.scan_stmt(stmt)?;
                }
            }
            Node::LetStatement {
                name,
                value,
                type_annot,
                ..
            } => {
                self.scan_expr(value)?;
                self.bindings.remove(name);
                if let Some(struct_name) = self.struct_name_from_expr(value) {
                    if type_annot
                        .as_deref()
                        .is_none_or(|ty| normalize_type_name(ty) == struct_name)
                    {
                        self.bindings.insert(
                            name.clone(),
                            TrackedValue {
                                struct_name: struct_name.to_string(),
                                state: self.initial_state_for(struct_name),
                            },
                        );
                    }
                }
            }
            Node::StaticLet { name, value, .. } | Node::Const { name, value, .. } => {
                self.scan_expr(value)?;
                self.bindings.remove(name);
            }
            Node::Assignment { name, value, .. } => {
                self.scan_expr(value)?;
                self.bindings.remove(name);
                if let Some(struct_name) = self.struct_name_from_expr(value) {
                    self.bindings.insert(
                        name.clone(),
                        TrackedValue {
                            struct_name: struct_name.to_string(),
                            state: self.initial_state_for(struct_name),
                        },
                    );
                }
            }
            Node::LetTupleDestructure { names, value, .. } => {
                self.scan_expr(value)?;
                for name in names {
                    self.bindings.remove(name);
                }
            }
            Node::LetDestructureStruct { fields, value, .. } => {
                self.scan_expr(value)?;
                for (_, name) in fields {
                    self.bindings.remove(name);
                }
            }
            Node::ExpressionStatement { expr, .. } => self.scan_expr(expr)?,
            Node::ReturnStatement {
                value: Some(value), ..
            } => self.scan_expr(value)?,
            Node::DeferStatement { expr, .. } => {
                self.scan_expr(expr)?;
                self.bindings.clear();
            }
            Node::IfStatement { .. }
            | Node::WhileStatement { .. }
            | Node::ForInStatement { .. }
            | Node::Match { .. }
            | Node::TryCatch { .. } => {
                // A state may be advanced on only one path, so retaining it
                // after control flow would turn a path-dependent fact into a
                // false compile-time error. Unknown is the safe result until
                // a flow-sensitive typestate analysis exists.
                self.bindings.clear();
            }
            other => self.scan_expr(other)?,
        }
        Ok(())
    }

    fn scan_expr(&mut self, node: &Node) -> Result<(), String> {
        match node {
            Node::CallExpression {
                function,
                arguments,
                span,
            } => {
                let field_receiver = match function.as_ref() {
                    Node::FieldAccess { target, field, .. } => {
                        self.scan_expr(target)?;
                        Some((binding_name(target).map(str::to_owned), field.as_str()))
                    }
                    _ => {
                        self.scan_expr(function)?;
                        None
                    }
                };
                for argument in arguments {
                    self.scan_expr(argument)?;
                }

                let free_function = match function.as_ref() {
                    Node::Identifier { name, .. } => Some(name.as_str()),
                    _ => None,
                };
                let direct_argument_names: Vec<String> = arguments
                    .iter()
                    .filter_map(binding_name)
                    .map(str::to_owned)
                    .collect();

                let consumed = if let Some((receiver, method)) = field_receiver {
                    receiver
                        .as_deref()
                        .map(|name| self.apply_transition(name, method, span))
                        .transpose()?
                        .flatten()
                } else if let (Some(method), Some(receiver)) =
                    (free_function, arguments.first().and_then(binding_name))
                {
                    self.apply_transition(receiver, method, span)?
                } else {
                    None
                };

                for name in direct_argument_names {
                    if consumed.as_deref() != Some(name.as_str()) {
                        self.bindings.remove(&name);
                    }
                }
            }
            Node::InfixExpression { left, right, .. } => {
                self.scan_expr(left)?;
                self.scan_expr(right)?;
            }
            Node::PrefixExpression { right, .. }
            | Node::TryExpression { expr: right, .. }
            | Node::NewtypeConstruct { value: right, .. } => self.scan_expr(right)?,
            Node::FieldAccess { target, .. } => self.scan_expr(target)?,
            Node::FieldAssignment { target, value, .. } => {
                self.scan_expr(target)?;
                self.scan_expr(value)?;
                if let Some(name) = binding_name(target) {
                    self.bindings.remove(name);
                }
            }
            Node::IndexExpression { target, index, .. } => {
                self.scan_expr(target)?;
                self.scan_expr(index)?;
            }
            Node::IndexAssignment {
                target,
                index,
                value,
                ..
            } => {
                self.scan_expr(target)?;
                self.scan_expr(index)?;
                self.scan_expr(value)?;
                if let Some(name) = binding_name(target) {
                    self.bindings.remove(name);
                }
            }
            Node::Slice { target, lo, hi, .. } => {
                self.scan_expr(target)?;
                if let Some(lo) = lo {
                    self.scan_expr(lo)?;
                }
                if let Some(hi) = hi {
                    self.scan_expr(hi)?;
                }
            }
            Node::StructLiteral { fields, base, .. } => {
                for (_, value) in fields {
                    self.scan_expr(value)?;
                }
                if let Some(base) = base {
                    self.scan_expr(base)?;
                }
            }
            Node::ArrayLiteral { items, .. } | Node::TupleLiteral { items, .. } => {
                for item in items {
                    self.scan_expr(item)?;
                }
            }
            Node::MapLiteral { entries, .. } => {
                for (key, value) in entries {
                    self.scan_expr(key)?;
                    self.scan_expr(value)?;
                }
            }
            Node::SetLiteral { items, .. } => {
                for item in items {
                    self.scan_expr(item)?;
                }
            }
            Node::NamedArg { value, .. } => self.scan_expr(value)?,
            Node::Assert {
                condition, message, ..
            }
            | Node::Assume {
                condition, message, ..
            } => {
                self.scan_expr(condition)?;
                if let Some(message) = message {
                    self.scan_expr(message)?;
                }
            }
            Node::UnsafeBlock { body, .. } => self.scan_stmt(body)?,
            Node::StaticAssert { condition, .. } => self.scan_expr(condition)?,
            Node::IfStatement { .. }
            | Node::WhileStatement { .. }
            | Node::ForInStatement { .. }
            | Node::Match { .. }
            | Node::OptionalChain { .. }
            | Node::FunctionLiteral { .. }
            | Node::Quantifier { .. }
            | Node::BenchBlock { .. } => self.bindings.clear(),
            Node::Block { .. } => self.bindings.clear(),
            _ => {}
        }
        Ok(())
    }

    fn struct_name_from_expr<'b>(&self, value: &'b Node) -> Option<&'b str> {
        let Node::StructLiteral { name, .. } = value else {
            return None;
        };
        self.specs
            .iter()
            .find(|spec| spec.struct_name == *name)
            .map(|_| name.as_str())
    }

    fn initial_state_for(&self, struct_name: &str) -> String {
        self.specs
            .iter()
            .find(|spec| spec.struct_name == struct_name)
            .map(initial_state)
            .unwrap_or_default()
    }

    fn apply_transition(
        &mut self,
        binding_name: &str,
        method: &str,
        span: &crate::span::Span,
    ) -> Result<Option<String>, String> {
        let Some(value) = self.bindings.get(binding_name).cloned() else {
            return Ok(None);
        };
        let Some(spec) = self
            .specs
            .iter()
            .find(|spec| spec.struct_name == value.struct_name)
        else {
            return Ok(None);
        };
        if !spec
            .transitions
            .keys()
            .any(|(_, transition_method)| transition_method == method)
        {
            return Ok(None);
        }

        let next = validate_call(&value.struct_name, &value.state, method).map_err(|msg| {
            format!(
                "{}:{}:{}: error: {}",
                self.source_path, span.start.line, span.start.column, msg
            )
        })?;
        if let Some(tracked) = self.bindings.get_mut(binding_name) {
            tracked.state = next;
        }
        Ok(Some(binding_name.to_string()))
    }
}

fn normalize_type_name(type_name: &str) -> &str {
    type_name
        .split_whitespace()
        .last()
        .unwrap_or(type_name)
        .trim_start_matches('&')
}

fn initial_state(spec: &TypestateSpec) -> String {
    spec.states.first().cloned().unwrap_or_default()
}

fn binding_name(node: &Node) -> Option<&str> {
    match node {
        Node::Identifier { name, .. } => Some(name),
        _ => None,
    }
}

fn validate_call_sites(
    program: &Node,
    source_path: &str,
    specs: &[TypestateSpec],
) -> Result<(), String> {
    let mut first_error = None;
    crate::uniqueness_walk::for_each_function(program, |_name, params, body| {
        if first_error.is_some() {
            return;
        }
        let mut analyzer = CallSiteAnalyzer::new(specs, params, source_path);
        if let Err(error) = analyzer.scan_body(body) {
            first_error = Some(error);
        }
    });
    first_error.map_or(Ok(()), Err)
}

fn collect_checked(source_path: &str) -> Result<Vec<TypestateSpec>, String> {
    let attrs = crate::feature_attrs::find_kind("typestate");
    let mut out = Vec::with_capacity(attrs.len());
    let mut seen = HashSet::with_capacity(attrs.len());

    for (item, rec) in attrs {
        if !seen.insert(item.clone()) {
            return Err(diag(
                source_path,
                rec.line,
                format!("duplicate typestate declaration `{item}`"),
            ));
        }
        out.push(parse_spec(source_path, rec.line, &item, &rec.args)?);
    }

    Ok(out)
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let specs = collect_checked(source_path)?;
    if specs.is_empty() {
        install(Vec::new());
        return Ok(());
    }
    install(specs.clone());
    validate_call_sites(program, source_path, &specs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_typestate(item: &str, args: &str, line: usize) {
        crate::feature_attrs::record(
            item,
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: args.into(),
                line,
            },
        );
    }

    #[test]
    fn file_protocol_validates_close_after_open() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate(
            "File",
            r#"states = "Closed Open", transitions = "Closed:open->Open Open:close->Closed""#,
            0,
        );
        install(collect());
        assert_eq!(
            validate_call("File", "Closed", "open").unwrap(),
            "Open".to_string()
        );
        assert!(validate_call("File", "Closed", "close").is_err());
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn invalid_method_in_valid_state_is_rejected() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate(
            "Lock",
            r#"states = "Locked Unlocked", transitions = "Locked:unlock->Unlocked Unlocked:lock->Locked""#,
            0,
        );
        install(collect());
        assert_eq!(
            validate_call("Lock", "Locked", "unlock").unwrap(),
            "Unlocked".to_string()
        );
        assert_eq!(
            validate_call("Lock", "Unlocked", "lock").unwrap(),
            "Locked".to_string()
        );
        assert!(validate_call("Lock", "Locked", "lock").is_err());
        assert!(validate_call("Lock", "Unlocked", "unlock").is_err());
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn unknown_struct_must_return_an_error() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate(
            "Known",
            r#"states = "S0 S1", transitions = "S0:next->S1""#,
            0,
        );
        install(collect());
        let result = validate_call("Nonexistent", "SomeState", "someMethod");
        assert!(result.is_err(), "unknown struct must return an error");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Nonexistent"),
            "error must name unknown struct: {msg}"
        );
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        install(Vec::new());
    }

    #[test]
    fn check_rejects_missing_states_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate("Thing", r#"transitions = "S0:go->S1""#, 11);
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:11:0: error:"), "{err}");
        assert!(err.contains("missing `states`"), "{err}");
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_missing_transitions_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate("Thing", r#"states = "S0 S1""#, 12);
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:12:0: error:"), "{err}");
        assert!(err.contains("missing `transitions`"), "{err}");
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_unknown_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate(
            "Thing",
            r#"states = "S0 S1", transitions = "S0:go->S1", mode = "strict""#,
            13,
        );
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:13:0: error:"), "{err}");
        assert!(err.contains("unknown typestate argument `mode`"), "{err}");
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_duplicate_declarations() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate(
            "Thing",
            r#"states = "S0 S1", transitions = "S0:go->S1""#,
            14,
        );
        record_typestate(
            "Thing",
            r#"states = "S0 S1", transitions = "S0:go->S1""#,
            15,
        );
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:15:0: error:"), "{err}");
        assert!(
            err.contains("duplicate typestate declaration `Thing`"),
            "{err}"
        );
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_malformed_transition() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate("Thing", r#"states = "S0 S1", transitions = "S0goS1""#, 16);
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:16:0: error:"), "{err}");
        assert!(err.contains("malformed transition"), "{err}");
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_unquoted_states_value() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate("Thing", r#"states = S0 S1, transitions = "S0:go->S1""#, 17);
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:17:0: error:"), "{err}");
        assert!(err.contains("requires quoted `states` string"), "{err}");
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_unquoted_transitions_value() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate("Thing", r#"states = "S0 S1", transitions = S0:go->S1"#, 18);
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:18:0: error:"), "{err}");
        assert!(
            err.contains("requires quoted `transitions` string"),
            "{err}"
        );
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_undefined_source_state() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate(
            "Thing",
            r#"states = "S0 S1", transitions = "S2:go->S1""#,
            19,
        );
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:19:0: error:"), "{err}");
        assert!(
            err.contains("undefined source state `S2`"),
            "error should mention undefined source state: {err}"
        );
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_undefined_target_state() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate(
            "Thing",
            r#"states = "S0 S1", transitions = "S0:go->S2""#,
            20,
        );
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:20:0: error:"), "{err}");
        assert!(
            err.contains("undefined target state `S2`"),
            "error should mention undefined target state: {err}"
        );
        install(Vec::new());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_initial_state_without_transitions() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate(
            "Thing",
            r#"states = "S0 S1", transitions = "S1:go->S0""#,
            21,
        );
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:21:0: error:"), "{err}");
        assert!(
            err.contains("initial state `S0` has no outgoing transitions"),
            "error should mention unreachable initial state: {err}"
        );
        install(Vec::new());
        crate::feature_attrs::reset();
    }
}
