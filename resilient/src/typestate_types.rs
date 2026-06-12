//! Feature 13/50 - Temporal Type States.
//!
//! `#[typestate(states = "Closed Open Flushed", transitions = "Closed:open->Open Open:flush->Flushed Open:close->Closed Flushed:close->Closed")]`
//! attached to a struct turns it into a typestate type: a value's
//! state evolves across method calls, and invalid calls are rejected.
//!
//! This is unlike session types (which are about channel protocols) -
//! typestates apply to any stateful object: file handles, MMIO
//! peripherals, lock guards, parser cursors. The effect is to make
//! "use after close" a compile-time error instead of a runtime panic.
//!
//! This module ships:
//!
//! * attribute parser into `TypestateSpec`
//! * a `validate_call(struct_name, current_state, method)` API that
//!   returns `Ok(next_state)` or `Err`

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

fn typestate_diag(source_path: &str, line: usize, msg: impl AsRef<str>) -> String {
    let (line, col) = if line == 0 { (0, 0) } else { (line, 1) };
    format!(
        "{source_path}:{line}:{col}: error[typestate]: {}",
        msg.as_ref()
    )
}

fn parse_typestate_record(
    source_path: &str,
    item: String,
    rec: &crate::feature_attrs::AttrRecord,
) -> Result<TypestateSpec, String> {
    let mut spec = TypestateSpec {
        struct_name: item,
        states: Vec::new(),
        transitions: HashMap::new(),
    };
    let mut seen_states = false;
    let mut seen_transitions = false;
    let mut pending_transitions = Vec::new();

    for raw_field in rec.args.split(',') {
        let field = raw_field.trim();
        if field.is_empty() {
            return Err(typestate_diag(
                source_path,
                rec.line,
                "empty typestate argument",
            ));
        }

        let Some((raw_key, raw_value)) = field.split_once('=') else {
            return Err(typestate_diag(
                source_path,
                rec.line,
                format!("malformed typestate argument `{field}`"),
            ));
        };

        let key = raw_key.trim();
        let value = raw_value.trim();
        if !(value.starts_with('"') && value.ends_with('"') && value.len() >= 2) {
            return Err(typestate_diag(
                source_path,
                rec.line,
                format!("typestate argument `{key}` must use a quoted string"),
            ));
        }
        let value = &value[1..value.len() - 1];

        match key {
            "states" => {
                if seen_states {
                    return Err(typestate_diag(
                        source_path,
                        rec.line,
                        "duplicate `states` argument",
                    ));
                }
                seen_states = true;

                let mut state_names = Vec::new();
                let mut seen_state_names = HashSet::new();
                for state in value.split_whitespace() {
                    if !seen_state_names.insert(state) {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!("duplicate state `{state}` in `states`"),
                        ));
                    }
                    state_names.push(state.to_string());
                }

                if state_names.is_empty() {
                    return Err(typestate_diag(
                        source_path,
                        rec.line,
                        "`states` must name at least one state",
                    ));
                }
                spec.states = state_names;
            }
            "transitions" => {
                if seen_transitions {
                    return Err(typestate_diag(
                        source_path,
                        rec.line,
                        "duplicate `transitions` argument",
                    ));
                }
                seen_transitions = true;

                if value.split_whitespace().next().is_none() {
                    return Err(typestate_diag(
                        source_path,
                        rec.line,
                        "`transitions` must name at least one transition",
                    ));
                }

                for transition in value.split_whitespace() {
                    let Some((lhs, next)) = transition.split_once("->") else {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!("malformed transition `{transition}`"),
                        ));
                    };
                    let Some((state, method)) = lhs.split_once(':') else {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!("malformed transition `{transition}`"),
                        ));
                    };

                    let state = state.trim();
                    let method = method.trim();
                    let next = next.trim();
                    if state.is_empty() || method.is_empty() || next.is_empty() {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!("malformed transition `{transition}`"),
                        ));
                    }
                    pending_transitions.push((
                        state.to_string(),
                        method.to_string(),
                        next.to_string(),
                    ));
                }
            }
            _ => {
                return Err(typestate_diag(
                    source_path,
                    rec.line,
                    format!("unknown typestate argument `{key}`"),
                ));
            }
        }
    }

    if !seen_states {
        return Err(typestate_diag(
            source_path,
            rec.line,
            "missing required `states` argument",
        ));
    }
    if !seen_transitions {
        return Err(typestate_diag(
            source_path,
            rec.line,
            "missing required `transitions` argument",
        ));
    }

    let known_states: HashSet<String> = spec.states.iter().cloned().collect();
    let mut seen_transition_forms = HashSet::new();
    for (state, method, next) in pending_transitions {
        if !known_states.contains(&state) {
            return Err(typestate_diag(
                source_path,
                rec.line,
                format!("undeclared state `{state}` in transition"),
            ));
        }
        if !known_states.contains(&next) {
            return Err(typestate_diag(
                source_path,
                rec.line,
                format!("undeclared state `{next}` in transition"),
            ));
        }
        if !seen_transition_forms.insert((state.clone(), method.clone())) {
            return Err(typestate_diag(
                source_path,
                rec.line,
                format!("duplicate transition `{state}:{method}`"),
            ));
        }
        spec.transitions.insert((state, method), next);
    }

    Ok(spec)
}

pub fn collect() -> Vec<TypestateSpec> {
    let attrs = crate::feature_attrs::find_kind("typestate");
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        if let Ok(spec) = parse_typestate_record("<collect>", item, &rec) {
            out.push(spec);
        }
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
    // RES-1549: hold the read guard during lookup so the
    // `Vec<TypestateSpec>` (each entry owns Vec<String> + HashMap)
    // does not get cloned just to find one spec by name. The
    // transition value's String still gets cloned (cheap, small)
    // since it is the function's return value.
    let g = SPECS
        .read()
        .map_err(|_| format!("no typestate {struct_name}"))?;
    let spec = g
        .iter()
        .find(|s| s.struct_name == struct_name)
        .ok_or_else(|| format!("no typestate {struct_name}"))?;
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

pub(crate) fn check(_program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = crate::feature_attrs::find_kind("typestate");
    if attrs.is_empty() {
        return Ok(());
    }

    let mut specs = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let spec = parse_typestate_record(source_path, item, &rec)?;
        specs.push(spec);
    }

    install(specs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_protocol_validates_close_after_open() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "File",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Closed Open", transitions = "Closed:open->Open Open:close->Closed""#.into(),
                line: 0,
            },
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
        crate::feature_attrs::record(
            "Lock",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Locked Unlocked", transitions = "Locked:unlock->Unlocked Unlocked:lock->Locked""#.into(),
                line: 0,
            },
        );
        install(collect());
        // Valid transitions
        assert_eq!(
            validate_call("Lock", "Locked", "unlock").unwrap(),
            "Unlocked"
        );
        assert_eq!(validate_call("Lock", "Unlocked", "lock").unwrap(), "Locked");
        // Invalid: calling `lock` on already-locked is undefined
        assert!(validate_call("Lock", "Locked", "lock").is_err());
        // Invalid: calling `unlock` on already-unlocked is undefined
        assert!(validate_call("Lock", "Unlocked", "unlock").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn three_state_machine_full_cycle() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Connection",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Idle Active Closed", transitions = "Idle:connect->Active Active:send->Active Active:disconnect->Closed""#.into(),
                line: 0,
            },
        );
        install(collect());
        // Walk the full state machine path
        let s1 = validate_call("Connection", "Idle", "connect").unwrap();
        assert_eq!(s1, "Active");
        let s2 = validate_call("Connection", "Active", "send").unwrap();
        assert_eq!(s2, "Active");
        let s3 = validate_call("Connection", "Active", "disconnect").unwrap();
        assert_eq!(s3, "Closed");
        // Can't connect again after closing
        assert!(validate_call("Connection", "Closed", "connect").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn unknown_struct_returns_error() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
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

    fn assert_check_err(item: &str, args: &str, line: usize, expected: &str) {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            item,
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: args.into(),
                line,
            },
        );

        let err = check(&crate::Node::Program(vec![]), "typestate.rz")
            .expect_err("malformed typestate attrs must fail check");
        let prefix = format!("typestate.rz:{line}:1: error[typestate]:");
        assert!(
            err.contains(&prefix),
            "diagnostic missing exact prefix `{prefix}`: {err}"
        );
        assert!(
            err.contains(expected),
            "diagnostic missing `{expected}`: {err}"
        );
        crate::feature_attrs::reset();
    }

    fn assert_check_ok(records: &[(&str, &str)]) {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        for (item, args) in records {
            crate::feature_attrs::record(
                item,
                crate::feature_attrs::AttrRecord {
                    name: "typestate".into(),
                    args: (*args).into(),
                    line: 11,
                },
            );
        }

        assert!(check(&crate::Node::Program(vec![]), "typestate.rz").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn missing_states_argument_is_rejected() {
        assert_check_err(
            "File",
            r#"transitions = "Closed:open->Open""#,
            7,
            "missing required `states` argument",
        );
    }

    #[test]
    fn duplicate_states_argument_is_rejected() {
        assert_check_err(
            "Lock",
            r#"states = "Locked Unlocked", states = "Open Closed", transitions = "Locked:unlock->Unlocked""#,
            8,
            "duplicate `states` argument",
        );
    }

    #[test]
    fn duplicate_transitions_argument_is_rejected() {
        assert_check_err(
            "Door",
            r#"states = "Open Closed", transitions = "Open:close->Closed", transitions = "Closed:open->Open""#,
            9,
            "duplicate `transitions` argument",
        );
    }

    #[test]
    fn unknown_argument_is_rejected() {
        assert_check_err(
            "Port",
            r#"states = "Up Down", mode = "fast", transitions = "Up:down->Down""#,
            10,
            "unknown typestate argument `mode`",
        );
    }

    #[test]
    fn malformed_transition_is_rejected() {
        assert_check_err(
            "Valve",
            r#"states = "Open Closed", transitions = "OpencloseClosed""#,
            11,
            "malformed transition `OpencloseClosed`",
        );
    }

    #[test]
    fn undeclared_transition_state_is_rejected() {
        assert_check_err(
            "Pump",
            r#"states = "Idle Active", transitions = "Idle:start->Running""#,
            12,
            "undeclared state `Running`",
        );
    }

    #[test]
    fn valid_single_transition_passes_check() {
        assert_check_ok(&[(
            "File",
            r#"states = "Closed Open", transitions = "Closed:open->Open Open:close->Closed""#,
        )]);
    }

    #[test]
    fn valid_three_state_cycle_passes_check() {
        assert_check_ok(&[(
            "Connection",
            r#"states = "Idle Active Closed", transitions = "Idle:connect->Active Active:send->Active Active:disconnect->Closed""#,
        )]);
    }

    #[test]
    fn valid_loopback_machine_passes_check() {
        assert_check_ok(&[(
            "Latch",
            r#"states = "Armed Disarmed", transitions = "Armed:disarm->Disarmed Disarmed:arm->Armed""#,
        )]);
    }
}
