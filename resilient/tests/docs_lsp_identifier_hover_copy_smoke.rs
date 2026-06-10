//! RES-3259: LSP docs describe implemented identifier hover support.

#[test]
fn lsp_docs_describe_current_identifier_hover_support() {
    let docs = include_str!("../../docs/lsp.md");
    let server = include_str!("../src/lsp_server.rs");

    for expected in [
        "| `42`    | `Int`",
        "| `3.14`  | `Float`",
        "| `\"hi\"`  | `String`",
        "| `true`  | `Bool`",
        "Hovering over identifiers also returns current best-effort type or",
        "top-level `let`, `const`, `static let`",
        "top-level function names, function parameters, and local `let`",
    ] {
        assert!(
            docs.contains(expected),
            "LSP docs should describe current hover support; missing {expected:?}"
        );
    }

    assert!(
        !docs.contains("Hover over identifiers (variables, functions) is a planned"),
        "LSP docs should not describe identifier hover as future-only"
    );
    assert!(
        !docs.contains("- Hover for identifiers (variables, parameters, function names)."),
        "LSP docs should not list implemented identifier hover as next work"
    );
    assert!(
        server.contains("The same server also exposes best-effort hover"),
        "LSP module header should mention current hover support"
    );
    assert!(
        !server.contains("Nothing else yet — no hover, no completion, no go-to-definition."),
        "LSP module header should not advertise the original minimum-viable scope"
    );
}
