//! Ralph-Loop Uniqueness #17 — static IO-byte budget per function.
//!
//! Embedded radios (LoRa, BLE, NB-IoT) have throughput budgets measured
//! in bytes per minute or per duty cycle. Production-grade IoT firmware
//! enforces this at runtime via counters; no language source-level
//! mechanism declares "this fn writes at most N bytes."
//!
//! Resilient encodes the budget by name suffix `_iobytes<N>` (powers of
//! two: 16, 32, 64, 128, 256, 512, 1024). The check sums the byte
//! literals at every IO call site (`net_send`, `radio_tx`, `serial_tx`,
//! `file_write_chunk`) by inspecting integer-literal arguments and
//! string-literal lengths. Calls without literal sizes count as 0
//! (conservative). Going over the static budget warns.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{for_each_function, visit};

const IO_FNS: &[&str] = &[
    "net_send",
    "radio_tx",
    "serial_tx",
    "file_write_chunk",
    "uart_write",
    "i2c_tx",
    "spi_tx",
];
const BUDGETS: &[(usize, &str)] = &[
    (16, "_iobytes16"),
    (32, "_iobytes32"),
    (64, "_iobytes64"),
    (128, "_iobytes128"),
    (256, "_iobytes256"),
    (512, "_iobytes512"),
    (1024, "_iobytes1024"),
];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1222: fast-reject — see stack_budget for the same pattern.
    // Skip the closure dispatch for programs that declare no
    // `_iobytes{N}` suffix.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let has_budget = stmts.iter().any(|s| {
        matches!(&s.node, Node::Function { name, .. }
            if BUDGETS.iter().any(|(_, suf)| name.ends_with(*suf)))
    });
    if !has_budget {
        return Ok(());
    }
    for_each_function(program, |fname, _params, body| {
        let Some(budget) = BUDGETS
            .iter()
            .find(|(_, s)| fname.ends_with(*s))
            .map(|(b, _)| *b)
        else {
            return;
        };
        let est = estimate_io_bytes(body);
        if est > budget {
            eprintln!(
                "warning: '{fname}' declares IO byte budget {budget} \
                 (by name suffix) but the body's literal-byte estimate is \
                 {est} — over the radio/UART duty cycle"
            );
        }
    });
    Ok(())
}

fn estimate_io_bytes(body: &Node) -> usize {
    let mut total = 0usize;
    visit(body, &mut |n| {
        if let Node::CallExpression {
            function,
            arguments,
            ..
        } = n
        {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if IO_FNS.contains(&name.as_str()) {
                    for a in arguments {
                        total += literal_size(a);
                    }
                }
            }
        }
    });
    total
}

fn literal_size(node: &Node) -> usize {
    match node {
        Node::IntegerLiteral { value, .. } if *value >= 0 => *value as usize,
        Node::StringLiteral { value, .. } => value.len(),
        _ => 0,
    }
}
