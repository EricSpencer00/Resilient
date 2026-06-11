//! Stateright bridge — `rz stateright check <file.rz>`.
//!
//! The first integration targets a narrow actor-state subset and translates it
//! into a Stateright `Model`. This makes the bridge useful today without
//! overclaiming support for the full language or full runtime actor semantics.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use crate::Node;
use crate::span::Span;
use stateright::{Checker, Model, Property};
use std::fs;
use std::path::Path;

const DEFAULT_MAX_DEPTH: usize = 256;
const DEFAULT_STATE_BOUND: i64 = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckOutcome {
    Clean,
    Violated,
    Unsupported,
    ParseError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckResult {
    outcome: CheckOutcome,
    diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BridgeState {
    value: i64,
}

#[derive(Debug, Clone)]
struct BridgeHandler {
    requires: Vec<Node>,
    next_state: Node,
}

#[derive(Debug, Clone)]
struct BridgeModel {
    actor_name: String,
    state_name: String,
    invariants: Vec<Node>,
    invariant_labels: Vec<String>,
    handlers: Vec<BridgeHandler>,
    initial_state: i64,
    property_name: &'static str,
    boundary_abs: i64,
}

impl Model for BridgeModel {
    type State = BridgeState;
    type Action = usize;

    fn init_states(&self) -> Vec<Self::State> {
        vec![BridgeState {
            value: self.initial_state,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend(0..self.handlers.len());
    }

    fn next_state(&self, last_state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let handler = self.handlers.get(action)?;
        if !handler
            .requires
            .iter()
            .all(|req| eval_bool(req, &self.state_name, last_state.value).unwrap_or(false))
        {
            return None;
        }
        let next = eval_int(&handler.next_state, &self.state_name, last_state.value).ok()?;
        Some(BridgeState { value: next })
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![Property::always(self.property_name, bridge_invariants_hold)]
    }

    fn within_boundary(&self, state: &Self::State) -> bool {
        state.value.abs() <= self.boundary_abs
    }
}

fn bridge_invariants_hold(model: &BridgeModel, state: &BridgeState) -> bool {
    model
        .invariants
        .iter()
        .all(|inv| eval_bool(inv, &model.state_name, state.value).unwrap_or(false))
}

pub fn dispatch_stateright_subcommand(args: &[String]) -> Option<i32> {
    let first = args.first()?;
    if first != "stateright" {
        return None;
    }
    let verb = args.get(1).map(String::as_str).unwrap_or("--help");
    match verb {
        "check" => Some(run_stateright_check(&args[2..])),
        "--help" | "-h" | "help" => {
            print_stateright_help();
            Some(0)
        }
        other => {
            eprintln!(
                "Error: unknown `stateright` subcommand `{}`. Try `rz stateright --help`.",
                other
            );
            Some(1)
        }
    }
}

fn run_stateright_check(args: &[String]) -> i32 {
    if is_stateright_check_help_args(args) {
        print_stateright_check_help();
        return 0;
    }

    let mut input_file: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            f if !f.starts_with('-') => input_file = Some(f),
            flag => {
                eprintln!(
                    "Error: unknown flag `{}`. Try `rz stateright check --help`.",
                    flag
                );
                return 1;
            }
        }
        i += 1;
    }

    let file = match input_file {
        Some(file) => file,
        None => {
            eprintln!("Error: `rz stateright check` requires a .rz file path.");
            eprintln!("Usage: rz stateright check <file.rz>");
            return 1;
        }
    };

    let path = Path::new(file);
    if !path.exists() {
        eprintln!("Error: file not found: {}", path.display());
        return 1;
    }

    let src = match fs::read_to_string(path) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("Error: could not read {}: {}", path.display(), e);
            return 1;
        }
    };

    let result = check_source(&src, file);
    for diag in &result.diagnostics {
        match result.outcome {
            CheckOutcome::Clean => println!("{diag}"),
            _ => eprintln!("{diag}"),
        }
    }
    match result.outcome {
        CheckOutcome::Clean => 0,
        CheckOutcome::Violated | CheckOutcome::Unsupported | CheckOutcome::ParseError => 1,
    }
}

const STATERIGHT_HELP_TEXT: &str = r#"rz stateright — model-check a Resilient actor subset with Stateright

USAGE:
    rz stateright <SUBCOMMAND>

SUBCOMMANDS:
    check      Model-check a supported `.rz` actor program
    help       Show this help text

Run `rz stateright check --help` for details on the verifier entry point.
"#;

const STATERIGHT_CHECK_HELP_TEXT: &str = r#"rz stateright check — model-check a Resilient actor subset with Stateright

USAGE:
    rz stateright check <file.rz>

SUPPORTED TODAY:
    - exactly one actor declaration
    - one integer actor state field
    - `always:` safety invariants
    - straight-line `receive` handlers

EXIT CODES:
    0 — no invariant violations found
    1 — violation found, unsupported construct, parse error, or file error
"#;

fn print_stateright_help() {
    print!("{STATERIGHT_HELP_TEXT}");
}

pub(crate) fn print_stateright_check_help() {
    print!("{STATERIGHT_CHECK_HELP_TEXT}");
}

pub(crate) fn is_stateright_check_help_request(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some("stateright")
        && args.get(2).map(String::as_str) == Some("check")
        && is_stateright_check_help_args(&args[3..])
}

fn is_stateright_check_help_args(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some("--help" | "-h" | "help")
    )
}

fn check_source(src: &str, filename: &str) -> CheckResult {
    let (program, errs) = crate::parse(src);
    if !errs.is_empty() {
        return CheckResult {
            outcome: CheckOutcome::ParseError,
            diagnostics: errs
                .into_iter()
                .map(|err| format!("{filename}:0:0: error: {err}"))
                .collect(),
        };
    }

    let model = match build_model(&program, filename) {
        Ok(model) => model,
        Err(diag) => {
            return CheckResult {
                outcome: CheckOutcome::Unsupported,
                diagnostics: vec![diag],
            };
        }
    };

    let checker = model
        .clone()
        .checker()
        .target_max_depth(DEFAULT_MAX_DEPTH)
        .spawn_bfs()
        .join();

    if let Some(path) = checker.discovery(model.property_name) {
        let state = path.last_state();
        let failed = first_failed_invariant(&model, state.value)
            .unwrap_or_else(|| "an invariant".to_string());
        return CheckResult {
            outcome: CheckOutcome::Violated,
            diagnostics: vec![format!(
                "{filename}:0:0: error: Stateright found an invariant violation for actor `{}`: {} (state = {})",
                model.actor_name, failed, state.value
            )],
        };
    }

    CheckResult {
        outcome: CheckOutcome::Clean,
        diagnostics: vec![format!(
            "{filename}:0:0: info: Stateright model check completed — no invariant violations found."
        )],
    }
}

fn build_model(program: &Node, filename: &str) -> Result<BridgeModel, String> {
    let Node::Program(stmts) = program else {
        return Err(format!(
            "{filename}:0:0: error: Stateright bridge expected a parsed program."
        ));
    };

    let actors: Vec<&Node> = stmts
        .iter()
        .filter_map(|stmt| match &stmt.node {
            Node::ActorDecl { .. } => Some(&stmt.node),
            _ => None,
        })
        .collect();

    if actors.len() != 1 {
        return Err(format!(
            "{filename}:0:0: error: Stateright bridge currently supports exactly one actor declaration."
        ));
    }

    let Node::ActorDecl {
        name,
        state_fields,
        always_clauses,
        receive_handlers,
        span,
        ..
    } = actors[0]
    else {
        unreachable!();
    };

    if always_clauses.is_empty() {
        return Err(format!(
            "{filename}:{}:{}: error: actor `{}` has no `always:` invariants for Stateright to check.",
            span.start.line, span.start.column, name
        ));
    }

    if state_fields.len() != 1 {
        return Err(format!(
            "{filename}:{}:{}: error: Stateright bridge currently supports exactly one actor state field.",
            span.start.line, span.start.column
        ));
    }

    let (type_name, state_name, init_expr) = &state_fields[0];
    if type_name != "int" {
        let s = node_span(init_expr);
        return Err(format!(
            "{filename}:{}:{}: error: Stateright bridge currently supports only integer actor state.",
            s.start.line, s.start.column
        ));
    }

    validate_expr(init_expr, state_name, true, filename)?;
    let initial_state = eval_int(init_expr, state_name, 0).map_err(|msg| {
        let s = node_span(init_expr);
        format!(
            "{filename}:{}:{}: error: {msg}",
            s.start.line, s.start.column
        )
    })?;

    let mut invariant_labels = Vec::with_capacity(always_clauses.len());
    for inv in always_clauses {
        validate_expr(inv, state_name, true, filename)?;
        invariant_labels.push(render_expr(inv));
    }

    let mut handlers = Vec::with_capacity(receive_handlers.len());
    for handler in receive_handlers {
        for req in &handler.requires {
            validate_expr(req, state_name, false, filename)?;
        }
        let Some(next_state) =
            crate::verifier_actors::straight_line_post_public(&handler.body, state_name)
        else {
            return Err(format!(
                "{filename}:{}:{}: error: Stateright bridge currently supports only straight-line receive handlers.",
                handler.span.start.line, handler.span.start.column
            ));
        };
        validate_expr(&next_state, state_name, false, filename)?;
        handlers.push(BridgeHandler {
            requires: handler.requires.clone(),
            next_state,
        });
    }

    Ok(BridgeModel {
        actor_name: name.clone(),
        state_name: state_name.clone(),
        invariants: always_clauses.clone(),
        invariant_labels,
        handlers,
        initial_state,
        property_name: Box::leak(format!("{}_always", name).into_boxed_str()),
        boundary_abs: DEFAULT_STATE_BOUND,
    })
}

fn first_failed_invariant(model: &BridgeModel, state_value: i64) -> Option<String> {
    model
        .invariants
        .iter()
        .zip(&model.invariant_labels)
        .find_map(|(inv, label)| {
            let ok = eval_bool(inv, &model.state_name, state_value).ok()?;
            if ok { None } else { Some(label.clone()) }
        })
}

fn validate_expr(
    expr: &Node,
    state_name: &str,
    allow_unknown_identifiers: bool,
    filename: &str,
) -> Result<(), String> {
    match expr {
        Node::IntegerLiteral { .. } | Node::BooleanLiteral { .. } => Ok(()),
        Node::Identifier { name, span } => {
            if name == state_name {
                Ok(())
            } else if allow_unknown_identifiers {
                Ok(())
            } else {
                Err(format!(
                    "{filename}:{}:{}: error: Stateright bridge does not support handler parameters or local variables in translated expressions.",
                    span.start.line, span.start.column
                ))
            }
        }
        Node::FieldAccess {
            target,
            field,
            span,
        } => {
            if matches!(target.as_ref(), Node::Identifier { name, .. } if name == "self")
                && field == state_name
            {
                Ok(())
            } else {
                Err(format!(
                    "{filename}:{}:{}: error: Stateright bridge supports only `self.{}` field access.",
                    span.start.line, span.start.column, state_name
                ))
            }
        }
        Node::PrefixExpression {
            operator, right, ..
        } => match *operator {
            "-" | "!" => validate_expr(right, state_name, allow_unknown_identifiers, filename),
            _ => {
                let s = node_span(expr);
                Err(format!(
                    "{filename}:{}:{}: error: unsupported prefix operator `{}` in Stateright bridge.",
                    s.start.line, s.start.column, operator
                ))
            }
        },
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => match *operator {
            "+" | "-" | "*" | "/" | "<" | "<=" | ">" | ">=" | "==" | "!=" | "&&" | "||" => {
                validate_expr(left, state_name, allow_unknown_identifiers, filename)?;
                validate_expr(right, state_name, allow_unknown_identifiers, filename)
            }
            _ => {
                let s = node_span(expr);
                Err(format!(
                    "{filename}:{}:{}: error: unsupported operator `{}` in Stateright bridge.",
                    s.start.line, s.start.column, operator
                ))
            }
        },
        _ => {
            let s = node_span(expr);
            Err(format!(
                "{filename}:{}:{}: error: unsupported expression shape in Stateright bridge.",
                s.start.line, s.start.column
            ))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvalValue {
    Int(i64),
    Bool(bool),
}

fn eval_int(expr: &Node, state_name: &str, state_value: i64) -> Result<i64, String> {
    match eval_expr(expr, state_name, state_value)? {
        EvalValue::Int(v) => Ok(v),
        EvalValue::Bool(_) => Err("expected integer expression in Stateright bridge".to_string()),
    }
}

fn eval_bool(expr: &Node, state_name: &str, state_value: i64) -> Result<bool, String> {
    match eval_expr(expr, state_name, state_value)? {
        EvalValue::Bool(v) => Ok(v),
        EvalValue::Int(_) => Err("expected boolean expression in Stateright bridge".to_string()),
    }
}

fn eval_expr(expr: &Node, state_name: &str, state_value: i64) -> Result<EvalValue, String> {
    match expr {
        Node::IntegerLiteral { value, .. } => Ok(EvalValue::Int(*value)),
        Node::BooleanLiteral { value, .. } => Ok(EvalValue::Bool(*value)),
        Node::Identifier { name, .. } if name == state_name => Ok(EvalValue::Int(state_value)),
        Node::FieldAccess { target, field, .. }
            if matches!(target.as_ref(), Node::Identifier { name, .. } if name == "self")
                && field == state_name =>
        {
            Ok(EvalValue::Int(state_value))
        }
        Node::PrefixExpression {
            operator, right, ..
        } => match (*operator, eval_expr(right, state_name, state_value)?) {
            ("-", EvalValue::Int(v)) => Ok(EvalValue::Int(-v)),
            ("!", EvalValue::Bool(v)) => Ok(EvalValue::Bool(!v)),
            _ => Err("type mismatch in prefix expression".to_string()),
        },
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let left = eval_expr(left, state_name, state_value)?;
            let right = eval_expr(right, state_name, state_value)?;
            eval_infix(*operator, left, right)
        }
        _ => Err("unsupported expression during Stateright evaluation".to_string()),
    }
}

fn eval_infix(operator: &str, left: EvalValue, right: EvalValue) -> Result<EvalValue, String> {
    match (operator, left, right) {
        ("+", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Int(a + b)),
        ("-", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Int(a - b)),
        ("*", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Int(a * b)),
        ("/", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Int(a / b)),
        ("<", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Bool(a < b)),
        ("<=", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Bool(a <= b)),
        (">", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Bool(a > b)),
        (">=", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Bool(a >= b)),
        ("==", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Bool(a == b)),
        ("!=", EvalValue::Int(a), EvalValue::Int(b)) => Ok(EvalValue::Bool(a != b)),
        ("==", EvalValue::Bool(a), EvalValue::Bool(b)) => Ok(EvalValue::Bool(a == b)),
        ("!=", EvalValue::Bool(a), EvalValue::Bool(b)) => Ok(EvalValue::Bool(a != b)),
        ("&&", EvalValue::Bool(a), EvalValue::Bool(b)) => Ok(EvalValue::Bool(a && b)),
        ("||", EvalValue::Bool(a), EvalValue::Bool(b)) => Ok(EvalValue::Bool(a || b)),
        _ => Err(format!("type mismatch for operator `{operator}`")),
    }
}

fn render_expr(expr: &Node) -> String {
    match expr {
        Node::Identifier { name, .. } => name.clone(),
        Node::IntegerLiteral { value, .. } => value.to_string(),
        Node::BooleanLiteral { value, .. } => value.to_string(),
        Node::FieldAccess { target, field, .. } => format!("{}.{}", render_expr(target), field),
        Node::PrefixExpression {
            operator, right, ..
        } => {
            format!("{operator}{}", render_expr(right))
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => format!("{} {} {}", render_expr(left), operator, render_expr(right)),
        _ => format!("{expr:?}"),
    }
}

fn node_span(node: &Node) -> Span {
    match node {
        Node::Identifier { span, .. }
        | Node::IntegerLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::FieldAssignment { span, .. }
        | Node::Assignment { span, .. }
        | Node::ExpressionStatement { span, .. }
        | Node::Block { span, .. }
        | Node::ActorDecl { span, .. } => *span,
        _ => Span::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_stateright_arg_returns_none() {
        let args: Vec<String> = vec!["check".into(), "foo.rz".into()];
        assert!(dispatch_stateright_subcommand(&args).is_none());
    }

    #[test]
    fn stateright_help_returns_zero() {
        let args: Vec<String> = vec!["stateright".into(), "--help".into()];
        assert_eq!(dispatch_stateright_subcommand(&args), Some(0));
    }

    #[test]
    fn stateright_check_missing_file_returns_one() {
        let args: Vec<String> = vec!["stateright".into(), "check".into()];
        assert_eq!(dispatch_stateright_subcommand(&args), Some(1));
    }

    #[test]
    fn stateright_check_help_request_detected() {
        let args = vec![
            "rz".into(),
            "stateright".into(),
            "check".into(),
            "--help".into(),
        ];
        assert!(is_stateright_check_help_request(&args));
    }

    #[test]
    fn bounded_actor_is_clean() {
        let src = r#"
actor Q {
    state: int = 0;
    always: state <= 2;
    receive push() requires state < 2 { self.state = self.state + 1; }
    receive pop() requires state > 0 { self.state = self.state - 1; }
}
"#;
        let result = check_source(src, "bounded.rz");
        assert_eq!(result.outcome, CheckOutcome::Clean);
        assert!(
            result.diagnostics[0].contains("no invariant violations found"),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn unbounded_actor_reports_violation() {
        let src = r#"
actor Q {
    state: int = 0;
    always: state <= 2;
    receive push() { self.state = self.state + 1; }
}
"#;
        let result = check_source(src, "broken.rz");
        assert_eq!(result.outcome, CheckOutcome::Violated);
        assert!(
            result.diagnostics[0].contains("state <= 2"),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn control_flow_handler_is_unsupported() {
        let src = r#"
actor Q {
    state: int = 0;
    always: state <= 2;
    receive push() {
        if state < 2 { self.state = self.state + 1; }
    }
}
"#;
        let result = check_source(src, "unsupported.rz");
        assert_eq!(result.outcome, CheckOutcome::Unsupported);
        assert!(
            result.diagnostics[0].contains("straight-line receive handlers"),
            "{:?}",
            result.diagnostics
        );
    }
}
