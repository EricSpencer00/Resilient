use std::collections::HashMap;

/// RES-777: actor boundaries must stay ownership-by-value. This helper
/// checks raw type names (including aliases and nested struct fields)
/// for any reference-shaped component that would smuggle aliasing
/// through actor state or mailbox payloads.
pub(crate) fn type_name_contains_reference(
    type_name: &str,
    type_aliases: &HashMap<String, String>,
    struct_field_decls: &HashMap<String, Vec<(String, String)>>,
    visiting: &mut Vec<String>,
) -> bool {
    let base = crate::linear::strip_linear(type_name).trim();
    if base.starts_with('&') {
        return true;
    }

    if visiting.iter().any(|name| name == base) {
        return false;
    }

    if let Some(target) = type_aliases.get(base) {
        visiting.push(base.to_string());
        let contains =
            type_name_contains_reference(target, type_aliases, struct_field_decls, visiting);
        visiting.pop();
        return contains;
    }

    if let Some(fields) = struct_field_decls.get(base) {
        visiting.push(base.to_string());
        let contains = fields.iter().any(|(field_ty, _field_name)| {
            type_name_contains_reference(field_ty, type_aliases, struct_field_decls, visiting)
        });
        visiting.pop();
        return contains;
    }

    false
}
