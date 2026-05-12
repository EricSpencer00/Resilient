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
    pub transitions: HashMap<(String, String), String>,
    pub initial_state: String,
}

static PERIPHERALS: LazyLock<RwLock<HashMap<String, PeripheralSpec>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<PeripheralSpec> {
    let attrs = crate::feature_attrs::find_kind("peripheral");
    let mut out = Vec::new();
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
                                    spec.transitions
                                        .insert((s.to_string(), m.to_string()), next.to_string());
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
    spec.transitions
        .get(&(current.to_string(), method.to_string()))
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
}
