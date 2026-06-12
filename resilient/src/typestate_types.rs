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

pub(crate) fn check(_program: &Node, source_path: &str) -> Result<(), String> {
    let specs = collect_checked(source_path)?;
    if specs.is_empty() {
        return Ok(());
    }
    install(specs);
    Ok(())
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
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_rejects_missing_states_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_typestate("Thing", r#"transitions = "S0:go->S1""#, 11);
        let err = check(&crate::parse("fn main() {}\n").0, "test").expect_err("expected error");
        assert!(err.contains("test:11:0: error:"), "{err}");
        assert!(err.contains("missing `states`"), "{err}");
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
        crate::feature_attrs::reset();
    }
}
