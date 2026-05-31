//! RES-2574: generic struct declarations — `struct Name<T> { T field }`.
//!
//! Mirrors the generic enum pattern from `generic_enums.rs`. All
//! generic-struct validation and substitution logic lives here; the
//! parser and typechecker make minimal, targeted calls into this module.
//!
//! ## Surface syntax
//!
//! ```text
//! struct Pair<T, U> {
//!     T first,
//!     U second,
//! }
//!
//! struct Wrapper<T> {
//!     T value,
//!     string label,
//! }
//! ```
//!
//! ## What this module provides
//!
//! - **Validation**: `check` ensures type-parameter names are unique,
//!   don't shadow built-in types, and that every field type is either a
//!   declared type parameter or a plausible concrete type.
//! - **Substitution**: `substitute_fields` rewrites field-type strings
//!   by replacing type-parameter names with their concrete bindings.
//! - **Lookup helpers**: `find_generic_struct`, `is_generic_struct`,
//!   `collect_generic_structs` for the typechecker and interpreter.

#![allow(dead_code)]

use crate::Node;
use crate::generic_enums::{Subst, is_reserved_type_name};
use crate::span::Span;
use std::collections::HashSet;

/// Substitute type parameters in a struct's field list, producing
/// concrete `(type_name, field_name)` pairs.
pub fn substitute_fields(fields: &[(String, String)], subst: &Subst) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(ty, name)| {
            let concrete = match subst.get(ty) {
                Some(replaced) => replaced.clone(),
                None => ty.clone(),
            };
            (concrete, name.clone())
        })
        .collect()
}

/// Validate every generic struct declaration in `program`.
///
/// Checks:
/// 1. Type-parameter names are unique within the struct.
/// 2. Type-parameter names don't shadow built-in types.
/// 3. Every field type is either a type parameter or a plausible
///    concrete type identifier.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    let has_generic_struct = stmts.iter().any(|s| {
        matches!(
            &s.node,
            Node::StructDecl { type_params, .. } if !type_params.is_empty()
        )
    });
    if !has_generic_struct {
        return Ok(());
    }

    for stmt in stmts {
        if let Node::StructDecl {
            name,
            type_params,
            fields,
            span,
            ..
        } = &stmt.node
        {
            if type_params.is_empty() {
                continue;
            }
            validate_generic_struct(name, type_params, fields, *span, source_path)?;
        }
    }
    Ok(())
}

fn validate_generic_struct(
    name: &str,
    type_params: &[String],
    fields: &[(String, String)],
    span: Span,
    source_path: &str,
) -> Result<(), String> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(type_params.len());
    for tp in type_params {
        if !seen.insert(tp.as_str()) {
            return Err(format!(
                "{}:{}:{}: error: duplicate type parameter `{}` in struct `{}`",
                source_path, span.start.line, span.start.column, tp, name
            ));
        }
        if is_reserved_type_name(tp) {
            return Err(format!(
                "{}:{}:{}: error: type parameter `{}` of struct `{}` shadows a built-in type — pick a different name (convention: single uppercase letter, e.g. `T`)",
                source_path, span.start.line, span.start.column, tp, name
            ));
        }
    }

    let tp_set: HashSet<&str> = type_params.iter().map(String::as_str).collect();
    for (ty, field_name) in fields {
        if ty.is_empty() {
            return Err(format!(
                "{}:{}:{}: error: empty type for field `{}` in struct `{}`",
                source_path, span.start.line, span.start.column, field_name, name
            ));
        }
        if tp_set.contains(ty.as_str()) {
            continue;
        }
        if !ty
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_' || c == '[')
            .unwrap_or(false)
        {
            return Err(format!(
                "{}:{}:{}: error: field type `{}` for field `{}` in struct `{}` is not a valid type identifier",
                source_path, span.start.line, span.start.column, ty, field_name, name
            ));
        }
    }
    Ok(())
}

/// True if `name` is the name of a generic struct declared in `program`.
pub fn is_generic_struct(program: &Node, name: &str) -> bool {
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::StructDecl {
                name: sn,
                type_params,
                ..
            } = &s.node
                && sn == name
                && !type_params.is_empty()
            {
                return true;
            }
        }
    }
    false
}

/// Find the generic struct named `name` and return its
/// `(type_params, fields)`. Returns `None` when the name doesn't
/// refer to a generic struct in `program`.
#[allow(clippy::type_complexity)]
pub fn find_generic_struct<'a>(
    program: &'a Node,
    name: &str,
) -> Option<(&'a Vec<String>, &'a Vec<(String, String)>)> {
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::StructDecl {
                name: sn,
                type_params,
                fields,
                ..
            } = &s.node
                && sn == name
                && !type_params.is_empty()
            {
                return Some((type_params, fields));
            }
        }
    }
    None
}

/// Collect every generic struct declaration from a program.
#[allow(clippy::type_complexity)]
pub fn collect_generic_structs(
    program: &Node,
) -> Vec<(&str, &Vec<String>, &Vec<(String, String)>)> {
    let mut out = Vec::new();
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::StructDecl {
                name,
                type_params,
                fields,
                ..
            } = &s.node
                && !type_params.is_empty()
            {
                out.push((name.as_str(), type_params, fields));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic_enums::Subst;

    #[test]
    fn substitute_fields_replaces_type_params() {
        let fields = vec![
            ("T".to_string(), "first".to_string()),
            ("U".to_string(), "second".to_string()),
            ("int".to_string(), "count".to_string()),
        ];
        let subst = Subst::from_pairs(
            &["T".to_string(), "U".to_string()],
            &["int".to_string(), "string".to_string()],
        );
        let result = substitute_fields(&fields, &subst);
        assert_eq!(result[0], ("int".to_string(), "first".to_string()));
        assert_eq!(result[1], ("string".to_string(), "second".to_string()));
        assert_eq!(result[2], ("int".to_string(), "count".to_string()));
    }

    #[test]
    fn substitute_fields_preserves_concrete_types() {
        let fields = vec![
            ("string".to_string(), "name".to_string()),
            ("int".to_string(), "age".to_string()),
        ];
        let subst = Subst::from_pairs(&["T".to_string()], &["float".to_string()]);
        let result = substitute_fields(&fields, &subst);
        assert_eq!(result[0], ("string".to_string(), "name".to_string()));
        assert_eq!(result[1], ("int".to_string(), "age".to_string()));
    }

    #[test]
    fn validate_rejects_duplicate_type_params() {
        let result = validate_generic_struct(
            "Bad",
            &["T".to_string(), "T".to_string()],
            &[("T".to_string(), "x".to_string())],
            Span::default(),
            "test.rz",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("duplicate type parameter"));
    }

    #[test]
    fn validate_rejects_shadowed_builtin() {
        let result = validate_generic_struct(
            "Bad",
            &["int".to_string()],
            &[("int".to_string(), "x".to_string())],
            Span::default(),
            "test.rz",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("shadows a built-in type"));
    }

    #[test]
    fn validate_accepts_valid_generic_struct() {
        let result = validate_generic_struct(
            "Pair",
            &["T".to_string(), "U".to_string()],
            &[
                ("T".to_string(), "first".to_string()),
                ("U".to_string(), "second".to_string()),
            ],
            Span::default(),
            "test.rz",
        );
        assert!(result.is_ok());
    }
}
