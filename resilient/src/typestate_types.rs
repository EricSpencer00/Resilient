//! Feature 13/50 — Temporal Type States.
//!
//! `#[typestate(states = "Closed Open Flushed", transitions = "Closed:open->Open Open:flush->Flushed Open:close->Closed Flushed:close->Closed")]`
//! attached to a struct turns it into a typestate type: the value's
//! state evolves across method calls, and calls that violate the
//! state machine are rejected.
//!
//! This is unlike session types (which are about channel protocols) —
//! typestates apply to any stateful object: file handles, MMIO
//! peripherals, lock guards, parser cursors. The effect is to make
//! "use after close" a compile error rather than a runtime panic.
//!
//! This module ships:
//!
//! * The attribute parser into a `TypestateSpec` struct.
//! * A `validate_call(struct_name, current_state, method)` API that
//!   returns `Ok(next_state)` or `Err`. Wired into the runtime by
//!   downstream PR; tests exercise it directly today.

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

fn typestate_diag(source_path: &str, line: usize, message: impl AsRef<str>) -> String {
    format!(
        "{source_path}:{line}:0: error[typestate]: {}",
        message.as_ref()
    )
}

fn validate_typestate_record(
    source_path: &str,
    struct_name: &str,
    rec: &crate::feature_attrs::AttrRecord,
) -> Result<(), String> {
    let mut states: Option<HashSet<String>> = None;
    let mut seen_transitions = HashSet::new();
    let mut saw_transitions = false;

    for chunk in rec.args.split(',') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }

        let Some((k, v)) = chunk.split_once('=') else {
            return Err(typestate_diag(
                source_path,
                rec.line,
                format!("typestate `{struct_name}` expects `key = value` entries, got `{chunk}`"),
            ));
        };

        let key = k.trim();
        let value = v.trim().trim_matches('"');
        match key {
            "states" => {
                if states.is_some() {
                    return Err(typestate_diag(
                        source_path,
                        rec.line,
                        format!("typestate `{struct_name}` declares `states` more than once"),
                    ));
                }

                let mut parsed = HashSet::new();
                for state in value.split_whitespace() {
                    if !parsed.insert(state.to_string()) {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!(
                                "typestate `{struct_name}` repeats state `{state}` in `states`"
                            ),
                        ));
                    }
                }

                if parsed.is_empty() {
                    return Err(typestate_diag(
                        source_path,
                        rec.line,
                        format!("typestate `{struct_name}` must declare at least one state"),
                    ));
                }

                states = Some(parsed);
            }
            "transitions" => {
                let Some(states) = states.as_ref() else {
                    return Err(typestate_diag(
                        source_path,
                        rec.line,
                        format!(
                            "typestate `{struct_name}` must declare `states` before `transitions`"
                        ),
                    ));
                };

                saw_transitions = true;
                for transition in value.split_whitespace() {
                    let Some((lhs, next)) = transition.split_once("->") else {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!(
                                "typestate `{struct_name}` transition `{transition}` must use `state:method->next`"
                            ),
                        ));
                    };

                    let Some((state, method)) = lhs.split_once(':') else {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!(
                                "typestate `{struct_name}` transition `{transition}` must use `state:method->next`"
                            ),
                        ));
                    };

                    if state.trim().is_empty() || method.trim().is_empty() || next.trim().is_empty()
                    {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!(
                                "typestate `{struct_name}` transition `{transition}` must name a state, method, and next state"
                            ),
                        ));
                    }

                    if !states.contains(state) {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!(
                                "typestate `{struct_name}` transition `{transition}` starts from unknown state `{state}`"
                            ),
                        ));
                    }

                    if !states.contains(next) {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!(
                                "typestate `{struct_name}` transition `{transition}` targets unknown state `{next}`"
                            ),
                        ));
                    }

                    let transition_key = (state.to_string(), method.to_string());
                    if !seen_transitions.insert(transition_key) {
                        return Err(typestate_diag(
                            source_path,
                            rec.line,
                            format!(
                                "typestate `{struct_name}` repeats transition `{state}:{method}`"
                            ),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    if states.is_none() {
        return Err(typestate_diag(
            source_path,
            rec.line,
            format!("typestate `{struct_name}` is missing a `states` declaration"),
        ));
    }

    if !saw_transitions {
        return Err(typestate_diag(
            source_path,
            rec.line,
            format!("typestate `{struct_name}` is missing a `transitions` declaration"),
        ));
    }

    Ok(())
}

pub fn collect() -> Vec<TypestateSpec> {
    let attrs = crate::feature_attrs::find_kind("typestate");
    // RES-1756: pre-size to attrs.len() — exactly one push per
    // attribute record. Same shape as RES-1754.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut spec = TypestateSpec {
            struct_name: item,
            states: Vec::new(),
            transitions: HashMap::new(),
        };
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "states" => {
                        spec.states = v.split_whitespace().map(|s| s.to_string()).collect();
                    }
                    "transitions" => {
                        for t in v.split_whitespace() {
                            // Format: state:method->next
                            if let Some((lhs, next)) = t.split_once("->") {
                                if let Some((state, method)) = lhs.split_once(':') {
                                    spec.transitions.insert(
                                        (state.to_string(), method.to_string()),
                                        next.to_string(),
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        out.push(spec);
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
    // RES-1549: hold the read guard for the lookup so the
    // `Vec<TypestateSpec>` (each entry owns Vec<String> + HashMap)
    // doesn't get cloned just to find one spec by name. The
    // transition value's String is still cloned (cheap, small)
    // since it's the function's return value.
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

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1306: gate `install` on the non-empty case — avoids a
    // wasted RwLock write per compilation and removes the
    // wipe-on-empty test race shape documented in RES-1302.
    let attrs = crate::feature_attrs::find_kind("typestate");
    let mut seen_specs = HashSet::new();
    for (item, rec) in &attrs {
        if !seen_specs.insert(item) {
            return Err(typestate_diag(
                _source_path,
                rec.line,
                format!("typestate `{item}` is declared more than once"),
            ));
        }
        validate_typestate_record(_source_path, item, rec)?;
    }

    let specs = collect();
    if specs.is_empty() {
        return Ok(());
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
            "error must name the unknown struct: {msg}"
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

    fn assert_check_err(args: &str, expected: &str) {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "File",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: args.into(),
                line: 0,
            },
        );
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "test").unwrap_err();
        assert!(
            err.contains(expected),
            "expected `{expected}` in diagnostic, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn empty_states_decl_is_rejected() {
        assert_check_err(
            r#"states = "", transitions = "Closed:open->Open""#,
            "must declare at least one state",
        );
    }

    #[test]
    fn duplicate_states_are_rejected() {
        assert_check_err(
            r#"states = "Closed Open Closed", transitions = "Closed:open->Open""#,
            "repeats state `Closed`",
        );
    }

    #[test]
    fn missing_transitions_decl_is_rejected() {
        assert_check_err(
            r#"states = "Closed Open""#,
            "missing a `transitions` declaration",
        );
    }

    #[test]
    fn malformed_transition_syntax_is_rejected() {
        assert_check_err(
            r#"states = "Closed Open", transitions = "Closed-open-Open""#,
            "must use `state:method->next`",
        );
    }

    #[test]
    fn unknown_state_in_transition_is_rejected() {
        assert_check_err(
            r#"states = "Closed Open", transitions = "Closed:open->Open Open:close->Sealed""#,
            "targets unknown state `Sealed`",
        );
    }

    #[test]
    fn duplicate_transition_is_rejected() {
        assert_check_err(
            r#"states = "Closed Open", transitions = "Closed:open->Open Closed:open->Closed""#,
            "repeats transition `Closed:open`",
        );
    }

    #[test]
    fn duplicate_typestate_attribute_is_rejected() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "File",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Closed Open", transitions = "Closed:open->Open""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "File",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Closed Open", transitions = "Open:close->Closed""#.into(),
                line: 1,
            },
        );
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "test").unwrap_err();
        assert!(
            err.contains("declared more than once"),
            "expected duplicate-attribute diagnostic, got: {err}"
        );
        crate::feature_attrs::reset();
    }
}
