//! Feature 27/50 — Typed MMIO Register Maps.
//!
//! `#[mmio(base = "0x40010C14", size_bytes = "0x400")]` on a struct
//! turns it into a typed memory-mapped peripheral. Field accesses
//! lower to volatile reads/writes at compile-determined offsets.
//!
//! Field-level metadata (bit ranges, R/W/RO permissions) lives on
//! each field via `#[bits(0..=15), rw]`; this module focuses on the
//! struct-level base address registry.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct MmioRegmap {
    pub struct_name: String,
    pub base_addr: u64,
    pub size_bytes: u64,
}

static REGMAPS: LazyLock<RwLock<HashMap<String, MmioRegmap>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn parse_addr(s: &str) -> Option<u64> {
    let s = s.trim().trim_matches('"');
    if let Some(rest) = s.strip_prefix("0x") {
        u64::from_str_radix(rest, 16).ok()
    } else {
        s.parse().ok()
    }
}

pub fn collect() -> Vec<MmioRegmap> {
    let attrs = crate::feature_attrs::find_kind("mmio");
    let mut out = Vec::new();
    for (item, rec) in attrs {
        let mut base = 0;
        let mut size = 0;
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                match k {
                    "base" => {
                        if let Some(b) = parse_addr(v) {
                            base = b;
                        }
                    }
                    "size_bytes" => {
                        if let Some(b) = parse_addr(v) {
                            size = b;
                        }
                    }
                    _ => {}
                }
            }
        }
        out.push(MmioRegmap {
            struct_name: item,
            base_addr: base,
            size_bytes: size,
        });
    }
    out
}

pub fn install(maps: Vec<MmioRegmap>) {
    if let Ok(mut g) = REGMAPS.write() {
        g.clear();
        for m in maps {
            g.insert(m.struct_name.clone(), m);
        }
    }
}

pub fn lookup(struct_name: &str) -> Option<MmioRegmap> {
    REGMAPS
        .read()
        .ok()
        .and_then(|g| g.get(struct_name).cloned())
}

pub(crate) fn check(_program: &Node, source_path: &str) -> Result<(), String> {
    // RES-1306: gate `install` (and the overlap loop, which is
    // already a no-op for empty maps) on the non-empty case —
    // avoids a wasted RwLock write per compilation and removes the
    // wipe-on-empty test race shape documented in RES-1302.
    let maps = collect();
    if maps.is_empty() {
        return Ok(());
    }
    install(maps.clone());
    // Detect overlapping address ranges — a real bug.
    for (i, a) in maps.iter().enumerate() {
        for b in &maps[i + 1..] {
            let a_end = a.base_addr.saturating_add(a.size_bytes);
            let b_end = b.base_addr.saturating_add(b.size_bytes);
            let overlap = a.base_addr < b_end && b.base_addr < a_end;
            if overlap {
                return Err(format!(
                    "{}:0:0: error: MMIO regmaps `{}` and `{}` overlap in address space",
                    source_path, a.struct_name, b.struct_name
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_base() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "GPIOA",
            crate::feature_attrs::AttrRecord {
                name: "mmio".into(),
                args: r#"base = "0x40010800", size_bytes = "0x400""#.into(),
                line: 0,
            },
        );
        let m = collect();
        assert_eq!(m[0].base_addr, 0x40010800);
        assert_eq!(m[0].size_bytes, 0x400);
        crate::feature_attrs::reset();
    }

    #[test]
    fn overlapping_maps_error() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "A",
            crate::feature_attrs::AttrRecord {
                name: "mmio".into(),
                args: r#"base = "0x100", size_bytes = "0x100""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "B",
            crate::feature_attrs::AttrRecord {
                name: "mmio".into(),
                args: r#"base = "0x180", size_bytes = "0x100""#.into(),
                line: 0,
            },
        );
        let res = check(&crate::Node::Program(vec![]), "test");
        assert!(res.is_err());
        crate::feature_attrs::reset();
    }
}
