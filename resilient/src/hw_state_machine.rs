//! Feature 31/50 — Hardware Lifecycle State Machine Types.
//!
//! `#[peripheral(states = "Reset Configured Suspended", transitions = "Reset:configure->Configured Configured:suspend->Suspended Suspended:resume->Configured")]`
//! attaches a state machine to a peripheral struct. The runtime
//! tracks the current state and rejects calls that are illegal in
//! that state (much like `typestate_types` but with the state
//! tracked by the peripheral instance, not the type alone).

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct PeripheralSpec {
    pub name: String,
    pub states: Vec<String>,
    /// Nested map: `current_state -> (method -> next_state)`.
    ///
    /// RES-2010: previously a flat `HashMap<(String, String), String>`.
    /// The flat shape forced `transition` to allocate two transient
    /// Strings per call (stdlib's `Borrow` impls don't allow looking
    /// up a `(String, String)` key by `(&str, &str)`). Same fix as
    /// RES-2008 for sibling `typestate_types::TypestateSpec`.
    pub transitions: HashMap<String, HashMap<String, String>>,
    pub initial_state: String,
}

static PERIPHERALS: LazyLock<RwLock<HashMap<String, PeripheralSpec>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<PeripheralSpec> {
    let attrs = crate::feature_attrs::find_kind("peripheral");
    // RES-1784: pre-size to attrs.len() — exactly one push per
    // attribute record.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut spec = PeripheralSpec {
            name: item,
            states: Vec::new(),
            transitions: HashMap::new(),
            initial_state: String::new(),
        };
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "states" => {
                        spec.states = v.split_whitespace().map(|s| s.to_string()).collect();
                        if let Some(first) = spec.states.first() {
                            spec.initial_state = first.clone();
                        }
                    }
                    "transitions" => {
                        for t in v.split_whitespace() {
                            if let Some((lhs, next)) = t.split_once("->") {
                                if let Some((s, m)) = lhs.split_once(':') {
                                    // RES-2010: nested map shape — see comment
                                    // on `PeripheralSpec::transitions`.
                                    spec.transitions
                                        .entry(s.to_string())
                                        .or_default()
                                        .insert(m.to_string(), next.to_string());
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

pub fn install(specs: Vec<PeripheralSpec>) {
    if let Ok(mut g) = PERIPHERALS.write() {
        g.clear();
        for s in specs {
            g.insert(s.name.clone(), s);
        }
    }
}

pub fn initial_state(name: &str) -> Option<String> {
    PERIPHERALS
        .read()
        .ok()
        .and_then(|g| g.get(name).map(|s| s.initial_state.clone()))
}

pub fn transition(peripheral: &str, current: &str, method: &str) -> Result<String, String> {
    let g = PERIPHERALS.read().map_err(|_| "lock poisoned")?;
    let spec = g
        .get(peripheral)
        .ok_or_else(|| format!("no peripheral spec for `{peripheral}`"))?;
    // RES-2010: nested-map lookup — `.get(&str)` on each level uses
    // the existing `String: Borrow<str>` impl. Zero per-call
    // allocations (the previous flat `(String, String)` key forced
    // two transient `String::to_string()` allocs per call).
    spec.transitions
        .get(current)
        .and_then(|m| m.get(method))
        .cloned()
        .ok_or_else(|| {
            format!(
                "peripheral `{}` does not allow `{}` from state `{}`",
                peripheral, method, current
            )
        })
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1306: gate `install` on the non-empty case — avoids a
    // wasted RwLock write per compilation and removes the
    // wipe-on-empty test race shape documented in RES-1302.
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
    fn usb_lifecycle_validates() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "USB",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "Reset Configured", transitions = "Reset:configure->Configured""#
                    .into(),
                line: 0,
            },
        );
        install(collect());
        assert_eq!(
            transition("USB", "Reset", "configure").unwrap(),
            "Configured"
        );
        assert!(transition("USB", "Configured", "configure").is_err());
        crate::feature_attrs::reset();
    }
    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn initial_state_returns_none_for_unknown_peripheral() {
        assert!(initial_state("UnknownPeripheral99").is_none());
    }

    // ── Extended malformed-input regression corpus (RES-3754) ────────────────

    #[test]
    fn malformed_missing_states() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "BadPeripheral",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"transitions = "Reset:init->Configured""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        assert!(!specs.is_empty(), "should parse despite missing states");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_missing_transitions() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "BadPeripheral",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "Reset Configured""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        assert!(!specs.is_empty(), "should parse without transitions");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_invalid_transition_syntax() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "BadPeripheral",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "A B", transitions = "invalid_syntax""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        // Should parse but transitions will be empty
        assert!(!specs.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_undefined_target_state() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "BadPeripheral",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "Reset", transitions = "Reset:init->UndefinedState""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        // Should parse - target validation is runtime
        assert!(!specs.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_undefined_source_state() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "BadPeripheral",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "A B", transitions = "UnknownState:op->B""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        // Should parse - source state not validated at collect time
        assert!(!specs.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_empty_states() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "BadPeripheral",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "", transitions = "A:op->B""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        // Empty states - should parse with empty state list
        assert!(!specs.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_duplicate_state_definitions() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "DupStates",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args:
                    r#"states = "Reset Configured Reset", transitions = "Reset:init->Configured""#
                        .into(),
                line: 0,
            },
        );
        let specs = collect();
        // Duplicates in state list - should still parse
        assert!(!specs.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_empty_method_name() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "BadPeripheral",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "A B", transitions = "A:->B""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        // Empty method name - parsing should handle gracefully
        assert!(!specs.is_empty());
        crate::feature_attrs::reset();
    }

    // Valid baseline cases

    #[test]
    fn valid_multiple_peripherals() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "GPIO",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "Input Output", transitions = "Input:set_output->Output""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "Timer",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "Stopped Running", transitions = "Stopped:start->Running Running:stop->Stopped""#.into(),
                line: 1,
            },
        );
        let specs = collect();
        assert_eq!(specs.len(), 2, "should collect multiple peripherals");
        crate::feature_attrs::reset();
    }

    #[test]
    fn valid_complex_transitions() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "UART",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "Reset Initialized Transmitting Receiving", transitions = "Reset:init->Initialized Initialized:send->Transmitting Initialized:receive->Receiving Transmitting:done->Initialized Receiving:done->Initialized""#.into(),
                line: 0,
            },
        );
        install(collect());
        assert_eq!(initial_state("UART"), Some("Reset".to_string()));
        assert_eq!(transition("UART", "Reset", "init").unwrap(), "Initialized");
        assert_eq!(
            transition("UART", "Initialized", "send").unwrap(),
            "Transmitting"
        );
        assert!(transition("UART", "Reset", "send").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn valid_state_reachability() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Device",
            crate::feature_attrs::AttrRecord {
                name: "peripheral".into(),
                args: r#"states = "Off Booting Ready Sleeping", transitions = "Off:power_on->Booting Booting:ready->Ready Ready:sleep->Sleeping Sleeping:wake->Ready""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        install(specs);
        assert_eq!(initial_state("Device"), Some("Off".to_string()));
        let reachable = vec![
            ("Off", "power_on", "Booting"),
            ("Booting", "ready", "Ready"),
            ("Ready", "sleep", "Sleeping"),
            ("Sleeping", "wake", "Ready"),
        ];
        for (from, method, to) in reachable {
            let result = transition("Device", from, method).unwrap();
            assert_eq!(result, to);
        }
        crate::feature_attrs::reset();
    }
}
