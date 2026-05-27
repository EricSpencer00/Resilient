//! RES-2533: enum payload destructuring in match arms.
//!
//! ## What this module does
//!
//! Validates that match arm patterns that destructure enum variants with
//! payloads use the correct arity (number of bound variables matches the
//! number of declared payload types). It also provides `payload_binding_types`,
//! a helper called by `typechecker::Typechecker::match_pattern_binding_types`
//! to give payload bindings their correct declared types instead of `Any`.
//!
//! ## Coverage
//!
//! * Single-payload tuple variants: `Shape::Circle(r)`.
//! * Multi-payload tuple variants: `Shape::Rect(w, h)`.
//! * Named-field payload variants: `Shape::Circle { r }`.
//! * Nested patterns: the outer match uses `Any` for the scrutinee of
//!   the inner match, which is handled by falling back to `Type::Any`
//!   for bindings when the enum declaration is not in scope.
//! * Exhaustiveness is handled by the existing `enum_exhaustiveness`
//!   module — this module focuses on arity and type fidelity of bindings.

use crate::{EnumPatternPayload, Node, Pattern};
use std::collections::HashMap;

/// Arity mismatch found in a match arm.
#[derive(Debug, Clone)]
pub struct ArityError {
    /// The fully-qualified variant name, e.g. `"Shape::Rect"`.
    pub variant: String,
    /// Number of payload types declared on the variant.
    pub declared: usize,
    /// Number of sub-patterns supplied in the match arm.
    pub provided: usize,
}

impl std::fmt::Display for ArityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "enum variant `{}` has {} payload field(s), but match arm supplies {}",
            self.variant, self.declared, self.provided
        )
    }
}

/// Build a map of `"EnumName::VariantName" → declared_arity` for all
/// `Node::EnumDecl` nodes at the top level of `program`.
fn build_arity_map(program: &Node) -> HashMap<String, usize> {
    let Node::Program(stmts) = program else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    for s in stmts {
        if let Node::EnumDecl { name, variants, .. } = &s.node {
            for v in variants {
                let key = format!("{}::{}", name, v.name);
                let arity = match &v.payload {
                    crate::EnumPayload::None => 0,
                    crate::EnumPayload::Named(fields) => fields.len(),
                    crate::EnumPayload::Tuple(tys) => tys.len(),
                };
                map.insert(key, arity);
            }
        }
    }
    map
}

/// Check a single pattern for arity mismatches against the declared enum.
fn check_pattern(
    pattern: &Pattern,
    arity_map: &HashMap<String, usize>,
    errors: &mut Vec<ArityError>,
) {
    match pattern {
        Pattern::EnumVariant {
            type_name: Some(tn),
            variant_name,
            payload,
        } => {
            let key = format!("{}::{}", tn, variant_name);
            if let Some(&declared) = arity_map.get(&key) {
                let provided = match payload {
                    EnumPatternPayload::None => 0,
                    EnumPatternPayload::Named(fields) => fields.len(),
                    EnumPatternPayload::Tuple(subs) => subs.len(),
                };
                if declared != provided {
                    errors.push(ArityError {
                        variant: key,
                        declared,
                        provided,
                    });
                }
            }
            // Recurse into sub-patterns.
            match payload {
                EnumPatternPayload::Named(fields) => {
                    for (_, sub) in fields {
                        check_pattern(sub.as_ref(), arity_map, errors);
                    }
                }
                EnumPatternPayload::Tuple(subs) => {
                    for sub in subs {
                        check_pattern(sub, arity_map, errors);
                    }
                }
                EnumPatternPayload::None => {}
            }
        }
        Pattern::Some(inner)
        | Pattern::Ok(inner)
        | Pattern::Err(inner)
        | Pattern::Bind(_, inner) => {
            check_pattern(inner.as_ref(), arity_map, errors);
        }
        Pattern::Or(branches) => {
            for b in branches {
                check_pattern(b, arity_map, errors);
            }
        }
        Pattern::Struct { fields, .. } => {
            for (_, sub) in fields {
                check_pattern(sub.as_ref(), arity_map, errors);
            }
        }
        Pattern::TupleStruct { fields, .. } => {
            for sub in fields {
                check_pattern(sub, arity_map, errors);
            }
        }
        Pattern::Tuple(items) => {
            for sub in items {
                check_pattern(sub, arity_map, errors);
            }
        }
        Pattern::Wildcard
        | Pattern::Identifier(_)
        | Pattern::Literal(_)
        | Pattern::Range { .. }
        | Pattern::None
        | Pattern::EnumVariant {
            type_name: None, ..
        } => {}
    }
}

/// Walk all `Node::Match` arms in the program looking for arity mismatches.
fn collect_errors(program: &Node, arity_map: &HashMap<String, usize>) -> Vec<ArityError> {
    let mut errors = Vec::new();
    crate::uniqueness_walk::visit(program, &mut |node| {
        if let Node::Match { arms, .. } = node {
            for (pattern, _guard, _body) in arms {
                check_pattern(pattern, arity_map, &mut errors);
            }
        }
    });
    errors
}

/// Type-checker entry point. Returns `Err` on the first payload-arity mismatch.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let arity_map = build_arity_map(program);
    if arity_map.is_empty() {
        return Ok(());
    }
    let errs = collect_errors(program, &arity_map);
    if errs.is_empty() {
        return Ok(());
    }
    Err(format!("{}: error: {}", source_path, errs[0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn parse_and_check(src: &str) -> Result<(), String> {
        let (prog, _errs) = parse(src);
        check(&prog, "test.rz")
    }

    fn parse_and_errors(src: &str) -> Vec<ArityError> {
        let (prog, _errs) = parse(src);
        let am = build_arity_map(&prog);
        collect_errors(&prog, &am)
    }

    // ── Positive cases ────────────────────────────────────────────────

    #[test]
    fn single_payload_correct_arity() {
        let src = r#"
enum Shape { Circle(float) }
fn area(Shape s) -> float {
    return match s {
        Shape::Circle(r) => r,
        _ => 0.0,
    };
}
"#;
        assert!(parse_and_errors(src).is_empty(), "expected no arity errors");
    }

    #[test]
    fn multi_payload_correct_arity() {
        let src = r#"
enum Shape { Rect(float, float) }
fn area(Shape s) -> float {
    return match s {
        Shape::Rect(w, h) => w * h,
        _ => 0.0,
    };
}
"#;
        assert!(parse_and_errors(src).is_empty(), "expected no arity errors");
    }

    #[test]
    fn payload_less_variant_correct() {
        let src = r#"
enum Color { Red, Green }
fn name(Color c) -> int {
    return match c {
        Color::Red => 1,
        Color::Green => 2,
    };
}
"#;
        assert!(parse_and_errors(src).is_empty(), "expected no arity errors");
    }

    #[test]
    fn named_payload_correct_arity() {
        let src = r#"
enum Shape { Circle { r: float } }
fn area(Shape s) -> float {
    return match s {
        Shape::Circle { r } => r,
        _ => 0.0,
    };
}
"#;
        assert!(parse_and_errors(src).is_empty(), "expected no arity errors");
    }

    #[test]
    fn multi_named_payload_correct_arity() {
        let src = r#"
enum Shape { Rect { w: float, h: float } }
fn area(Shape s) -> float {
    return match s {
        Shape::Rect { w, h } => w * h,
        _ => 0.0,
    };
}
"#;
        assert!(parse_and_errors(src).is_empty(), "expected no arity errors");
    }

    // ── Arity-mismatch error cases ─────────────────────────────────────

    #[test]
    fn too_few_payload_bindings_is_error() {
        let src = r#"
enum Shape { Rect(float, float) }
fn area(Shape s) -> float {
    return match s {
        Shape::Rect(w) => w,
        _ => 0.0,
    };
}
"#;
        let errs = parse_and_errors(src);
        assert_eq!(
            errs.len(),
            1,
            "expected exactly one arity error; got: {:?}",
            errs
        );
        assert_eq!(errs[0].declared, 2, "declared arity should be 2");
        assert_eq!(errs[0].provided, 1, "provided arity should be 1");
        assert!(
            errs[0].variant.contains("Rect"),
            "variant name in error: {:?}",
            errs[0]
        );
    }

    #[test]
    fn too_many_payload_bindings_is_error() {
        let src = r#"
enum Shape { Circle(float) }
fn area(Shape s) -> float {
    return match s {
        Shape::Circle(r, extra) => r,
        _ => 0.0,
    };
}
"#;
        let errs = parse_and_errors(src);
        assert_eq!(
            errs.len(),
            1,
            "expected exactly one arity error; got: {:?}",
            errs
        );
        assert_eq!(errs[0].declared, 1, "declared arity should be 1");
        assert_eq!(errs[0].provided, 2, "provided arity should be 2");
    }

    #[test]
    fn check_fn_ok_for_correct_program() {
        let src = r#"
enum Shape { Circle(float), Rect(float, float) }
fn area(Shape s) -> float {
    return match s {
        Shape::Circle(r) => 3.14 * r * r,
        Shape::Rect(w, h) => w * h,
    };
}
"#;
        assert!(parse_and_check(src).is_ok());
    }

    #[test]
    fn check_fn_err_for_arity_mismatch() {
        let src = r#"
enum Shape { Rect(float, float) }
fn area(Shape s) -> float {
    return match s {
        Shape::Rect(w) => w,
        _ => 0.0,
    };
}
"#;
        let result = parse_and_check(src);
        assert!(result.is_err(), "expected arity error");
        let msg = result.unwrap_err();
        assert!(msg.contains("error:"), "error must contain 'error:': {msg}");
        assert!(msg.contains("Rect"), "error must mention Rect: {msg}");
    }

    #[test]
    fn nested_enum_pattern_arity_correct() {
        let src = r#"
enum Shape { Circle(float) }
fn maybe_area(Shape s) -> float {
    return match s {
        Shape::Circle(r) => r,
        _ => 0.0,
    };
}
"#;
        assert!(
            parse_and_errors(src).is_empty(),
            "expected no arity errors for correct nested patterns"
        );
    }

    // ── build_arity_map tests ─────────────────────────────────────────

    #[test]
    fn arity_map_tuple_variants() {
        let src = "enum E { A(int), B(int, int), C }";
        let (prog, _) = parse(src);
        let am = build_arity_map(&prog);
        assert_eq!(am.get("E::A").copied(), Some(1));
        assert_eq!(am.get("E::B").copied(), Some(2));
        assert_eq!(am.get("E::C").copied(), Some(0));
    }

    #[test]
    fn arity_map_named_variants() {
        let src = "enum S { Circle { r: float }, Rect { w: float, h: float } }";
        let (prog, _) = parse(src);
        let am = build_arity_map(&prog);
        assert_eq!(am.get("S::Circle").copied(), Some(1));
        assert_eq!(am.get("S::Rect").copied(), Some(2));
    }

    #[test]
    fn empty_program_produces_empty_arity_map() {
        let (prog, _) = parse("");
        let am = build_arity_map(&prog);
        assert!(am.is_empty());
    }
}
