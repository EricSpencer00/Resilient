// region_inference.rs
//
// RES-394 PR 1: region-variable machinery + unification table.
// RES-394 PR 2: inference pass — assigns region vars to unlabeled
//               reference parameters and walks the call graph.
#![allow(dead_code)]

use std::collections::HashMap;

// ============================================================
// Region vocabulary
// ============================================================

/// An inference variable assigned to an unlabeled reference parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionVar(pub u32);

/// A region is either a concrete user-declared label or an inference variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Region {
    /// A user-declared region label, e.g. from `region A;`.
    Named(String),
    /// An unresolved inference variable.
    Var(RegionVar),
}

impl Region {
    /// Convenience constructor.
    pub fn named(label: impl Into<String>) -> Self {
        Region::Named(label.into())
    }
}

// ============================================================
// Union-find table
// ============================================================

/// Maps region variables to their canonical `Region` representative.
///
/// Implements a simple union-find (without path compression): each variable
/// either points to another `Region` (its representative) or is free.
pub struct RegionTable {
    next_id: u32,
    parent: HashMap<u32, Region>,
}

impl RegionTable {
    pub fn new() -> Self {
        RegionTable {
            next_id: 0,
            parent: HashMap::new(),
        }
    }

    /// Allocate a fresh region variable.
    pub fn fresh(&mut self) -> RegionVar {
        let id = self.next_id;
        self.next_id += 1;
        RegionVar(id)
    }

    /// Resolve a `Region` to its canonical representative.
    ///
    /// Follows variable chains until a `Region::Named` or an unbound
    /// `Region::Var` is reached.
    pub fn resolve(&self, mut r: Region) -> Region {
        loop {
            match &r {
                Region::Var(v) => match self.parent.get(&v.0) {
                    Some(parent) => r = parent.clone(),
                    None => return r,
                },
                Region::Named(_) => return r,
            }
        }
    }

    /// Unify two regions — constrain them to refer to the same memory area.
    ///
    /// Returns `Err` if both regions resolve to different concrete labels
    /// (i.e. the user labeled them differently and they truly cannot alias).
    pub fn unify(&mut self, a: Region, b: Region) -> Result<(), String> {
        let ra = self.resolve(a);
        let rb = self.resolve(b);

        if ra == rb {
            return Ok(());
        }

        match (ra, rb) {
            // Variable unified with a concrete label or another variable.
            (Region::Var(va), rhs) => {
                self.parent.insert(va.0, rhs);
                Ok(())
            }
            // Concrete label unified with a variable.
            (lhs, Region::Var(vb)) => {
                self.parent.insert(vb.0, lhs);
                Ok(())
            }
            // Two different concrete labels — genuine conflict.
            (Region::Named(a), Region::Named(b)) => Err(format!(
                "region conflict: label `{}` cannot unify with label `{}`",
                a, b
            )),
        }
    }

    /// Return the number of variables allocated so far.
    pub fn var_count(&self) -> u32 {
        self.next_id
    }
}

impl Default for RegionTable {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================
// Region map — per-function parameter→region mapping
// ============================================================

/// Identifies a specific function parameter by function name and
/// zero-based index within the parameter list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParamKey {
    pub fn_name: String,
    pub param_idx: usize,
}

/// Associates each reference parameter with an inferred `Region`.
pub struct RegionMap {
    pub table: RegionTable,
    /// Mapping from `(fn_name, param_idx)` → `Region`.
    pub entries: HashMap<ParamKey, Region>,
}

impl RegionMap {
    fn new() -> Self {
        RegionMap {
            table: RegionTable::new(),
            entries: HashMap::new(),
        }
    }

    /// Look up the region for a parameter, resolving any inference
    /// variable to its canonical representative.
    pub fn get_resolved(&self, key: &ParamKey) -> Option<Region> {
        self.entries.get(key).map(|r| self.table.resolve(r.clone()))
    }
}

// ============================================================
// Inference pass (RES-394 PR 2)
// ============================================================

/// Parse the region label from an encoded parameter type string.
///
/// Replicates the logic in `crate::parse_ref_type` without needing
/// to import it (keeping this module self-contained).
fn region_from_type_str(ty: &str) -> Option<(bool, Option<String>)> {
    let rest = ty.strip_prefix('&')?;
    let (is_mut, rest) = if let Some(r) = rest.strip_prefix("mut") {
        (true, r)
    } else {
        (false, rest)
    };
    let rest = rest.trim_start();
    if let Some(after_bracket) = rest.strip_prefix('[') {
        let close = after_bracket.find(']')?;
        let label = after_bracket[..close].trim().to_string();
        if label.is_empty() {
            return Some((is_mut, None));
        }
        Some((is_mut, Some(label)))
    } else {
        Some((is_mut, None))
    }
}

/// RES-394 PR 2: walk the program AST and build a `RegionMap` by
/// assigning region variables to unlabeled reference parameters.
///
/// Labeled parameters (`&[A] T`) keep their concrete `Region::Named`
/// label; unlabeled ones (`&T` / `&mut T`) receive a fresh `RegionVar`.
pub fn build_region_map(program: &crate::Node) -> RegionMap {
    let mut map = RegionMap::new();
    let stmts = match program {
        crate::Node::Program(s) => s,
        _ => return map,
    };
    for spanned in stmts {
        if let crate::Node::Function {
            name: fn_name,
            parameters,
            ..
        } = &spanned.node
        {
            for (idx, (ty, _pname)) in parameters.iter().enumerate() {
                if let Some((_is_mut, label)) = region_from_type_str(ty) {
                    let region = match label {
                        Some(l) => Region::named(l),
                        None => Region::Var(map.table.fresh()),
                    };
                    map.entries.insert(
                        ParamKey {
                            fn_name: fn_name.clone(),
                            param_idx: idx,
                        },
                        region,
                    );
                }
            }
        }
    }
    map
}

/// EXTENSION_PASSES entry point — runs after type-checking.
///
/// Builds the region map for the program. Currently a no-op with respect
/// to errors; PR D5 will wire the map into `check_region_aliasing` for
/// unlabeled-parameter coverage.
pub fn infer(program: &crate::Node, _source_path: &str) -> Result<(), String> {
    let _map = build_region_map(program);
    Ok(())
}

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_vars_are_distinct() {
        let mut table = RegionTable::new();
        let a = table.fresh();
        let b = table.fresh();
        assert_ne!(a, b);
    }

    #[test]
    fn unbound_var_resolves_to_itself() {
        let mut table = RegionTable::new();
        let v = table.fresh();
        assert_eq!(table.resolve(Region::Var(v)), Region::Var(v));
    }

    #[test]
    fn unify_var_with_named_resolves_to_named() {
        let mut table = RegionTable::new();
        let v = table.fresh();
        table
            .unify(Region::Var(v), Region::named("A"))
            .expect("unify");
        assert_eq!(
            table.resolve(Region::Var(v)),
            Region::Named("A".to_string())
        );
    }

    #[test]
    fn unify_two_vars_chains_to_named() {
        let mut table = RegionTable::new();
        let v1 = table.fresh();
        let v2 = table.fresh();
        table
            .unify(Region::Var(v1), Region::Var(v2))
            .expect("unify v1=v2");
        table
            .unify(Region::Var(v2), Region::named("B"))
            .expect("unify v2=B");
        assert_eq!(
            table.resolve(Region::Var(v1)),
            Region::Named("B".to_string())
        );
    }

    #[test]
    fn unify_two_different_named_regions_errors() {
        let mut table = RegionTable::new();
        let err = table
            .unify(Region::named("X"), Region::named("Y"))
            .unwrap_err();
        assert!(
            err.contains("X") && err.contains("Y"),
            "error should mention both labels: {err}"
        );
    }

    #[test]
    fn unify_same_named_region_is_ok() {
        let mut table = RegionTable::new();
        table
            .unify(Region::named("Z"), Region::named("Z"))
            .expect("same-label unify should succeed");
    }

    #[test]
    fn build_region_map_assigns_vars_to_unlabeled_params() {
        let src = "region A; fn f(&mut[A] int a, &mut int b, int c) {}";
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);

        let map = build_region_map(&program);
        let key_a = ParamKey {
            fn_name: "f".to_string(),
            param_idx: 0,
        };
        let key_b = ParamKey {
            fn_name: "f".to_string(),
            param_idx: 1,
        };
        let key_c = ParamKey {
            fn_name: "f".to_string(),
            param_idx: 2,
        };

        // Labeled param → Named region.
        assert_eq!(
            map.get_resolved(&key_a),
            Some(Region::named("A")),
            "labeled param should resolve to Named"
        );
        // Unlabeled ref param → Var (resolved to itself when unbound).
        assert!(
            matches!(map.get_resolved(&key_b), Some(Region::Var(_))),
            "unlabeled ref param should get a RegionVar"
        );
        // Non-ref param → not in map.
        assert_eq!(map.entries.get(&key_c), None, "non-ref param not in map");
    }
}
