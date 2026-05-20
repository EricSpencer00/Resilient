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

/// RES-2200: dropped the redundant `struct_name: String` field. Two
/// readers — `install`'s key clone and the overlap-check error
/// format — both used it strictly as a name tied to the registry
/// entry. The field stored exactly what the registry HashMap key
/// encoded. Pipeline now carries `(String, MmioRegmap)` tuples from
/// `collect()` to `install()`; the overlap check walks the tuple
/// vec and reads the name from the tuple. Same dead-field pattern
/// as RES-2106 / … / RES-2198.
#[derive(Debug, Clone, Copy)]
pub struct MmioRegmap {
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

pub fn collect() -> Vec<(String, MmioRegmap)> {
    let attrs = crate::feature_attrs::find_kind("mmio");
    // RES-1764: pre-size to attrs.len() — conditional push (only when
    // both base and size parse non-zero), upper bound.
    let mut out = Vec::with_capacity(attrs.len());
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
        out.push((
            item,
            MmioRegmap {
                base_addr: base,
                size_bytes: size,
            },
        ));
    }
    out
}

pub fn install(maps: Vec<(String, MmioRegmap)>) {
    if let Ok(mut g) = REGMAPS.write() {
        g.clear();
        // RES-2200: move (name, map) pairs straight from `collect()`
        // into the registry. The previous shape per-map cloned
        // `m.struct_name` to produce the key, since the field and
        // the key encoded the same string.
        g.extend(maps);
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
    // RES-1491: validate (overlap check) before `install` so `maps`
    // moves in instead of cloning. Same shape as RES-1481 (derives)
    // / RES-1485 (recursive_types) / RES-1487 (ghost+async). The
    // overlap check returns Err on real bug — install never runs on
    // failure, which is the right behavior (don't pollute the
    // registry with overlapping maps).
    for (i, (a_name, a)) in maps.iter().enumerate() {
        for (b_name, b) in &maps[i + 1..] {
            let a_end = a.base_addr.saturating_add(a.size_bytes);
            let b_end = b.base_addr.saturating_add(b.size_bytes);
            let overlap = a.base_addr < b_end && b.base_addr < a_end;
            if overlap {
                return Err(format!(
                    "{}:0:0: error: MMIO regmaps `{}` and `{}` overlap in address space",
                    source_path, a_name, b_name
                ));
            }
        }
    }
    install(maps);
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
        assert_eq!(m[0].0, "GPIOA");
        assert_eq!(m[0].1.base_addr, 0x40010800);
        assert_eq!(m[0].1.size_bytes, 0x400);
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
