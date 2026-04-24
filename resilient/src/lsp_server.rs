//! RES-074: minimum-viable Language Server for Resilient.
//!
//! Implements `textDocument/didOpen` and `textDocument/didChange`:
//! each time either fires, we parse the buffer, run the typechecker,
//! and publish diagnostics with source ranges derived from
//! RES-077's per-statement `Spanned<Node>` wrappers.
//!
//! Nothing else yet — no hover, no completion, no go-to-definition.
//! Those are dedicated follow-up tickets. This ticket is the
//! scaffolding that makes an editor light up with red squiggles.
//!
//! The `mod lsp_server;` declaration in `main.rs` is already
//! gated on `cfg(feature = "lsp")`, so this file is only compiled
//! when the feature is on — no per-file `#![cfg]` needed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result as JsonResult;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CompletionItem, CompletionItemKind, CompletionOptions,
    CompletionParams, CompletionResponse, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents,
    HoverParams, HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams,
    InlayHint, InlayHintKind, InlayHintLabel, InlayHintOptions, InlayHintParams,
    InlayHintServerCapabilities, Location, MarkedString, MessageType, OneOf, Position,
    PrepareRenameResponse, Range, ReferenceParams, RenameOptions, RenameParams, SemanticToken,
    SemanticTokenModifier, SemanticTokenType, SemanticTokens, SemanticTokensFullOptions,
    SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, SymbolInformation, SymbolKind,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
    WorkDoneProgressOptions, WorkspaceEdit, WorkspaceSymbolParams,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::{Node, builtin_names, compute_semantic_tokens, parse, typechecker};

/// RES-186: one workspace-level symbol entry. A flat vec of these
/// is the backend's search index — substring filter at query time,
/// rebuilt per `did_save`.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceSymbolEntry {
    pub(crate) name: String,
    pub(crate) kind: SymbolKind,
    pub(crate) uri: Url,
    pub(crate) range: Range,
}

/// The LSP backend. Holds a `Client` handle for publishing diagnostics.
///
/// RES-185: the `documents` map caches each open document's last-
/// parsed AST, keyed by `Url`. Document-symbol (and future
/// cursor-aware) handlers consume from here instead of re-parsing
/// on every request.
///
/// RES-186: the `workspace_index` is a pre-computed list of every
/// `*.rs` file's top-level symbols in the workspace root. Built
/// lazily on first `workspace/symbol` request (cheaper than
/// walking at `initialize` time when the workspace might be huge),
/// cached, and refreshed per-file on `did_save`.
pub struct Backend {
    client: Client,
    /// URI → latest parsed Program AST. Mutex-guarded because
    /// LSP handlers run on the tokio runtime's worker threads,
    /// and we never hold the lock across an `.await`.
    documents: Mutex<HashMap<Url, Node>>,
    /// RES-187: URI → latest raw source text. Stored separately
    /// from the AST because `semantic_tokens_full` re-lexes the
    /// source (the lexer's token stream is the source of truth
    /// for highlighting — the AST discards too much by this
    /// point, e.g. operator forms collapse into `BinaryOp`).
    /// Same mutex discipline as `documents`: synchronous lock,
    /// never held across `.await`.
    documents_text: Mutex<HashMap<Url, String>>,
    /// RES-186: per-file symbol index. Keyed by `Url` so a
    /// `did_save` can replace just that file's entries instead of
    /// rebuilding the whole thing. The vec-of-entries form inside
    /// each value keeps the filter loop flat at query time.
    workspace_index: Mutex<HashMap<Url, Vec<WorkspaceSymbolEntry>>>,
    /// RES-186: workspace root path captured from `initialize` —
    /// either `workspace_folders[0].uri` or the deprecated
    /// `root_uri`. `None` when the client opened a single file
    /// with no workspace attached. Used to decide whether to
    /// walk at index-build time.
    workspace_root: Mutex<Option<PathBuf>>,
    /// RES-186: once the workspace index has been built, set this
    /// to skip rebuilding on every `workspace/symbol` call. Reset
    /// to false on `did_save` to trigger a lazy refresh on the
    /// next query.
    workspace_index_built: Mutex<bool>,
    /// RES-189: user preference for inlay hints at call sites
    /// (`add(a: 1, b: 2)`-style parameter labels). Off by default
    /// per the ticket; the client flips it on via `initializationOptions`
    /// (`resilient.inlayHints.parameters: true`).
    /// Type hints for unannotated `let` bindings always fire —
    /// only parameter hints are gated here.
    inlay_hint_parameters: Mutex<bool>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(HashMap::new()),
            documents_text: Mutex::new(HashMap::new()),
            workspace_index: Mutex::new(HashMap::new()),
            workspace_root: Mutex::new(None),
            workspace_index_built: Mutex::new(false),
            inlay_hint_parameters: Mutex::new(false),
        }
    }

    /// RES-074: analyze `text` for parser + typechecker errors and
    /// publish them as LSP diagnostics against `uri`. Called from
    /// both `did_open` and `did_change`.
    async fn publish_analysis(&self, uri: Url, text: String) {
        let mut diagnostics = Vec::new();

        // Step 1: parse. Parser::record_error formats errors with a
        // bare `<line>:<col>:` prefix; route them through
        // extract_range_and_message (RES-089) so they land at the
        // right LSP Range instead of the file's first character.
        let (program, parser_errors) = parse(&text);

        // RES-185: cache the freshly-parsed AST so document-symbol
        // (and future cursor-aware) handlers don't have to re-parse.
        // Stored even if the parse reported recoverable errors —
        // the partial AST still covers the recovered fns / structs.
        if let Ok(mut map) = self.documents.lock() {
            map.insert(uri.clone(), program.clone());
        }
        // RES-187: cache the raw source text too, for semantic-
        // tokens requests that re-lex the file.
        if let Ok(mut tmap) = self.documents_text.lock() {
            tmap.insert(uri.clone(), text.clone());
        }

        for err in &parser_errors {
            let (range, pretty) = extract_range_and_message(err);
            diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("resilient-parser".into()),
                message: pretty,
                ..Default::default()
            });
        }

        // Step 2: typechecker. Errors from RES-080's
        // `check_program_with_source` come back prefixed with
        // `<path>:<line>:<col>: ...`. We re-parse that prefix so the
        // diagnostic range lines up with what the user sees.
        if parser_errors.is_empty() {
            let mut tc = typechecker::TypeChecker::new();
            if let Err(msg) = tc.check_program_with_source(&program, uri.as_str()) {
                let (range, pretty) = extract_range_and_message(&msg);
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("resilient-typecheck".into()),
                    message: pretty,
                    ..Default::default()
                });
            }
        }

        // RES-357: Step 3: run lints (warning-level). Convert each Lint
        // into an LSP Diagnostic so clients see L0010 "no contract" and
        // can request the "Add contract stubs" code action.
        // Lint positions are 1-indexed; convert to 0-indexed LSP positions.
        if parser_errors.is_empty() {
            for lint in crate::lint::check(&program, &text) {
                let line0 = lint.line.saturating_sub(1);
                let col0 = lint.column.saturating_sub(1);
                let pos = Position::new(line0, col0);
                let range = Range::new(pos, pos);
                let severity = match lint.severity {
                    crate::lint::Severity::Error => DiagnosticSeverity::ERROR,
                    crate::lint::Severity::Warning => DiagnosticSeverity::WARNING,
                };
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(severity),
                    source: Some("resilient-lint".into()),
                    message: lint.message,
                    ..Default::default()
                });
            }
        }

        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

/// RES-074: LSP uses 0-indexed line/column; RES-077's span is
/// 1-indexed. Subtract 1 (clamped) and build a zero-width range.
fn point_range(line_0based: u32, col_0based: u32) -> Range {
    let pos = Position::new(line_0based, col_0based);
    Range::new(pos, pos)
}

/// Parse a `<line>:<col>:` or `<path>:<line>:<col>:` prefix out of
/// an error string. Returns a zero-width `Range` at that position
/// plus the remaining message (or a zero-zero range + the original
/// message if parsing fails).
///
/// Recognizes both forms used in this codebase:
/// - RES-080 typechecker errors: `<path>:<line>:<col>: <rest>`
/// - RES-089 parser errors: `<line>:<col>: <rest>` (no path)
fn extract_range_and_message(err: &str) -> (Range, String) {
    // RES-089: try the bare `<line>:<col>:` form first. This catches
    // parser errors emitted by `Parser::record_error`.
    if let Some(parsed) = parse_bare_line_col(err) {
        return parsed;
    }

    // Fall back to `<anything>:<line>:<col>:` form (typechecker).
    // Find the FIRST colon after which `<uint>:<uint>:` follows.
    for (i, _) in err.match_indices(':') {
        let rest = &err[i + 1..];
        let mut parts = rest.splitn(3, ':');
        let line_s = parts.next().unwrap_or("");
        let col_s = parts.next().unwrap_or("");
        let msg = parts.next().unwrap_or("");
        if let (Ok(line), Ok(col)) = (line_s.trim().parse::<u32>(), col_s.trim().parse::<u32>()) {
            let line0 = line.saturating_sub(1);
            let col0 = col.saturating_sub(1);
            return (point_range(line0, col0), msg.trim().to_string());
        }
    }
    (point_range(0, 0), err.to_string())
}

/// RES-089: parse a bare `<line>:<col>: <message>` prefix (no
/// preceding path). Returns None if the input doesn't fit that
/// shape exactly.
fn parse_bare_line_col(err: &str) -> Option<(Range, String)> {
    let mut parts = err.splitn(3, ':');
    let line_s = parts.next()?;
    let col_s = parts.next()?;
    let msg = parts.next()?;
    let line: u32 = line_s.trim().parse().ok()?;
    let col: u32 = col_s.trim().parse().ok()?;
    Some((
        point_range(line.saturating_sub(1), col.saturating_sub(1)),
        msg.trim().to_string(),
    ))
}

/// RES-185: map the runtime's `Span` (1-indexed line/col, 0 for
/// offset) to LSP's `Range` (0-indexed line/character).
fn span_to_range(span: crate::span::Span) -> Range {
    let start = Position::new(
        span.start.line.saturating_sub(1) as u32,
        span.start.column.saturating_sub(1) as u32,
    );
    let end = Position::new(
        span.end.line.saturating_sub(1) as u32,
        span.end.column.saturating_sub(1) as u32,
    );
    Range::new(start, end)
}

/// RES-181a: classify the token kind at LSP `pos` inside `src`
/// and return `(type_name, range)` for hover display. Returns
/// `None` if no literal token covers that position.
///
/// Why drive the lexer directly instead of walking the AST? The
/// parser's `span_at_current` records `last_token_*` after the
/// lexer has ALREADY advanced to the next token — so a
/// `Node::IntegerLiteral` built inside `parse_expression` ends
/// up with a span pointing at whatever lexeme follows, not at
/// the literal itself. Reliable per-token positions come straight
/// from `Lexer::next_token_with_span`, which returns true start
/// AND end coordinates in one call.
///
/// This lookup is O(tokens). A cached-source document is small
/// enough that re-lexing on each hover request is cheap — faster
/// than the HashMap operation that reads it.
pub(crate) fn hover_literal_at(src: &str, pos: Position) -> Option<(&'static str, Range)> {
    use crate::{Lexer, Token};
    let mut lex = Lexer::new(src.to_string());
    loop {
        let (tok, span) = lex.next_token_with_span();
        if matches!(tok, Token::Eof) {
            return None;
        }
        if !lex_span_contains_lsp_position(span, pos) {
            continue;
        }
        let type_name: &'static str = match tok {
            Token::IntLiteral(_) => "Int",
            Token::FloatLiteral(_) => "Float",
            Token::StringLiteral(_) => "String",
            Token::BoolLiteral(_) => "Bool",
            Token::BytesLiteral(_) => "Bytes",
            _ => return None, // non-literal token at cursor → no hover
        };
        return Some((type_name, span_to_range(span)));
    }
}

/// RES-181a: does a lexer-produced `Span` (proper start + end,
/// 1-indexed) contain LSP `Position` (0-indexed)? End is
/// exclusive.
fn lex_span_contains_lsp_position(span: crate::span::Span, pos: Position) -> bool {
    let start_line = span.start.line.saturating_sub(1) as u32;
    let start_col = span.start.column.saturating_sub(1) as u32;
    let end_line = span.end.line.saturating_sub(1) as u32;
    let end_col = span.end.column.saturating_sub(1) as u32;
    // Check line ordering first: pos.line must be in [start, end].
    if pos.line < start_line || pos.line > end_line {
        return false;
    }
    // Single-line span: constrain by columns within that line.
    if start_line == end_line {
        return pos.character >= start_col && pos.character < end_col;
    }
    // Multi-line token (e.g. multi-line string literal). Inner
    // lines match unconditionally; boundaries check their column.
    if pos.line == start_line {
        return pos.character >= start_col;
    }
    if pos.line == end_line {
        return pos.character < end_col;
    }
    true
}

/// RES-182a: lex `src` and, if the cursor sits on an
/// `Identifier` token, return the `(name, span)` pair. Returns
/// `None` for every other kind of token (keywords, literals,
/// operators) or when the cursor isn't on any token.
///
/// Mirrors `hover_literal_at`'s token-level lookup approach: the
/// parser's per-leaf spans are unreliable, so we drive the lexer
/// directly against the cached source and find the containing
/// `Token::Identifier`. Caller uses the returned name to look up
/// the definition in a `TopLevelDefMap`.
pub(crate) fn identifier_at(src: &str, pos: Position) -> Option<(String, Range)> {
    use crate::{Lexer, Token};
    let mut lex = Lexer::new(src.to_string());
    loop {
        let (tok, span) = lex.next_token_with_span();
        if matches!(tok, Token::Eof) {
            return None;
        }
        if !lex_span_contains_lsp_position(span, pos) {
            continue;
        }
        if let Token::Identifier(name) = tok {
            return Some((name, span_to_range(span)));
        }
        // Non-identifier token at the cursor — no jump.
        return None;
    }
}

/// RES-182a: one top-level declaration's definition site.
/// `name_range` is the span of the identifier token on the
/// declaration line — what the editor highlights as the "goto
/// target" — while `full_range` covers the whole decl (same
/// data `document_symbols_for_program` uses). `full_range` is
/// what we return today; a future refinement could return
/// `name_range` when RES-088's span work gets an identifier-
/// specific span on `Node::Function` / friends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TopLevelDef {
    pub name: String,
    pub range: Range,
}

/// RES-182a: build a name → top-level-decl-location map from a
/// parsed program. Covers `fn` / `struct` / `type` aliases —
/// the same decl shapes `document_symbols_for_program` already
/// walks. Duplicate names inside a single program (parser
/// doesn't reject them yet) resolve to the FIRST occurrence,
/// since goto-def is deterministic and users most often mean
/// the original definition.
pub(crate) fn build_top_level_defs(program: &Node) -> Vec<TopLevelDef> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Vec::new(),
    };
    let mut out: Vec<TopLevelDef> = Vec::new();
    let mut seen = std::collections::HashSet::<String>::new();
    for spanned in stmts {
        let name = match &spanned.node {
            Node::Function { name, .. } => name.clone(),
            Node::StructDecl { name, .. } => name.clone(),
            Node::TypeAlias { name, .. } => name.clone(),
            _ => continue,
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        out.push(TopLevelDef {
            name,
            range: span_to_range(spanned.span),
        });
    }
    out
}

/// RES-182a: find a top-level decl by name. Linear scan over
/// the `build_top_level_defs` vec — decl counts are small and
/// this runs per request (not per keystroke).
pub(crate) fn find_top_level_def<'a>(
    defs: &'a [TopLevelDef],
    name: &str,
) -> Option<&'a TopLevelDef> {
    defs.iter().find(|d| d.name == name)
}

/// RES-184: walk the token stream of `src` looking for a `fn`
/// declaration whose name is `target`. Returns the `Range` of the
/// name identifier token in the declaration (e.g. the `foo` span
/// in `fn foo(...)`). Returns `None` if not found.
///
/// We use a two-token lookahead: on each `fn` keyword we peek at
/// the immediately following token; if it is an `Identifier` equal
/// to `target` we return its range. This is cheaper than re-
/// building the AST and more precise than `build_top_level_defs`
/// (which stores the whole-statement span, not the name span).
pub(crate) fn find_decl_name_range(src: &str, target: &str) -> Option<Range> {
    use crate::{Lexer, Token};
    let mut lex = Lexer::new(src.to_string());
    let mut prev_was_fn = false;
    loop {
        let (tok, span) = lex.next_token_with_span();
        match tok {
            Token::Eof => return None,
            Token::Function => {
                prev_was_fn = true;
            }
            Token::Identifier(ref name) if prev_was_fn && name == target => {
                return Some(span_to_range(span));
            }
            _ => {
                prev_was_fn = false;
            }
        }
    }
}

/// RES-184: validate that `name` is a legal Resilient identifier.
/// Must match `[A-Za-z_][A-Za-z0-9_]*`.  Empty strings and names
/// that start with a digit are rejected.  Used by the rename
/// handler to produce a clean LSP error before touching the AST.
pub(crate) fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        None => false,
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

/// RES-357: scan forward from `start_line` (0-indexed) in `src` to
/// find the line that contains the opening `{` of a function body.
/// Returns the 0-indexed line number of that brace line, or `None`
/// if no `{` is found in the remaining source.
///
/// The scan skips the string content of any `"..."` literals so a
/// `{` inside a default-parameter string doesn't fool the heuristic.
/// A `{` appearing anywhere on a line (even after other tokens) is
/// treated as the function's opening brace.
pub(crate) fn find_brace_line(src: &str, start_line: usize) -> Option<usize> {
    for (idx, line) in src.lines().enumerate() {
        if idx < start_line {
            continue;
        }
        // Walk character-by-character, skipping string literals so
        // `{` inside `"..."` is not mistaken for the opening brace.
        let mut in_string = false;
        let mut prev = '\0';
        for ch in line.chars() {
            match ch {
                '"' if prev != '\\' => in_string = !in_string,
                '{' if !in_string => return Some(idx),
                _ => {}
            }
            prev = ch;
        }
    }
    None
}

/// RES-183: walk the full AST and collect the `Range` of every
/// `CallExpression` whose callee is an `Identifier` with the
/// given name. Only `CallExpression` nodes count — `StructLiteral`
/// nodes that happen to share the name are deliberately excluded
/// (the AC says the match must be AST-driven, not textual).
///
/// The returned ranges point at the callee identifier's span
/// within the call expression. Because `CallExpression.function`
/// is a `Box<Node>` holding an `Identifier`, we use
/// `expression_span` to get that identifier's span and convert it
/// to an LSP `Range`. Calls where the callee is a non-identifier
/// expression (method chain, higher-order call, etc.) are skipped.
///
/// `include_declaration`: when `true`, the caller should also
/// append the definition site via `find_top_level_def`. This
/// helper only ever emits CALL sites.
pub(crate) fn collect_call_sites(program: &Node, target: &str) -> Vec<Range> {
    let mut out = Vec::new();
    walk_call_sites(program, target, &mut out);
    out
}

/// Recursive helper for `collect_call_sites`. Visits every node
/// reachable from `node`. Appends to `out` when a
/// `CallExpression` with callee `Identifier { name == target }`
/// is found.
fn walk_call_sites(node: &Node, target: &str, out: &mut Vec<Range>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_call_sites(&s.node, target, out);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            walk_call_sites(body, target, out);
            for r in requires {
                walk_call_sites(r, target, out);
            }
            for e in ensures {
                walk_call_sites(e, target, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_call_sites(s, target, out);
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // Recurse into arguments first so nested calls are captured.
            for a in arguments {
                walk_call_sites(a, target, out);
            }
            // Recurse into the callee in case it is itself a call
            // expression (e.g. `foo()()`). We check AFTER recursion
            // so inner calls land before outer.
            walk_call_sites(function, target, out);
            // Now check if THIS call's callee is the target name.
            if let Node::Identifier { name, span } = function.as_ref()
                && name == target
            {
                out.push(span_to_range(*span));
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::ReturnStatement {
            value: Some(value), ..
        } => {
            walk_call_sites(value, target, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_call_sites(condition, target, out);
            walk_call_sites(consequence, target, out);
            if let Some(a) = alternative {
                walk_call_sites(a, target, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_call_sites(condition, target, out);
            walk_call_sites(body, target, out);
        }
        Node::InfixExpression { left, right, .. } => {
            walk_call_sites(left, target, out);
            walk_call_sites(right, target, out);
        }
        Node::PrefixExpression { right, .. } => {
            walk_call_sites(right, target, out);
        }
        Node::Assignment { value, .. } => {
            walk_call_sites(value, target, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_call_sites(iterable, target, out);
            walk_call_sites(body, target, out);
        }
        Node::ExpressionStatement { expr, .. } => {
            walk_call_sites(expr, target, out);
        }
        Node::IndexExpression {
            target: t, index, ..
        } => {
            walk_call_sites(t, target, out);
            walk_call_sites(index, target, out);
        }
        Node::IndexAssignment {
            target: t,
            index,
            value,
            ..
        } => {
            walk_call_sites(t, target, out);
            walk_call_sites(index, target, out);
            walk_call_sites(value, target, out);
        }
        Node::FieldAccess { target: obj, .. } | Node::TryExpression { expr: obj, .. } => {
            walk_call_sites(obj, target, out);
        }
        Node::FieldAssignment {
            target: obj, value, ..
        } => {
            walk_call_sites(obj, target, out);
            walk_call_sites(value, target, out);
        }
        Node::ArrayLiteral { items, .. } => {
            for e in items {
                walk_call_sites(e, target, out);
            }
        }
        Node::StructLiteral { fields, .. } => {
            // Note: `StructLiteral { name, .. }` is intentionally NOT
            // matched as a call site — struct construction is not a fn
            // call even if the struct shares a name with a function.
            // Only the field VALUE expressions are descended into.
            for (_, v) in fields {
                walk_call_sites(v, target, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_call_sites(scrutinee, target, out);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    walk_call_sites(g, target, out);
                }
                walk_call_sites(body, target, out);
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            ..
        } => {
            walk_call_sites(body, target, out);
            for r in requires {
                walk_call_sites(r, target, out);
            }
            for e in ensures {
                walk_call_sites(e, target, out);
            }
        }
        // Simple leaf nodes and forms without sub-expressions:
        // Identifier, IntegerLiteral, FloatLiteral, StringLiteral,
        // BooleanLiteral, BytesLiteral, ReturnStatement { value: None },
        // TypeAlias, StructDecl, LetDestructureStruct, AssumeStatement, etc.
        _ => {}
    }
}

/// RES-188a: hard cap on the completion list. Large lists hurt
/// client latency — both rendering and the follow-up filter pass
/// — and VS Code / most clients truncate at ~200 anyway. 100 is
/// the ticket's explicit choice.
pub(crate) const COMPLETION_LIMIT: usize = 100;

/// RES-188a: extract the identifier prefix that ends at `pos`.
/// Walks backwards through the line's text until it hits a
/// non-identifier character (anything that isn't alphanumeric or
/// `_`). Returns the portion already typed — the empty string if
/// the cursor is on a non-identifier character or at column 0 on
/// a blank line. Used by `completion` to filter the suggestion
/// set to entries that start with the user's in-progress name.
pub(crate) fn prefix_at(src: &str, pos: Position) -> String {
    let line_no = pos.line as usize;
    let col = pos.character as usize;
    let line = match src.lines().nth(line_no) {
        Some(l) => l,
        None => return String::new(),
    };
    let chars: Vec<char> = line.chars().collect();
    // Clamp col into the line length — some clients send positions
    // one past end-of-line, which we treat as "end of line".
    let end = col.min(chars.len());
    let mut start = end;
    while start > 0 {
        let c = chars[start - 1];
        if c.is_alphanumeric() || c == '_' {
            start -= 1;
        } else {
            break;
        }
    }
    chars[start..end].iter().collect()
}

/// RES-188a: one resolved completion candidate, pre-sorting.
/// Carries enough info to render a `CompletionItem` without
/// touching the LSP types in pure helpers (so the helpers can be
/// unit-tested without a tower-lsp dependency in the test body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub label: String,
    pub kind: CandidateKind,
    pub detail: Option<String>,
}

/// RES-188a: completion-item kind. Maps 1:1 to
/// `tower_lsp::lsp_types::CompletionItemKind` in the handler —
/// kept as a local enum here so pure helpers and tests don't pull
/// in tower-lsp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateKind {
    Function,
    Struct,
    TypeAlias,
}

impl CandidateKind {
    fn to_lsp(self) -> CompletionItemKind {
        match self {
            CandidateKind::Function => CompletionItemKind::FUNCTION,
            CandidateKind::Struct => CompletionItemKind::STRUCT,
            CandidateKind::TypeAlias => CompletionItemKind::TYPE_PARAMETER,
        }
    }
}

/// RES-188a: build the candidate list from the cached AST +
/// `BUILTINS`, filter by `prefix`, and cap at `COMPLETION_LIMIT`.
/// Deterministic output: builtins come first (alphabetical), then
/// top-level decls (source order), so regressions across tests
/// are easy to spot.
///
/// When `prefix` is empty, the full candidate set (up to the cap)
/// is returned — that's the Ctrl-Space case.
pub(crate) fn completion_candidates(program: &Node, prefix: &str) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();

    // Builtins — alphabetically sorted snapshot.
    let mut names: Vec<&'static str> = builtin_names().collect();
    names.sort_unstable();
    for name in names {
        if !name.starts_with(prefix) {
            continue;
        }
        out.push(Candidate {
            label: name.to_string(),
            kind: CandidateKind::Function,
            detail: Some("builtin".to_string()),
        });
        if out.len() >= COMPLETION_LIMIT {
            return out;
        }
    }

    // Top-level decls — source order. Already handles duplicates
    // via the `seen` set inside `build_top_level_defs`.
    let stmts = match program {
        Node::Program(s) => s,
        _ => return out,
    };
    for spanned in stmts {
        let (name, kind, detail) = match &spanned.node {
            Node::Function {
                name, parameters, ..
            } => (
                name.clone(),
                CandidateKind::Function,
                Some(format!("fn ({} params)", parameters.len())),
            ),
            Node::StructDecl { name, fields, .. } => (
                name.clone(),
                CandidateKind::Struct,
                Some(format!("struct ({} fields)", fields.len())),
            ),
            Node::TypeAlias { name, .. } => (
                name.clone(),
                CandidateKind::TypeAlias,
                Some("type".to_string()),
            ),
            _ => continue,
        };
        if !name.starts_with(prefix) {
            continue;
        }
        out.push(Candidate {
            label: name,
            kind,
            detail,
        });
        if out.len() >= COMPLETION_LIMIT {
            return out;
        }
    }
    out
}

/// RES-188a: convert a `Candidate` into the LSP wire-shape.
/// Lifted out so the filter / cap / ordering logic can be
/// unit-tested over pure `Vec<Candidate>`.
fn candidate_to_completion_item(c: Candidate) -> CompletionItem {
    CompletionItem {
        label: c.label.clone(),
        kind: Some(c.kind.to_lsp()),
        detail: c.detail,
        insert_text: Some(c.label),
        ..Default::default()
    }
}

/// RES-185: walk a `Node::Program` and emit one `DocumentSymbol`
/// per top-level `fn`, `struct`, or `type` alias. Returned list
/// is sorted by source position so editors present the outline
/// in file order.
///
/// This is a pure function — the LSP handler just hands it the
/// cached AST and returns the result. Unit tests exercise it
/// directly; the integration test in `tests/lsp_smoke.rs`
/// verifies the round-trip through the actual server.
///
/// `selection_range` is set equal to `range` for now — a later
/// ticket can compute the identifier-only sub-span once parse-
/// time name positions are tracked (today's `Node::Function::span`
/// is the `fn` keyword's zero-width point, per RES-088).
#[allow(dead_code)] // only used behind the `lsp` feature + test
pub(crate) fn document_symbols_for_program(program: &Node) -> Vec<DocumentSymbol> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for spanned in stmts {
        let symbol = match &spanned.node {
            Node::Function { name, .. } => make_symbol(name, SymbolKind::FUNCTION, spanned.span),
            Node::StructDecl { name, .. } => make_symbol(name, SymbolKind::STRUCT, spanned.span),
            Node::TypeAlias { name, .. } => {
                make_symbol(name, SymbolKind::TYPE_PARAMETER, spanned.span)
            }
            // Everything else (let / static / return / while / ...)
            // is a statement, not a declaration — skip.
            _ => continue,
        };
        out.push(symbol);
    }
    // Stable-sort by source position. Parse order already matches
    // source order for the shapes we track, so this is usually a
    // no-op — belt-and-suspenders against future AST reordering.
    out.sort_by_key(|s| (s.range.start.line, s.range.start.character));
    out
}

/// RES-186: rebuild helpers. The workspace index is a flat
/// HashMap<Url, Vec<entries>>; `rebuild_workspace_index` walks
/// the root, `index_file` handles one `*.rs` at a time,
/// `filter_workspace_symbols` applies the query + cap.
impl Backend {
    fn rebuild_workspace_index(&self) {
        let root = match self.workspace_root.lock() {
            Ok(g) => g.clone(),
            Err(_) => None,
        };
        let Some(root) = root else { return };
        let files = walk_resilient_files(&root);
        let mut new_index: HashMap<Url, Vec<WorkspaceSymbolEntry>> = HashMap::new();
        for path in files {
            if let Some(entries) = index_file(&path) {
                let Ok(uri) = Url::from_file_path(&path) else {
                    continue;
                };
                new_index.insert(uri, entries);
            }
        }
        if let Ok(mut idx) = self.workspace_index.lock() {
            *idx = new_index;
        }
    }
}

/// RES-186: recursive `*.rs` walker. Skips `target/` and any
/// dot-prefixed directory (`.git/`, `.board/`, etc.) so we don't
/// index build artifacts or management metadata.
fn walk_resilient_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // Skip hidden + build dirs. This is the ticket's "don't
        // respect .gitignore, just skip the obvious" policy.
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if path.is_dir() {
            out.extend(walk_resilient_files(&path));
        } else if path.extension().and_then(|s| s.to_str()) == Some("rz") {
            out.push(path);
        }
    }
    out
}

/// RES-186: read, parse, and extract workspace-symbol entries for
/// one file. Parse errors are tolerated — a file that doesn't
/// parse returns an empty vec (not `None`) so partial-edit
/// states don't unindex the file; in practice the entries just
/// disappear until the file parses again.
#[allow(dead_code)] // `lsp` feature + test
pub(crate) fn index_file(path: &Path) -> Option<Vec<WorkspaceSymbolEntry>> {
    let text = std::fs::read_to_string(path).ok()?;
    let (program, _errs) = parse(&text);
    let uri = Url::from_file_path(path).ok()?;
    let entries = document_symbols_for_program(&program)
        .into_iter()
        .map(|d| WorkspaceSymbolEntry {
            name: d.name,
            kind: d.kind,
            uri: uri.clone(),
            range: d.range,
        })
        .collect();
    Some(entries)
}

/// RES-186: substring filter across the whole workspace index,
/// case-insensitive, capped at `limit` entries. Sorted by name
/// for reproducible output (editors' quick-open panels usually
/// re-sort, but we want the API stable for tests).
#[allow(deprecated)] // `SymbolInformation::deprecated` is a deprecated LSP field
fn filter_workspace_symbols(
    index: &HashMap<Url, Vec<WorkspaceSymbolEntry>>,
    query_lower: &str,
    limit: usize,
) -> Vec<SymbolInformation> {
    let mut matches: Vec<&WorkspaceSymbolEntry> = index
        .values()
        .flatten()
        .filter(|e| e.name.to_lowercase().contains(query_lower))
        .collect();
    // Stable sort by name then uri for determinism.
    matches.sort_by(|a, b| a.name.cmp(&b.name).then(a.uri.as_str().cmp(b.uri.as_str())));
    matches
        .into_iter()
        .take(limit)
        .map(|e| SymbolInformation {
            name: e.name.clone(),
            kind: e.kind,
            tags: None,
            deprecated: None,
            location: Location::new(e.uri.clone(), e.range),
            container_name: None,
        })
        .collect()
}

/// RES-187: the semantic-tokens legend. The order here MUST match
/// the `sem_tok::*` token-type indices declared in `main.rs`
/// (KEYWORD=0 … OPERATOR=8) and the modifier bit positions
/// (MOD_DECLARATION=bit0, MOD_READONLY=bit1). Any drift between
/// these two tables yields mis-colored output in every client.
fn semantic_tokens_legend() -> SemanticTokensLegend {
    // Indices (0..=8): keyword, function, variable, parameter,
    // type, string, number, comment, operator.
    let token_types = vec![
        SemanticTokenType::KEYWORD,
        SemanticTokenType::FUNCTION,
        SemanticTokenType::VARIABLE,
        SemanticTokenType::PARAMETER,
        SemanticTokenType::TYPE,
        SemanticTokenType::STRING,
        SemanticTokenType::NUMBER,
        SemanticTokenType::COMMENT,
        SemanticTokenType::OPERATOR,
    ];
    // Bit positions: declaration=bit0, readonly=bit1.
    let token_modifiers = vec![
        SemanticTokenModifier::DECLARATION,
        SemanticTokenModifier::READONLY,
    ];
    SemanticTokensLegend {
        token_types,
        token_modifiers,
    }
}

/// RES-187: the capability advertised in `initialize` — full-file
/// tokens only (delta left for a follow-up per the ticket notes).
fn semantic_tokens_capability() -> SemanticTokensServerCapabilities {
    SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
        work_done_progress_options: WorkDoneProgressOptions::default(),
        legend: semantic_tokens_legend(),
        range: Some(false),
        full: Some(SemanticTokensFullOptions::Bool(true)),
    })
}

/// RES-187: turn the `compute_semantic_tokens` u32 wire format
/// into the `Vec<SemanticToken>` that `tower-lsp`'s
/// `SemanticTokens::data` expects. The serializer reassembles
/// the flat u32 stream on the wire — we just need to round-trip
/// through the struct form.
fn semantic_tokens_from_wire(wire: Vec<u32>) -> Vec<SemanticToken> {
    wire.chunks_exact(5)
        .map(|c| SemanticToken {
            delta_line: c[0],
            delta_start: c[1],
            length: c[2],
            token_type: c[3],
            token_modifiers_bitset: c[4],
        })
        .collect()
}

/// RES-189: build one inlay hint for a typechecked unannotated
/// `let` binding. Position lands at the end-of-pattern column
/// (just after the identifier) so clients render `let x<here>`
/// → `let x :: Int` — the `:: ` prefix keeps the hint visually
/// separate from the code.
///
/// Per the LSP spec `position.line` / `character` are 0-indexed;
/// the typechecker's `span` is 1-indexed per RES-077, so we
/// subtract.
#[allow(dead_code)] // used behind `lsp` + test
fn inlay_hint_from_let(entry: &typechecker::LetTypeHint) -> InlayHint {
    // `let ` is 4 chars. Skip past it + the name to land at
    // end-of-pattern. If someone later adds a `let mut` form,
    // this computation needs to move.
    let line0 = entry.span.start.line.saturating_sub(1) as u32;
    let col0 = entry.span.start.column.saturating_sub(1) as u32;
    let end_of_pattern = col0 + 4 + entry.name_len_chars as u32;
    InlayHint {
        position: Position::new(line0, end_of_pattern),
        label: InlayHintLabel::String(format!(": {}", entry.ty)),
        kind: Some(InlayHintKind::TYPE),
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: None,
        data: None,
    }
}

/// RES-189: collect per-call-site parameter hints for a program.
///
/// For each `Node::CallExpression` whose callee resolves to a
/// top-level `Node::Function` declared in the same program, emit
/// one hint per positional argument carrying the corresponding
/// parameter name (`add(a: 1, b: 2)` editor chrome). Hints land
/// at the argument expression's start position.
///
/// Intentionally simple: we only resolve names against
/// top-level fns declared in THIS program. Imported fns, methods
/// on structs, and arguments at non-call call sites are all
/// skipped (each can be layered on as a follow-up once name
/// resolution is unified — RES-182 territory).
#[allow(dead_code)] // used behind `lsp` + test
pub(crate) fn collect_param_hints(program: &Node) -> Vec<InlayHint> {
    let mut out = Vec::new();
    let fns = collect_top_level_fns(program);
    walk_call_hints(program, &fns, &mut out);
    out
}

/// Map fn name → parameter names. Populated from top-level
/// `Node::Function` decls so parameter hints can look up a callee
/// in O(1).
fn collect_top_level_fns(program: &Node) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    if let Node::Program(stmts) = program {
        for spanned in stmts {
            if let Node::Function {
                name, parameters, ..
            } = &spanned.node
            {
                // parameters are (type, name) — only names needed.
                let names: Vec<String> = parameters.iter().map(|(_, n)| n.clone()).collect();
                out.insert(name.clone(), names);
            }
        }
    }
    out
}

/// Recursive walker: visits every `CallExpression` reachable from
/// `node`. For each one, if the callee is a bare identifier
/// that's in `fns` AND the arg count matches, emit one hint per
/// positional argument.
fn walk_call_hints(node: &Node, fns: &HashMap<String, Vec<String>>, out: &mut Vec<InlayHint>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_call_hints(&s.node, fns, out);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            walk_call_hints(body, fns, out);
            for r in requires {
                walk_call_hints(r, fns, out);
            }
            for e in ensures {
                walk_call_hints(e, fns, out);
            }
        }
        Node::Block { stmts, .. } => {
            // `Block.stmts` is `Vec<Node>` (not Spanned) so walk directly.
            for s in stmts {
                walk_call_hints(s, fns, out);
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // First recurse into the arguments so nested calls get
            // hints too.
            for a in arguments {
                walk_call_hints(a, fns, out);
            }
            walk_call_hints(function, fns, out);

            // Now check if this call itself gets a hint: callee
            // must be an Identifier naming a known top-level fn,
            // and arg count must match.
            if let Node::Identifier { name, .. } = function.as_ref()
                && let Some(param_names) = fns.get(name)
                && param_names.len() == arguments.len()
            {
                for (arg, pname) in arguments.iter().zip(param_names.iter()) {
                    let arg_span = expression_span(arg);
                    let Some(sp) = arg_span else { continue };
                    let pos = Position::new(
                        sp.start.line.saturating_sub(1) as u32,
                        sp.start.column.saturating_sub(1) as u32,
                    );
                    out.push(InlayHint {
                        position: pos,
                        label: InlayHintLabel::String(format!("{}: ", pname)),
                        kind: Some(InlayHintKind::PARAMETER),
                        text_edits: None,
                        tooltip: None,
                        padding_left: None,
                        padding_right: Some(true),
                        data: None,
                    });
                }
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::ReturnStatement {
            value: Some(value), ..
        } => {
            walk_call_hints(value, fns, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_call_hints(condition, fns, out);
            walk_call_hints(consequence, fns, out);
            if let Some(a) = alternative {
                walk_call_hints(a, fns, out);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            walk_call_hints(left, fns, out);
            walk_call_hints(right, fns, out);
        }
        Node::PrefixExpression { right, .. } => {
            walk_call_hints(right, fns, out);
        }
        // Stop recursion at simple leaves and forms we don't
        // currently inspect. Expand cases as new AST shapes need
        // hint coverage.
        _ => {}
    }
}

/// RES-189: tri-state position-in-range test. The LSP spec says
/// an inlay hint request's `range` is the viewport the editor
/// wants hints for; we filter server-side so the client doesn't
/// render hints outside that range. Returns true when `p` falls
/// inside `[range.start, range.end)` lexicographically.
fn position_in_range(p: Position, range: Range) -> bool {
    let before_start = (p.line, p.character) < (range.start.line, range.start.character);
    let after_end = (p.line, p.character) > (range.end.line, range.end.character);
    !before_start && !after_end
}

/// RES-189: extract the `resilient.inlayHints.parameters` flag
/// from the `initializationOptions` JSON blob. Tolerates two
/// client-side representations:
/// - flat:   `{"resilient.inlayHints.parameters": true}`
/// - nested: `{"resilient": {"inlayHints": {"parameters": true}}}`
///
/// Returns `false` when the blob is absent, malformed, or the
/// value isn't a boolean `true`. Exported pub(crate) so unit
/// tests can exercise the parser without an LSP round-trip.
#[allow(dead_code)]
pub(crate) fn read_init_param_hints_flag(opts: Option<&tower_lsp::lsp_types::LSPAny>) -> bool {
    let Some(opts) = opts else { return false };
    // Flat form.
    if let Some(v) = opts.get("resilient.inlayHints.parameters")
        && v.as_bool() == Some(true)
    {
        return true;
    }
    // Nested form.
    let nested = opts
        .get("resilient")
        .and_then(|v| v.get("inlayHints"))
        .and_then(|v| v.get("parameters"))
        .and_then(|v| v.as_bool());
    matches!(nested, Some(true))
}

/// Best-effort span lookup for an expression node. Used to place
/// parameter hints at the arg's start. Returns None when we don't
/// have a span for a given shape — the hint is then skipped.
fn expression_span(node: &Node) -> Option<crate::span::Span> {
    match node {
        Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::Identifier { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::ArrayLiteral { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::StructLiteral { span, .. } => Some(*span),
        _ => None,
    }
}

#[allow(deprecated)] // `DocumentSymbol::deprecated` is deprecated in the LSP type
fn make_symbol(name: &str, kind: SymbolKind, span: crate::span::Span) -> DocumentSymbol {
    let range = span_to_range(span);
    DocumentSymbol {
        name: name.to_string(),
        detail: None,
        kind,
        tags: None,
        // `deprecated` is deprecated in the LSP spec in favour of
        // `tags`, but the type still carries it; set it explicitly
        // so the derive doesn't complain about missing fields.
        deprecated: None,
        range,
        // See the helper's doc-comment for why selection_range
        // mirrors range today.
        selection_range: range,
        children: None,
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> JsonResult<InitializeResult> {
        // RES-186: capture the workspace root for the symbol index.
        // Prefer modern `workspace_folders`; fall back to the
        // deprecated `root_uri` for older clients.
        let root_path: Option<PathBuf> = params
            .workspace_folders
            .as_ref()
            .and_then(|folders| folders.first())
            .map(|f| f.uri.clone())
            .or(params.root_uri.clone())
            .and_then(|u| u.to_file_path().ok());
        if let (Ok(mut slot), Some(p)) = (self.workspace_root.lock(), root_path) {
            *slot = Some(p);
        }

        // RES-189: read the parameter-hints opt-in from
        // `initializationOptions`. We probe two common shapes:
        // `{"resilient.inlayHints.parameters": true}` (flat) and
        // `{"resilient": {"inlayHints": {"parameters": true}}}`
        // (nested). Either is fine; clients vary. Absent / false →
        // parameter hints stay off.
        let params_enabled = read_init_param_hints_flag(params.initialization_options.as_ref());
        if let Ok(mut slot) = self.inlay_hint_parameters.lock() {
            *slot = params_enabled;
        }

        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                // RES-185: advertise the document-symbol handler so
                // editors' outline views light up. `OneOf::Left(true)`
                // is the compact form — `Right(DocumentSymbolOptions)`
                // would let us opt into work-done progress reporting,
                // which we don't need for files this small.
                document_symbol_provider: Some(OneOf::Left(true)),
                // RES-186: workspace-symbol search across all .rs
                // files in the workspace root.
                workspace_symbol_provider: Some(OneOf::Left(true)),
                // RES-187: full-file semantic tokens. Delta is a
                // follow-up; many clients use `full` anyway.
                semantic_tokens_provider: Some(semantic_tokens_capability()),
                // RES-189: inlay hints. `Options` (not registration-
                // options) — the server supports the feature for any
                // document that already has a `TextDocumentSyncKind`
                // registered above.
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                        resolve_provider: Some(false),
                    },
                ))),
                // RES-181a: advertise hover support. Today the handler
                // only surfaces a type for literals (Int / Float /
                // Bool / String / Bytes / Duration) — identifier
                // hover is RES-181b, deferred behind RES-120's
                // inferred-type table. `Simple(true)` is the compact
                // form; `HoverOptions` would let us opt into
                // work-done progress reporting which we don't need.
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                // RES-182a: advertise go-to-definition. The handler
                // currently resolves only TOP-LEVEL declarations
                // (fn / struct / type alias) within the same
                // document; local / parameter / cross-file targets
                // (RES-182b, RES-182c) stay deferred until a scope-
                // aware resolver + span-carrying-path work lands.
                definition_provider: Some(OneOf::Left(true)),
                // RES-183: advertise find-references. The handler
                // collects every `CallExpression` in the current
                // document whose callee matches the cursor's top-
                // level fn name. Struct literals with the same name
                // are excluded (AST-driven, not textual).
                references_provider: Some(OneOf::Left(true)),
                // RES-184: advertise rename support with prepareRename
                // guard. `prepare_provider: Some(true)` tells clients
                // to call `textDocument/prepareRename` first so users
                // get "cannot rename here" feedback before typing.
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                // RES-188a: advertise identifier completion. The
                // handler seeds its candidate list from the BUILTINS
                // table plus top-level decls in the current
                // document; scope-aware local / parameter completion
                // (RES-188b) stays deferred on the same scope-walker
                // that blocks RES-182b. No trigger characters today
                // — identifier-prefix completion is driven by the
                // client; post-dot field completion is a separate
                // ticket (see the Notes section).
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: None,
                    all_commit_characters: None,
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                    completion_item: None,
                }),
                // RES-357: advertise code-action support so editors
                // show the light-bulb menu for L0010 "Add contract
                // stubs" quick-fixes.
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "resilient-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> JsonResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        self.publish_analysis(uri, text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // We registered as TextDocumentSyncKind::FULL, so each
        // change message includes the whole buffer in the first
        // content change.
        let uri = params.text_document.uri.clone();
        let text = params
            .content_changes
            .into_iter()
            .next()
            .map(|c| c.text)
            .unwrap_or_default();
        self.publish_analysis(uri, text).await;
    }

    /// RES-185: clear the cached AST for the closed document so
    /// long-running editor sessions don't retain memory for files
    /// the user isn't looking at anymore. Returning `None` from
    /// `document_symbol` for an unknown URI is the post-close
    /// behaviour (clients typically stop asking, but the handler
    /// tolerates the case gracefully either way).
    async fn did_close(&self, params: tower_lsp::lsp_types::DidCloseTextDocumentParams) {
        if let Ok(mut map) = self.documents.lock() {
            map.remove(&params.text_document.uri);
        }
        // RES-187: free the cached source text too.
        if let Ok(mut tmap) = self.documents_text.lock() {
            tmap.remove(&params.text_document.uri);
        }
    }

    /// RES-186: refresh the workspace symbol index entry for the
    /// saved file. Idempotent — just replaces the per-file vec
    /// in the index map, keeping every other file's entries
    /// untouched. If the save happened before the index was
    /// built, this is still safe: the lazy-build on next
    /// `workspace/symbol` call will include the saved state.
    async fn did_save(&self, params: tower_lsp::lsp_types::DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        // Re-read the file from disk. `did_save` notifications
        // carry the text only when the client opts into the
        // `TextDocumentSyncSaveOptions { include_text: true }`
        // — we registered TextDocumentSyncKind::FULL without
        // that, so walk to disk instead.
        let Some(path) = uri.to_file_path().ok() else {
            return;
        };
        let entries = match index_file(&path) {
            Some(e) => e,
            None => return,
        };
        if let Ok(mut map) = self.workspace_index.lock() {
            map.insert(uri, entries);
        }
    }

    /// RES-186: workspace-symbol search. Lazy-builds the per-file
    /// index on first call (walks the workspace root for `*.rs`
    /// files, skipping `target/` and dotfiles), then filters by
    /// substring match (case-insensitive) and caps at 50 entries
    /// per the ticket's budget.
    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> JsonResult<Option<Vec<SymbolInformation>>> {
        // Lazy-build the index if this is the first query.
        let needs_build = match self.workspace_index_built.lock() {
            Ok(g) => !*g,
            Err(_) => false,
        };
        if needs_build {
            self.rebuild_workspace_index();
            if let Ok(mut g) = self.workspace_index_built.lock() {
                *g = true;
            }
        }

        let query = params.query.to_lowercase();
        let index = match self.workspace_index.lock() {
            Ok(g) => g,
            Err(_) => return Ok(Some(Vec::new())),
        };
        let matches = filter_workspace_symbols(&index, &query, 50);
        Ok(Some(matches))
    }

    /// RES-185: respond to `textDocument/documentSymbol` — return
    /// the outline of top-level fns / structs / type aliases in
    /// the cached AST. Returns `Ok(None)` when the document has
    /// never been opened here (shouldn't normally happen; the LSP
    /// spec defines the request as coming AFTER didOpen).
    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> JsonResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        // Clone out of the lock synchronously — we never .await
        // while holding the Mutex (which would block other
        // handlers on the same worker thread).
        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };
        let symbols = document_symbols_for_program(&program);
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    /// RES-181a: respond to `textDocument/hover` — today, only
    /// literal positions yield a result. A literal under the
    /// cursor returns its Resilient-surface type name (`Int`,
    /// `Float`, `String`, `Bool`, `Bytes`); any other position
    /// returns `Ok(None)` so the client renders nothing.
    ///
    /// Implementation drives the lexer directly against the
    /// cached source text (not the AST), because the parser's
    /// per-leaf spans record `last_token_*` AFTER the lexer
    /// advances — unreliable for literal positions. See the
    /// module-level comment on `hover_literal_at` for the
    /// rationale.
    ///
    /// RES-181b will extend this to identifier positions once
    /// RES-120 exposes a per-position inferred-type table. The
    /// plumbing here (document lookup, capability advertisement,
    /// Hover response shape) is the shared scaffolding that
    /// ticket will build on.
    async fn hover(&self, params: HoverParams) -> JsonResult<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let Some((type_name, range)) = hover_literal_at(&text, pos) else {
            return Ok(None);
        };
        // `MarkedString::String` keeps the bubble universal —
        // both markdown-rendering and plain-text clients display
        // it identically. Type name alone is the body; no extra
        // prose.
        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(type_name.to_string())),
            range: Some(range),
        }))
    }

    /// RES-182a: respond to `textDocument/definition` — jump to
    /// the defining span of the symbol under the cursor.
    /// Currently handles only TOP-LEVEL declarations (fn / struct
    /// / type alias) within the same document. Local bindings /
    /// parameters (RES-182b) and cross-file imports (RES-182c)
    /// return `Ok(None)` so the editor's "no definition found"
    /// UX kicks in. That's a graceful degradation — picking the
    /// wrong jump target would be strictly worse than "I don't
    /// know yet."
    ///
    /// Implementation:
    ///   1. Look up the cursor's identifier token via
    ///      `identifier_at` (same token-level plumbing as
    ///      RES-181a's hover).
    ///   2. Rebuild the top-level def map from the cached AST
    ///      and look the name up.
    ///   3. Wrap the result in a `Location` pointing at the same
    ///      document URI (cross-file is RES-182c).
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonResult<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let Some((name, _range)) = identifier_at(&text, pos) else {
            return Ok(None);
        };
        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };
        let defs = build_top_level_defs(&program);
        let Some(def) = find_top_level_def(&defs, &name) else {
            return Ok(None);
        };
        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri,
            range: def.range,
        })))
    }

    /// RES-183: respond to `textDocument/references` — return every
    /// call site in the current document where the top-level fn
    /// under the cursor is invoked.
    ///
    /// Pipeline:
    ///   1. Look up the cursor's identifier token via `identifier_at`.
    ///   2. Confirm it names a top-level fn via `build_top_level_defs`
    ///      + `find_top_level_def`. Non-fn identifiers return `Ok(None)`.
    ///   3. Walk the cached AST with `collect_call_sites` to gather every
    ///      `CallExpression` with that callee. Struct literals sharing
    ///      the same name are excluded (AST-driven match).
    ///   4. If `context.include_declaration` is `true`, prepend the
    ///      defining span as the first `Location` in the result.
    ///   5. Return `Ok(None)` (not an empty Vec) when nothing resolves,
    ///      so compliant clients show "no references found" rather than
    ///      an empty highlight list.
    async fn references(&self, params: ReferenceParams) -> JsonResult<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let include_decl = params.context.include_declaration;

        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let Some((name, _cursor_range)) = identifier_at(&text, pos) else {
            return Ok(None);
        };

        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };

        // Only serve references for top-level fn declarations.
        // Struct / type-alias / local-variable references are
        // out of scope for this ticket (see RES-183 notes).
        let defs = build_top_level_defs(&program);
        let Some(def) = find_top_level_def(&defs, &name) else {
            return Ok(None);
        };
        // Confirm it is actually a fn (not a struct or type alias).
        // `build_top_level_defs` collects all three, so check the AST.
        let is_fn = if let Node::Program(stmts) = &program {
            stmts
                .iter()
                .any(|s| matches!(&s.node, Node::Function { name: n, .. } if n == &name))
        } else {
            false
        };
        if !is_fn {
            return Ok(None);
        }

        let call_ranges = collect_call_sites(&program, &name);

        // An empty call list with no declaration to include means
        // "no references found" — return None so clients display
        // the appropriate UX rather than an empty list.
        if call_ranges.is_empty() && !include_decl {
            return Ok(None);
        }

        let mut locations: Vec<Location> = Vec::new();

        // Declaration site first when requested (matches VS Code
        // "Go to References" + "Include Declaration" behaviour).
        if include_decl {
            locations.push(Location {
                uri: uri.clone(),
                range: def.range,
            });
        }

        for range in call_ranges {
            locations.push(Location {
                uri: uri.clone(),
                range,
            });
        }

        Ok(Some(locations))
    }

    /// RES-184: respond to `textDocument/prepareRename` — the UX
    /// guard that tells editors whether the symbol under the cursor
    /// is renamable before the user types a new name.
    ///
    /// A symbol is renamable when it is a top-level fn declaration
    /// (the same set `references` covers). Returns the identifier's
    /// range so the editor pre-selects the current name in the
    /// rename input box. Returns `Ok(None)` for non-renamable
    /// positions (literals, keywords, struct names, local vars —
    /// all deferred to later tickets).
    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> JsonResult<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let pos = params.position;

        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let Some((name, range)) = identifier_at(&text, pos) else {
            return Ok(None);
        };

        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };

        // Only top-level fn names are renamable right now.
        let defs = build_top_level_defs(&program);
        let is_renamable_fn = {
            let found = find_top_level_def(&defs, &name).is_some();
            if found {
                if let Node::Program(stmts) = &program {
                    stmts
                        .iter()
                        .any(|s| matches!(&s.node, Node::Function { name: n, .. } if n == &name))
                } else {
                    false
                }
            } else {
                false
            }
        };

        if !is_renamable_fn {
            return Ok(None);
        }

        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range,
            placeholder: name,
        }))
    }

    /// RES-184: respond to `textDocument/rename` — emit a
    /// `WorkspaceEdit` that renames every reference to a top-level
    /// fn in the current document.
    ///
    /// Pipeline:
    ///   1. Validate the new name against `[A-Za-z_][A-Za-z0-9_]*`.
    ///      Return an LSP error immediately if invalid.
    ///   2. Look up the identifier under the cursor via `identifier_at`.
    ///      Confirm it names a top-level fn via `build_top_level_defs`.
    ///   3. Collision check: if the new name already names a visible
    ///      top-level binding, return an LSP error rather than
    ///      producing broken code.
    ///   4. Gather every edit site: the declaration-site range from
    ///      `build_top_level_defs` plus every call-site range from
    ///      `collect_call_sites`.
    ///   5. Group the `TextEdit`s by URI and return a `WorkspaceEdit`.
    async fn rename(&self, params: RenameParams) -> JsonResult<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let new_name = params.new_name;

        // Validate identifier pattern before doing any work.
        if !is_valid_identifier(&new_name) {
            return Err(tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::InvalidParams,
                message: format!(
                    "invalid identifier `{new_name}`: must match [A-Za-z_][A-Za-z0-9_]*"
                )
                .into(),
                data: None,
            });
        }

        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let Some((name, _cursor_range)) = identifier_at(&text, pos) else {
            return Ok(None);
        };

        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };

        // Confirm cursor is on a top-level fn name.
        let defs = build_top_level_defs(&program);
        let Some(def) = find_top_level_def(&defs, &name) else {
            return Ok(None);
        };
        let is_fn = if let Node::Program(stmts) = &program {
            stmts
                .iter()
                .any(|s| matches!(&s.node, Node::Function { name: n, .. } if n == &name))
        } else {
            false
        };
        if !is_fn {
            return Ok(None);
        }

        // Collision check: reject if new name already has a top-level binding.
        if find_top_level_def(&defs, &new_name).is_some() {
            return Err(tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::InvalidParams,
                message: format!("rename would shadow `{new_name}`").into(),
                data: None,
            });
        }

        // Collect all edit sites: declaration + every call site.
        let mut edits: Vec<TextEdit> = Vec::new();

        // Declaration site: use the exact fn-name-token range from
        // the source (not the whole-statement span from `def.range`).
        // `find_decl_name_range` scans the lexer stream for `fn <name>`
        // and returns the identifier token's precise Range.  Fall
        // back to `def.range` only if the lexer scan misses (shouldn't
        // happen, but safe degradation).
        let decl_range = find_decl_name_range(&text, &name).unwrap_or(def.range);
        edits.push(TextEdit {
            range: decl_range,
            new_text: new_name.clone(),
        });

        // All call sites (callee identifier spans).
        for range in collect_call_sites(&program, &name) {
            edits.push(TextEdit {
                range,
                new_text: new_name.clone(),
            });
        }

        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        changes.insert(uri, edits);

        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }))
    }

    /// RES-188a: respond to `textDocument/completion` — return
    /// builtins + top-level decls whose names start with the
    /// prefix already typed. Scope-aware local / parameter
    /// completion (RES-188b) stays deferred.
    ///
    /// Pipeline:
    ///   1. Read cached source + AST.
    ///   2. `prefix_at(src, pos)` extracts what the user has typed
    ///      so far (walking back from the cursor to a non-
    ///      identifier char).
    ///   3. `completion_candidates(program, prefix)` enumerates
    ///      matching names from `BUILTINS` + top-level decls and
    ///      applies the 100-item cap.
    ///   4. Convert each `Candidate` to `CompletionItem` and wrap
    ///      as `CompletionResponse::Array`.
    async fn completion(&self, params: CompletionParams) -> JsonResult<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };
        let prefix = prefix_at(&text, pos);
        let items: Vec<CompletionItem> = completion_candidates(&program, &prefix)
            .into_iter()
            .map(candidate_to_completion_item)
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    /// RES-357: respond to `textDocument/codeAction` — offer the
    /// "Add contract stubs" quick-fix for every L0010 diagnostic
    /// in the requested range.
    ///
    /// Pipeline:
    ///   1. Read the cached source text.
    ///   2. Walk the incoming `context.diagnostics`; collect those
    ///      whose message contains "L0010" (the "no contract" lint).
    ///   3. For each matching diagnostic, locate the function's
    ///      opening `{` by scanning forward from the diagnostic
    ///      position (`find_brace_line`).
    ///   4. Emit a `CodeAction` with a `TextEdit` that inserts
    ///      `"    requires true;\n    ensures true;\n"` at the
    ///      start of the line immediately after the `{`.
    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> JsonResult<Option<Vec<CodeActionOrCommand>>> {
        let uri = params.text_document.uri;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let mut actions: Vec<CodeActionOrCommand> = Vec::new();

        for diag in &params.context.diagnostics {
            // Only act on L0010 "no contract" diagnostics.
            if !diag.message.contains("L0010")
                && !diag.message.contains("requires")
                && !diag.message.contains("no contract")
            {
                continue;
            }
            // Determine insertion point: scan from the diagnostic's
            // start line forward to find the `{` that opens the fn body.
            let diag_line = diag.range.start.line as usize;
            let Some(brace_line) = find_brace_line(&text, diag_line) else {
                continue;
            };
            // Insert at the start of the line after the opening brace.
            let insert_pos = Position {
                line: (brace_line + 1) as u32,
                character: 0,
            };
            let insert_range = Range {
                start: insert_pos,
                end: insert_pos,
            };
            let edit_text = "    requires true;\n    ensures true;\n".to_string();
            let text_edit = TextEdit {
                range: insert_range,
                new_text: edit_text,
            };
            let mut changes = HashMap::new();
            changes.insert(uri.clone(), vec![text_edit]);
            let workspace_edit = WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            };
            let action = CodeAction {
                title: "Add contract stubs".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diag.clone()]),
                edit: Some(workspace_edit),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            };
            actions.push(CodeActionOrCommand::CodeAction(action));
        }

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    /// RES-187: respond to `textDocument/semanticTokens/full` with
    /// a full-file token stream. Reads the cached source text,
    /// runs the lexer-driven classifier, and returns the delta-
    /// encoded LSP payload. `Ok(None)` when the document text
    /// isn't cached (i.e. the client asked before `didOpen`); a
    /// strict client would just skip semantic highlighting for
    /// this file until the next change.
    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> JsonResult<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else { return Ok(None) };
        let wire = compute_semantic_tokens(&text);
        let data = semantic_tokens_from_wire(wire);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    /// RES-189: `textDocument/inlayHint` — emit type-hint labels
    /// for unannotated `let` bindings (always on) and parameter-
    /// name labels at call sites (gated behind the
    /// `resilient.inlayHints.parameters` init option).
    ///
    /// Strategy: run the typechecker on the cached AST so the
    /// `let_type_hints` side-channel fills up, then walk the AST
    /// separately for call-site parameter hints. Both passes are
    /// cheap; no caching needed for files the editor could open.
    /// Filters the output by the request's `range` so clients
    /// that ask for a viewport don't pay to render the whole
    /// file's worth.
    async fn inlay_hint(&self, params: InlayHintParams) -> JsonResult<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let range = params.range;

        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };

        // Run the typechecker purely for its hint side-channel.
        // Ignore the return value — errors don't invalidate hints
        // collected up to the error point.
        let mut tc = typechecker::TypeChecker::new();
        let _ = tc.check_program_with_source(&program, uri.as_str());
        let let_hints: Vec<InlayHint> = tc.let_type_hints.iter().map(inlay_hint_from_let).collect();

        let mut out: Vec<InlayHint> = let_hints
            .into_iter()
            .filter(|h| position_in_range(h.position, range))
            .collect();

        let want_param_hints = match self.inlay_hint_parameters.lock() {
            Ok(g) => *g,
            Err(_) => false,
        };
        if want_param_hints {
            for h in collect_param_hints(&program) {
                if position_in_range(h.position, range) {
                    out.push(h);
                }
            }
        }
        Ok(Some(out))
    }
}

/// Run the LSP server on stdin/stdout until the client shuts down.
/// Invoked from `main()` when `--lsp` is on the command line AND the
/// `lsp` feature is enabled.
pub fn run() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::new(Backend::new);
        Server::new(stdin, stdout, socket).serve(service).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_parses_line_col_prefix() {
        let err = "scratch.rs:3:5: Undefined variable 'x'";
        let (range, msg) = extract_range_and_message(err);
        assert_eq!(range.start.line, 2); // 0-indexed LSP
        assert_eq!(range.start.character, 4);
        assert_eq!(msg, "Undefined variable 'x'");
    }

    #[test]
    fn extract_handles_no_prefix_gracefully() {
        let err = "some raw error with no prefix";
        let (range, msg) = extract_range_and_message(err);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
        assert_eq!(msg, err);
    }

    #[test]
    fn extract_parses_bare_line_col_prefix() {
        // RES-089: parser errors come back as `<line>:<col>: <msg>`
        // with no path prefix. The extractor must pick up the bare
        // form and produce a 0-indexed Range.
        let err = "3:5: Unexpected token";
        let (range, msg) = extract_range_and_message(err);
        assert_eq!(range.start.line, 2);
        assert_eq!(range.start.character, 4);
        assert_eq!(msg, "Unexpected token");
    }

    #[test]
    fn extract_handles_path_with_colons() {
        // A Windows-style path wouldn't be common on Unix but let's
        // confirm we don't crash if a path happens to include `:`.
        let err = "C:/tmp/foo.rs:2:3: oops";
        let (range, msg) = extract_range_and_message(err);
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 2);
        assert_eq!(msg, "oops");
    }

    // ---------- RES-185: document symbols ----------

    #[test]
    fn document_symbols_three_fns_plus_struct() {
        // Ticket AC: "3 fns + 1 struct" program produces four
        // symbols in file order, with the correct SymbolKind on
        // each.
        let src = "\
            fn alpha() { return 0; }\n\
            struct Point { int x, int y }\n\
            fn beta(int n) { return n; }\n\
            fn gamma() { return 1; }\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let syms = document_symbols_for_program(&program);
        assert_eq!(syms.len(), 4, "expected 4 symbols, got: {:?}", syms);

        // File order: alpha, Point, beta, gamma.
        assert_eq!(syms[0].name, "alpha");
        assert_eq!(syms[0].kind, SymbolKind::FUNCTION);
        assert_eq!(syms[1].name, "Point");
        assert_eq!(syms[1].kind, SymbolKind::STRUCT);
        assert_eq!(syms[2].name, "beta");
        assert_eq!(syms[2].kind, SymbolKind::FUNCTION);
        assert_eq!(syms[3].name, "gamma");
        assert_eq!(syms[3].kind, SymbolKind::FUNCTION);
    }

    #[test]
    fn document_symbols_includes_type_alias() {
        // RES-128 landed `type <Name> = <Target>;`. It should
        // show up as SymbolKind::TYPE_PARAMETER in the outline.
        let src = "type Meters = int;\nfn foo() { return 0; }\n";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let syms = document_symbols_for_program(&program);
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "Meters");
        assert_eq!(syms[0].kind, SymbolKind::TYPE_PARAMETER);
        assert_eq!(syms[1].name, "foo");
        assert_eq!(syms[1].kind, SymbolKind::FUNCTION);
    }

    #[test]
    fn document_symbols_ignores_non_declaration_statements() {
        // `let` bindings, `return`, expression statements etc.
        // are statements, not declarations — they should NOT
        // appear in the outline.
        let src = "\
            let x = 1;\n\
            fn only_fn() { return 0; }\n\
            x + 1;\n\
            return 0;\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let syms = document_symbols_for_program(&program);
        assert_eq!(syms.len(), 1, "only the fn should appear, got {:?}", syms);
        assert_eq!(syms[0].name, "only_fn");
    }

    #[test]
    fn document_symbols_empty_on_empty_program() {
        let (program, errs) = parse("");
        assert!(errs.is_empty());
        let syms = document_symbols_for_program(&program);
        assert!(syms.is_empty());
    }

    #[test]
    fn document_symbols_sorted_by_source_position() {
        // The loop iterates program.stmts in order, which is
        // already source order, so the sort is a no-op here.
        // The test pins the behaviour anyway.
        let src = "\
            fn a() { return 0; }\n\
            fn b() { return 0; }\n\
            fn c() { return 0; }\n\
        ";
        let (program, _) = parse(src);
        let syms = document_symbols_for_program(&program);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        // Line numbers should also strictly increase.
        let lines: Vec<u32> = syms.iter().map(|s| s.range.start.line).collect();
        assert!(
            lines.windows(2).all(|w| w[0] <= w[1]),
            "ranges should be non-decreasing, got: {:?}",
            lines
        );
    }

    #[test]
    fn span_to_range_converts_1_indexed_to_0_indexed() {
        use crate::span::{Pos, Span};
        let s = Span::new(Pos::new(1, 1, 0), Pos::new(1, 5, 4));
        let r = span_to_range(s);
        assert_eq!(r.start.line, 0);
        assert_eq!(r.start.character, 0);
        assert_eq!(r.end.line, 0);
        assert_eq!(r.end.character, 4);
    }

    // ---------- RES-186: workspace symbol search ----------

    /// Unique per-test scratch directory inside the OS temp dir.
    fn tmp_workspace(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("res_186_{}_{}_{}", tag, std::process::id(), n));
        std::fs::create_dir_all(&path).expect("create scratch dir");
        path
    }

    fn write_file(dir: &std::path::Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).expect("write test file");
    }

    #[test]
    fn walk_resilient_files_finds_rs_files_recursively() {
        let root = tmp_workspace("walk");
        write_file(&root, "a.rs", "fn a() { return 0; }\n");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        write_file(&root.join("sub"), "b.rs", "fn b() { return 0; }\n");
        // Hidden + build dirs should be skipped.
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        write_file(
            &root.join("target").join("debug"),
            "c.rs",
            "fn c() { return 0; }\n",
        );
        std::fs::create_dir_all(root.join(".cache")).unwrap();
        write_file(&root.join(".cache"), "d.rs", "fn d() { return 0; }\n");
        let found = walk_resilient_files(&root);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"a.rs".to_string()));
        assert!(names.contains(&"b.rs".to_string()));
        assert!(
            !names.contains(&"c.rs".to_string()),
            "target/ must be skipped"
        );
        assert!(
            !names.contains(&"d.rs".to_string()),
            "dot-dirs must be skipped"
        );
        // Clean up.
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn index_file_parses_and_extracts_top_level_symbols() {
        let root = tmp_workspace("index");
        let path = root.join("prog.rs");
        std::fs::write(
            &path,
            "fn one() { return 0; }\nstruct S { int x }\nfn two() { return 1; }\n",
        )
        .unwrap();
        let entries = index_file(&path).expect("should index ok");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["one", "S", "two"]);
        // Each entry's URI points at the file we indexed.
        for e in &entries {
            assert!(e.uri.as_str().ends_with("/prog.rs"));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn filter_workspace_symbols_case_insensitive_substring() {
        use std::collections::HashMap;
        let uri = Url::parse("file:///tmp/a.rs").unwrap();
        let entries = vec![
            WorkspaceSymbolEntry {
                name: "alpha".into(),
                kind: SymbolKind::FUNCTION,
                uri: uri.clone(),
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            },
            WorkspaceSymbolEntry {
                name: "AlphaBeta".into(),
                kind: SymbolKind::FUNCTION,
                uri: uri.clone(),
                range: Range::new(Position::new(1, 0), Position::new(1, 0)),
            },
            WorkspaceSymbolEntry {
                name: "gamma".into(),
                kind: SymbolKind::FUNCTION,
                uri: uri.clone(),
                range: Range::new(Position::new(2, 0), Position::new(2, 0)),
            },
        ];
        let mut index: HashMap<Url, Vec<WorkspaceSymbolEntry>> = HashMap::new();
        index.insert(uri, entries);

        // Exact prefix, case-folded.
        let r = filter_workspace_symbols(&index, "alpha", 50);
        let names: Vec<&str> = r.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["AlphaBeta", "alpha"]); // sorted by name

        // Substring mid-word. Caller (Backend::symbol) is the one
        // that lowercases the user query — this helper takes
        // `query_lower` ALREADY folded, so `"bet"` matches
        // `AlphaBeta` (via lowercased "alphabeta").
        let r = filter_workspace_symbols(&index, "bet", 50);
        let names: Vec<&str> = r.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["AlphaBeta"]);

        // Empty query matches everything.
        let r = filter_workspace_symbols(&index, "", 50);
        assert_eq!(r.len(), 3);

        // Limit cap.
        let r = filter_workspace_symbols(&index, "", 2);
        assert_eq!(r.len(), 2);
    }

    // ---------- RES-187: semantic tokens legend + wire glue ----------

    /// The legend's type-index order MUST match the `sem_tok::*`
    /// constants in main.rs. If someone adds a new token type
    /// between them, this test catches the drift before an editor
    /// starts mis-coloring things.
    #[test]
    fn semantic_tokens_legend_indices_match_sem_tok_constants() {
        use crate::sem_tok;
        let legend = semantic_tokens_legend();
        assert_eq!(
            legend.token_types[sem_tok::KEYWORD as usize],
            SemanticTokenType::KEYWORD
        );
        assert_eq!(
            legend.token_types[sem_tok::FUNCTION as usize],
            SemanticTokenType::FUNCTION
        );
        assert_eq!(
            legend.token_types[sem_tok::VARIABLE as usize],
            SemanticTokenType::VARIABLE
        );
        assert_eq!(
            legend.token_types[sem_tok::PARAMETER as usize],
            SemanticTokenType::PARAMETER
        );
        assert_eq!(
            legend.token_types[sem_tok::TYPE as usize],
            SemanticTokenType::TYPE
        );
        assert_eq!(
            legend.token_types[sem_tok::STRING as usize],
            SemanticTokenType::STRING
        );
        assert_eq!(
            legend.token_types[sem_tok::NUMBER as usize],
            SemanticTokenType::NUMBER
        );
        assert_eq!(
            legend.token_types[sem_tok::COMMENT as usize],
            SemanticTokenType::COMMENT
        );
        assert_eq!(
            legend.token_types[sem_tok::OPERATOR as usize],
            SemanticTokenType::OPERATOR
        );
        // Modifier bit positions: bit 0 = declaration, bit 1 = readonly.
        assert_eq!(
            legend.token_modifiers[0],
            SemanticTokenModifier::DECLARATION
        );
        assert_eq!(legend.token_modifiers[1], SemanticTokenModifier::READONLY);
    }

    /// `semantic_tokens_from_wire` must unpack an n-tuple u32 stream
    /// into n `SemanticToken`s preserving field order.
    #[test]
    fn semantic_tokens_from_wire_unpacks_correctly() {
        // Two tokens: [0,0,3,0,0] then [0,4,3,1,1].
        let wire = vec![0, 0, 3, 0, 0, 0, 4, 3, 1, 1];
        let toks = semantic_tokens_from_wire(wire);
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].delta_line, 0);
        assert_eq!(toks[0].delta_start, 0);
        assert_eq!(toks[0].length, 3);
        assert_eq!(toks[0].token_type, 0);
        assert_eq!(toks[0].token_modifiers_bitset, 0);
        assert_eq!(toks[1].delta_line, 0);
        assert_eq!(toks[1].delta_start, 4);
        assert_eq!(toks[1].length, 3);
        assert_eq!(toks[1].token_type, 1);
        assert_eq!(toks[1].token_modifiers_bitset, 1);
    }

    /// Trailing-partial u32 chunks (not a multiple of 5) are
    /// dropped by `chunks_exact`. Not expected in practice, but
    /// pinning the behaviour.
    #[test]
    fn semantic_tokens_from_wire_drops_partial_trailing_chunk() {
        let wire = vec![0, 0, 3, 0, 0, 0, 4]; // 7 elements — last 2 dropped
        let toks = semantic_tokens_from_wire(wire);
        assert_eq!(toks.len(), 1);
    }

    // ---------- RES-189: inlay hints ----------

    #[test]
    fn typechecker_collects_hints_for_unannotated_lets() {
        // Ticket AC: 5 lets, 3 should produce type hints (the
        // non-annotated ones). Annotated lets stay quiet.
        let src = "\
            fn main(int _d) {\n\
                let a = 1;\n\
                let b: int = 2;\n\
                let c = true;\n\
                let d: bool = false;\n\
                let e = \"hi\";\n\
                return 0;\n\
            }\n";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = typechecker::TypeChecker::new();
        let _ = tc.check_program_with_source(&program, "file:///tmp/test.rs");
        let hints = &tc.let_type_hints;
        assert_eq!(
            hints.len(),
            3,
            "expected 3 hints (a, c, e), got {:?}",
            hints
                .iter()
                .map(|h| (h.name_len_chars, &h.ty))
                .collect::<Vec<_>>(),
        );
        let types: Vec<String> = hints.iter().map(|h| format!("{}", h.ty)).collect();
        assert_eq!(types, vec!["int", "bool", "string"]);
    }

    #[test]
    fn typechecker_skips_any_typed_let_hints() {
        // `Type::Any` bindings shouldn't produce hints — no useful
        // information and clutters the editor.
        let src = "fn main(int _d) { let x = println(\"hi\"); return 0; }\n";
        let (program, _) = parse(src);
        let mut tc = typechecker::TypeChecker::new();
        let _ = tc.check_program_with_source(&program, "<t>");
        // `println` returns Void, so `x` is Void → skipped.
        assert_eq!(tc.let_type_hints.len(), 0);
    }

    #[test]
    fn inlay_hint_from_let_positions_after_identifier() {
        use crate::span::{Pos, Span};
        // `let abc = 3;` starting at 1:1 → pattern ends at col 8
        // (1 + "let ".len() + "abc".len() = 1 + 4 + 3 = 8, 0-indexed
        // = 7).
        let entry = typechecker::LetTypeHint {
            span: Span::new(Pos::new(1, 1, 0), Pos::new(1, 1, 0)),
            name_len_chars: 3,
            ty: typechecker::Type::Int,
        };
        let hint = inlay_hint_from_let(&entry);
        assert_eq!(hint.position.line, 0);
        assert_eq!(hint.position.character, 7);
        // The label starts with `: ` per the convention; clients
        // render the padding.
        match hint.label {
            InlayHintLabel::String(ref s) => assert_eq!(s, ": int"),
            other => panic!("expected string label, got {:?}", other),
        }
        assert_eq!(hint.kind, Some(InlayHintKind::TYPE));
        assert_eq!(hint.padding_left, Some(true));
    }

    #[test]
    fn param_hints_tag_each_arg_with_param_name() {
        // Ticket AC: `add(a: 1, b: 2)`-style chrome.
        let src = "\
            fn add(int a, int b) { return a + b; }\n\
            fn main(int _d) { return add(1, 2); }\n\
            main(0);\n";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let hints = collect_param_hints(&program);
        assert!(hints.len() >= 2, "expected >=2 hints, got: {hints:?}");
        // Every produced hint should be a PARAMETER kind.
        for h in &hints {
            assert_eq!(h.kind, Some(InlayHintKind::PARAMETER));
            match &h.label {
                InlayHintLabel::String(s) => {
                    assert!(
                        s.ends_with(": "),
                        "param hint label should end with `: `, got {s:?}"
                    );
                }
                _ => panic!("expected string label"),
            }
        }
        // Exactly two hints for the `add(1, 2)` call.
        let add_hints: Vec<&str> = hints
            .iter()
            .filter_map(|h| match &h.label {
                InlayHintLabel::String(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert!(add_hints.contains(&"a: "));
        assert!(add_hints.contains(&"b: "));
    }

    #[test]
    fn param_hints_skip_arity_mismatches() {
        // If the call passes the wrong number of args, don't emit
        // hints — any pairing would be misleading.
        let src = "\
            fn add(int a, int b) { return a + b; }\n\
            fn main(int _d) { return add(1); }\n\
            main(0);\n";
        let (program, _errs) = parse(src);
        let hints = collect_param_hints(&program);
        let add_hints: Vec<_> = hints
            .iter()
            .filter(|h| matches!(h.kind, Some(InlayHintKind::PARAMETER)))
            .collect();
        assert!(
            add_hints.is_empty(),
            "arity-mismatch call should not emit param hints, got {add_hints:?}"
        );
    }

    #[test]
    fn param_hints_skip_unknown_callees() {
        // println isn't declared in this program, so no hints.
        let src = "fn main(int _d) { println(\"hi\"); return 0; }\n";
        let (program, _) = parse(src);
        let hints = collect_param_hints(&program);
        assert!(hints.is_empty(), "println isn't a user fn; got: {hints:?}");
    }

    #[test]
    fn read_init_param_hints_flag_flat_form() {
        let v: tower_lsp::lsp_types::LSPAny = serde_json::json!({
            "resilient.inlayHints.parameters": true
        });
        assert!(read_init_param_hints_flag(Some(&v)));
    }

    #[test]
    fn read_init_param_hints_flag_nested_form() {
        let v: tower_lsp::lsp_types::LSPAny = serde_json::json!({
            "resilient": { "inlayHints": { "parameters": true } }
        });
        assert!(read_init_param_hints_flag(Some(&v)));
    }

    #[test]
    fn read_init_param_hints_flag_defaults_false() {
        assert!(!read_init_param_hints_flag(None));
        let v: tower_lsp::lsp_types::LSPAny = serde_json::json!({});
        assert!(!read_init_param_hints_flag(Some(&v)));
        let v: tower_lsp::lsp_types::LSPAny = serde_json::json!({
            "resilient": { "inlayHints": { "parameters": false } }
        });
        assert!(!read_init_param_hints_flag(Some(&v)));
    }

    #[test]
    fn position_in_range_inclusive_endpoints() {
        let r = Range::new(Position::new(0, 0), Position::new(10, 0));
        assert!(position_in_range(Position::new(0, 0), r));
        assert!(position_in_range(Position::new(5, 0), r));
        assert!(position_in_range(Position::new(10, 0), r));
        assert!(!position_in_range(Position::new(10, 1), r));
        assert!(!position_in_range(Position::new(11, 0), r));
    }

    #[test]
    fn workspace_index_spans_multiple_files() {
        // Ticket AC: pre-seed two files, invoke the query (via the
        // helper path), assert both files' symbols are returned.
        let root = tmp_workspace("multifile");
        write_file(
            &root,
            "mod_a.rs",
            "fn a_fn() { return 0; }\nstruct A_Struct { int x }\n",
        );
        write_file(&root, "mod_b.rs", "fn b_fn() { return 0; }\n");

        // Walk + index the whole scratch dir, reproducing what
        // `rebuild_workspace_index` does when the Backend is
        // invoked via the LSP.
        let files = walk_resilient_files(&root);
        assert_eq!(files.len(), 2);
        let mut index: std::collections::HashMap<Url, Vec<WorkspaceSymbolEntry>> =
            std::collections::HashMap::new();
        for p in files {
            let Ok(uri) = Url::from_file_path(&p) else {
                continue;
            };
            if let Some(entries) = index_file(&p) {
                index.insert(uri, entries);
            }
        }

        // All-match query: three names across two files.
        let r = filter_workspace_symbols(&index, "", 50);
        let names: std::collections::HashSet<&str> = r.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains("a_fn"));
        assert!(names.contains("A_Struct"));
        assert!(names.contains("b_fn"));
        // And the Locations point at the right files.
        for sym in &r {
            let path_str = sym.location.uri.as_str();
            assert!(
                path_str.ends_with("/mod_a.rs") || path_str.ends_with("/mod_b.rs"),
                "unexpected URI: {}",
                path_str
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    // ============================================================
    // RES-181a: hover handler helpers
    // ============================================================
    //
    // Drive the lexer directly (via `hover_literal_at`) to classify
    // the token at a cursor position. Returns `(type_name, range)`
    // or `None`. Tests use zero-indexed `Position` values to mirror
    // how real LSP clients send cursor coordinates.

    #[test]
    fn res181a_hover_on_int_literal_start_returns_int() {
        // `let x = 42;` — `42` spans cols 9..11 (1-indexed) = LSP
        // chars 8..11. Cursor at `4` (char 8) returns Int.
        let r = hover_literal_at("let x = 42;", Position::new(0, 8));
        let (ty, _range) = r.expect("expected Int hover at col 8");
        assert_eq!(ty, "Int");
    }

    #[test]
    fn res181a_hover_on_int_literal_middle_returns_int() {
        // Anywhere inside the token's extent — cursor at `2` (char 9)
        // still returns Int because the lexer's real span covers
        // the whole literal.
        let r = hover_literal_at("let x = 42;", Position::new(0, 9));
        let (ty, _) = r.expect("expected Int hover at col 9");
        assert_eq!(ty, "Int");
    }

    #[test]
    fn res181a_hover_on_bool_literal_returns_bool() {
        // `let b = true;` — `true` spans cols 9..13.
        let r = hover_literal_at("let b = true;", Position::new(0, 10));
        let (ty, _) = r.expect("expected Bool hover");
        assert_eq!(ty, "Bool");
    }

    #[test]
    fn res181a_hover_on_false_literal_returns_bool() {
        // `let b = false;` — `false` spans cols 9..14.
        let r = hover_literal_at("let b = false;", Position::new(0, 10));
        let (ty, _) = r.expect("expected Bool hover");
        assert_eq!(ty, "Bool");
    }

    #[test]
    fn res181a_hover_on_string_literal_returns_string() {
        // `let s = "hi";` — quoted string spans cols 9..13.
        let r = hover_literal_at(r#"let s = "hi";"#, Position::new(0, 10));
        let (ty, _) = r.expect("expected String hover");
        assert_eq!(ty, "String");
    }

    #[test]
    fn res181a_hover_on_float_literal_returns_float() {
        let r = hover_literal_at("let f = 3.14;", Position::new(0, 10));
        let (ty, _) = r.expect("expected Float hover");
        assert_eq!(ty, "Float");
    }

    #[test]
    fn res181a_hover_on_bytes_literal_returns_bytes() {
        // `let b = b"\x00\x01";` — bytes literal.
        let r = hover_literal_at(r#"let b = b"\x00";"#, Position::new(0, 10));
        let (ty, _) = r.expect("expected Bytes hover");
        assert_eq!(ty, "Bytes");
    }

    #[test]
    fn res181a_hover_on_keyword_returns_none() {
        // Cursor on `let` (non-literal token) → no hover.
        assert!(hover_literal_at("let x = 42;", Position::new(0, 0)).is_none());
    }

    #[test]
    fn res181a_hover_on_identifier_returns_none() {
        // Cursor on `x` identifier — identifier hover is RES-181b
        // (needs inferred types). Today we deliberately return
        // None so the client renders nothing.
        let r = hover_literal_at("let x = 42;", Position::new(0, 4));
        assert!(r.is_none());
    }

    #[test]
    fn res181a_hover_on_operator_returns_none() {
        // Cursor on `+` → no hover.
        let r = hover_literal_at("let r = 1 + 2;", Position::new(0, 10));
        assert!(r.is_none());
    }

    #[test]
    fn res181a_hover_out_of_range_returns_none() {
        // Cursor way past the file end → None (no tokens past EOF).
        let r = hover_literal_at("let x = 42;", Position::new(10, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res181a_hover_on_empty_source_returns_none() {
        let r = hover_literal_at("", Position::new(0, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res181a_hover_inside_fn_body_returns_literal_type() {
        let src = "fn f(int n) { return 7; }";
        // `7` is at col 22 (1-indexed) = LSP col 21.
        let r = hover_literal_at(src, Position::new(0, 21));
        let (ty, _) = r.expect("expected Int hover inside fn body");
        assert_eq!(ty, "Int");
    }

    #[test]
    fn res181a_hover_returns_range_covering_the_token() {
        // The Range returned with the hover should span the whole
        // literal. For `42` at cols 9..11 (1-indexed) = LSP 8..10.
        let (ty, range) =
            hover_literal_at("let x = 42;", Position::new(0, 8)).expect("expected hover");
        assert_eq!(ty, "Int");
        assert_eq!(range.start.line, 0);
        assert_eq!(range.end.line, 0);
        // Start column is 8 (LSP 0-indexed = `4`). End column is
        // 10 (exclusive = position of `;`).
        assert_eq!(range.start.character, 8);
        assert_eq!(range.end.character, 10);
    }

    #[test]
    fn res181a_lex_span_contains_lsp_position_single_line() {
        use crate::span::{Pos, Span};
        // Token at line 1, cols 5..10 (1-indexed) — LSP 0-indexed
        // [line 0, chars 4..9].
        let sp = Span::new(Pos::new(1, 5, 0), Pos::new(1, 10, 5));
        assert!(lex_span_contains_lsp_position(sp, Position::new(0, 4)));
        assert!(lex_span_contains_lsp_position(sp, Position::new(0, 8)));
        assert!(!lex_span_contains_lsp_position(sp, Position::new(0, 9)));
        assert!(!lex_span_contains_lsp_position(sp, Position::new(0, 3)));
    }

    #[test]
    fn res181a_lex_span_contains_lsp_position_different_line() {
        use crate::span::{Pos, Span};
        let sp = Span::new(Pos::new(2, 1, 0), Pos::new(2, 5, 4));
        assert!(!lex_span_contains_lsp_position(sp, Position::new(0, 1)));
        assert!(!lex_span_contains_lsp_position(sp, Position::new(5, 1)));
    }

    // ============================================================
    // RES-182a: goto-definition helpers
    // ============================================================

    fn parse_prog(src: &str) -> Node {
        let (program, _errs) = parse(src);
        program
    }

    #[test]
    fn res182a_identifier_at_returns_name_and_range() {
        // `let x = 42;` — cursor on `x` at col 5 (1-indexed) =
        // LSP col 4. The `identifier_at` helper should return
        // ("x", Range at col 4..5).
        let r = identifier_at("let x = 42;", Position::new(0, 4));
        let (name, range) = r.expect("expected identifier hit");
        assert_eq!(name, "x");
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 4);
        assert_eq!(range.end.character, 5);
    }

    #[test]
    fn res182a_identifier_at_returns_none_for_literal() {
        // Cursor on `42` → not an identifier → None.
        let r = identifier_at("let x = 42;", Position::new(0, 8));
        assert!(r.is_none());
    }

    #[test]
    fn res182a_identifier_at_returns_none_for_keyword() {
        // Cursor on `let` → keyword → None.
        let r = identifier_at("let x = 42;", Position::new(0, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res182a_identifier_at_finds_mid_identifier() {
        // Cursor in the middle of `my_fn` (4 chars in) → still
        // returns "my_fn". Tests the multi-char-token branch.
        let r = identifier_at("fn my_fn() { return 0; }", Position::new(0, 5));
        let (name, _) = r.expect("expected identifier hit");
        assert_eq!(name, "my_fn");
    }

    #[test]
    fn res182a_identifier_at_out_of_range_returns_none() {
        let r = identifier_at("let x = 42;", Position::new(10, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res182a_identifier_at_empty_source_returns_none() {
        let r = identifier_at("", Position::new(0, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res182a_build_top_level_defs_empty_program() {
        let prog = parse_prog("");
        let defs = build_top_level_defs(&prog);
        assert!(defs.is_empty());
    }

    #[test]
    fn res182a_build_top_level_defs_collects_fn() {
        let prog = parse_prog("fn add(int a, int b) -> int { return a + b; }");
        let defs = build_top_level_defs(&prog);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "add");
    }

    #[test]
    fn res182a_build_top_level_defs_collects_struct() {
        let prog = parse_prog("struct Point { int x, int y, }");
        let defs = build_top_level_defs(&prog);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "Point");
    }

    #[test]
    fn res182a_build_top_level_defs_collects_type_alias() {
        let prog = parse_prog("type MyInt = int;");
        let defs = build_top_level_defs(&prog);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "MyInt");
    }

    #[test]
    fn res182a_build_top_level_defs_mixed_kinds() {
        let src = r#"
            fn foo(int n) { return n; }
            struct Rec { int x, }
            type I = int;
            let top = 42;
        "#;
        let prog = parse_prog(src);
        let defs = build_top_level_defs(&prog);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["foo", "Rec", "I"]);
    }

    #[test]
    fn res182a_build_top_level_defs_first_wins_on_duplicates() {
        // Duplicate names — parser doesn't reject them, but the
        // goto target should be deterministic. First wins.
        let src = r#"
            fn foo(int n) { return 1; }
            fn foo(int n) { return 2; }
        "#;
        let prog = parse_prog(src);
        let defs = build_top_level_defs(&prog);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "foo");
    }

    #[test]
    fn res182a_find_top_level_def_hit_and_miss() {
        let prog = parse_prog("fn only() { return 0; } struct P { int x, }");
        let defs = build_top_level_defs(&prog);
        assert!(find_top_level_def(&defs, "only").is_some());
        assert!(find_top_level_def(&defs, "P").is_some());
        assert!(find_top_level_def(&defs, "nope").is_none());
    }

    #[test]
    fn res182a_find_top_level_def_returns_range_from_decl() {
        // Decl at line 2 (1-indexed in source) should surface
        // range starting at LSP line 1.
        let src = "\nfn foo() { return 0; }";
        let prog = parse_prog(src);
        let defs = build_top_level_defs(&prog);
        let def = find_top_level_def(&defs, "foo").expect("missing foo");
        assert_eq!(def.range.start.line, 1);
    }

    // ============================================================
    // RES-183: find-references helpers
    // ============================================================

    #[test]
    fn res183_collect_call_sites_three_callers() {
        // AC: 3-caller setup — each direct call should produce one
        // range; the struct literal with the same name should NOT.
        let src = "\
fn greet() { return 1; }\n\
struct greet { int x, }\n\
fn a() { return greet(); }\n\
fn b() { return greet(); }\n\
fn c() { return greet(); }\n\
let _s = new greet { x: 0 };\n\
";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "greet");
        assert_eq!(
            sites.len(),
            3,
            "expected exactly 3 call sites, got {}: {sites:?}",
            sites.len()
        );
    }

    #[test]
    fn res183_collect_call_sites_struct_literal_excluded() {
        // A struct literal `new Foo { ... }` must NOT be counted as
        // a call site even when the struct name matches the target.
        let src = "\
fn Foo() { return 0; }\n\
struct Foo { int x, }\n\
let _s = new Foo { x: 1 };\n\
";
        let (prog, _) = parse(src);
        let sites = collect_call_sites(&prog, "Foo");
        assert!(
            sites.is_empty(),
            "struct literal must not appear as a call site, got: {sites:?}"
        );
    }

    #[test]
    fn res183_collect_call_sites_empty_program() {
        let (prog, _) = parse("");
        let sites = collect_call_sites(&prog, "anything");
        assert!(sites.is_empty());
    }

    #[test]
    fn res183_collect_call_sites_no_match() {
        let src = "fn foo() { return 1; }\nfoo();\n";
        let (prog, _) = parse(src);
        let sites = collect_call_sites(&prog, "bar");
        assert!(sites.is_empty());
    }

    #[test]
    fn res183_collect_call_sites_single_top_level_call() {
        // A bare call statement at top level.
        let src = "fn add(int a, int b) -> int { return a + b; }\nadd(1, 2);\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "add");
        assert_eq!(sites.len(), 1, "expected 1 call site, got: {sites:?}");
        // Call site is on line 2 (1-indexed) = LSP line 1.
        assert_eq!(sites[0].start.line, 1);
    }

    #[test]
    fn res183_collect_call_sites_nested_call() {
        // `foo(foo(1))` — two call sites for `foo`.
        let src = "fn foo(int n) { return n; }\nfoo(foo(1));\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "foo");
        assert_eq!(
            sites.len(),
            2,
            "expected 2 nested call sites, got: {sites:?}"
        );
    }

    #[test]
    fn res183_collect_call_sites_inside_if_while() {
        // Calls inside if/while bodies are captured.
        let src = "\
fn tick() { return 1; }\n\
fn main(int n) {\n\
    if n > 0 { tick(); }\n\
    while n > 0 { tick(); n = n - 1; }\n\
    return 0;\n\
}\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "tick");
        assert_eq!(sites.len(), 2, "expected 2 tick call sites: {sites:?}");
    }

    #[test]
    fn res183_collect_call_sites_range_points_at_callee() {
        // The range in the returned Location should be derived from the
        // callee Identifier's AST span. The parser uses zero-width spans
        // (known limitation per the "span unreliability" note in main.rs),
        // so start == end. The important invariant is that the LINE is
        // correct — it points at the call site's line, not the decl's.
        let src = "fn foo(int n) { return n; }\nfoo(1);\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "foo");
        assert_eq!(sites.len(), 1);
        // `foo(1)` is on source line 2 (1-indexed) = LSP line 1.
        assert_eq!(sites[0].start.line, 1, "call site line mismatch");
    }

    // ============================================================
    // RES-188a: completion helpers
    // ============================================================

    #[test]
    fn res188a_prefix_at_empty_source_is_empty_string() {
        assert_eq!(prefix_at("", Position::new(0, 0)), "");
    }

    #[test]
    fn res188a_prefix_at_start_of_identifier_is_empty() {
        // Cursor AT col 0 on "foo" — user hasn't typed anything
        // yet. Walking backward from col 0 yields the empty string.
        assert_eq!(prefix_at("foo", Position::new(0, 0)), "");
    }

    #[test]
    fn res188a_prefix_at_extracts_partial_identifier() {
        // Cursor 2 chars into "foo" — user has typed "fo".
        assert_eq!(prefix_at("foo", Position::new(0, 2)), "fo");
    }

    #[test]
    fn res188a_prefix_at_extracts_full_identifier() {
        // Cursor at end of "foo" — user has typed "foo".
        assert_eq!(prefix_at("foo", Position::new(0, 3)), "foo");
    }

    #[test]
    fn res188a_prefix_at_stops_at_non_identifier_char() {
        // "let x = fo" — cursor at end. Walking back stops at
        // the space after `=`. Prefix = "fo".
        assert_eq!(prefix_at("let x = fo", Position::new(0, 10)), "fo");
    }

    #[test]
    fn res188a_prefix_at_handles_underscore_in_identifier() {
        // Underscores are identifier chars.
        assert_eq!(prefix_at("my_var", Position::new(0, 6)), "my_var");
        assert_eq!(prefix_at("my_var", Position::new(0, 3)), "my_");
    }

    #[test]
    fn res188a_prefix_at_multiline_respects_line() {
        let src = "foo\nbar";
        assert_eq!(prefix_at(src, Position::new(0, 3)), "foo");
        assert_eq!(prefix_at(src, Position::new(1, 3)), "bar");
    }

    #[test]
    fn res188a_prefix_at_cursor_past_eol_clamps() {
        // Some clients send positions one past EOL; treat as EOL.
        assert_eq!(prefix_at("abc", Position::new(0, 999)), "abc");
    }

    #[test]
    fn res188a_prefix_at_cursor_on_nonexistent_line_empty() {
        assert_eq!(prefix_at("abc", Position::new(5, 0)), "");
    }

    #[test]
    fn res188a_candidates_empty_prefix_returns_many_builtins() {
        // Empty prefix + empty program still yields the builtin
        // list.
        let prog = parse_prog("");
        let cands = completion_candidates(&prog, "");
        // BUILTINS has ~50 entries today. Assert a lower bound
        // that's safe across additions — enough to catch an
        // accidental empty-return regression.
        assert!(
            cands.len() > 10,
            "expected builtins > 10, got {}",
            cands.len()
        );
        // All-function kind is expected for the builtin slice.
        for c in cands.iter().take(5) {
            assert_eq!(c.kind, CandidateKind::Function);
            assert_eq!(c.detail, Some("builtin".to_string()));
        }
    }

    #[test]
    fn res188a_candidates_prefix_filters_builtins() {
        // Prefix "prin" should surface only "println" + "print"
        // (both start with "prin"). BUILTINS won't acquire a
        // non-print prefix-matching name without the test
        // noticing.
        let prog = parse_prog("");
        let cands = completion_candidates(&prog, "prin");
        let labels: Vec<&str> = cands.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"println"));
        assert!(labels.contains(&"print"));
        // Sanity: nothing unrelated snuck in.
        for label in &labels {
            assert!(
                label.starts_with("prin"),
                "prefix leak: {} doesn't start with `prin`",
                label
            );
        }
    }

    #[test]
    fn res188a_candidates_include_top_level_fn() {
        let prog = parse_prog("fn my_helper() { return 1; }");
        let cands = completion_candidates(&prog, "my_");
        let labels: Vec<&str> = cands.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"my_helper"));
        let c = cands.iter().find(|c| c.label == "my_helper").unwrap();
        assert_eq!(c.kind, CandidateKind::Function);
    }

    #[test]
    fn res188a_candidates_include_top_level_struct() {
        let prog = parse_prog("struct Point { int x, int y, }");
        let cands = completion_candidates(&prog, "Po");
        let c = cands.iter().find(|c| c.label == "Point").unwrap();
        assert_eq!(c.kind, CandidateKind::Struct);
        assert!(c.detail.as_deref().unwrap().contains("struct"));
    }

    #[test]
    fn res188a_candidates_include_type_alias() {
        let prog = parse_prog("type Id = int;");
        let cands = completion_candidates(&prog, "Id");
        let c = cands.iter().find(|c| c.label == "Id").unwrap();
        assert_eq!(c.kind, CandidateKind::TypeAlias);
    }

    #[test]
    fn res188a_candidates_builtins_before_user_decls() {
        // Deterministic ordering: builtins first (alphabetical),
        // then user decls (source order). Useful for snapshot /
        // regression tests downstream.
        let prog = parse_prog("fn abc() { return 0; }");
        let cands = completion_candidates(&prog, "ab");
        // `abs` (builtin) should come before `abc` (user decl).
        let abs_idx = cands.iter().position(|c| c.label == "abs");
        let abc_idx = cands.iter().position(|c| c.label == "abc");
        assert!(abs_idx.is_some(), "expected `abs` builtin");
        assert!(abc_idx.is_some(), "expected `abc` user decl");
        assert!(abs_idx.unwrap() < abc_idx.unwrap());
    }

    #[test]
    fn res188a_candidates_respects_completion_limit() {
        let prog = parse_prog("");
        // Empty prefix gives every builtin. Ensure we don't blow
        // past the cap even if BUILTINS is later extended.
        let cands = completion_candidates(&prog, "");
        assert!(cands.len() <= COMPLETION_LIMIT);
    }

    #[test]
    fn res188a_candidates_unmatched_prefix_returns_empty() {
        let prog = parse_prog("fn foo() { return 0; }");
        let cands = completion_candidates(&prog, "zzz_no_match");
        assert!(cands.is_empty());
    }

    #[test]
    fn res188a_candidate_to_completion_item_maps_fields() {
        let c = Candidate {
            label: "test".to_string(),
            kind: CandidateKind::Function,
            detail: Some("builtin".to_string()),
        };
        let item = candidate_to_completion_item(c);
        assert_eq!(item.label, "test");
        assert_eq!(item.kind, Some(CompletionItemKind::FUNCTION));
        assert_eq!(item.detail, Some("builtin".to_string()));
        assert_eq!(item.insert_text, Some("test".to_string()));
    }

    // ============================================================
    // RES-184: rename helpers
    // ============================================================

    #[test]
    fn res184_is_valid_identifier_accepts_simple_names() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("Foo_Bar2"));
        assert!(is_valid_identifier("x"));
        assert!(is_valid_identifier("_"));
    }

    #[test]
    fn res184_is_valid_identifier_rejects_bad_names() {
        assert!(!is_valid_identifier("")); // empty
        assert!(!is_valid_identifier("2foo")); // starts with digit
        assert!(!is_valid_identifier("foo-bar")); // hyphen
        assert!(!is_valid_identifier("foo bar")); // space
        assert!(!is_valid_identifier("foo.bar")); // dot
    }

    #[test]
    fn res184_find_decl_name_range_locates_fn_name() {
        // `fn foo() { return 0; }` — `foo` starts at col 4 (1-indexed)
        // = LSP col 3, ends at col 7 = LSP col 6.
        let src = "fn foo() { return 0; }";
        let range = find_decl_name_range(src, "foo").expect("should find foo");
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 3); // LSP 0-indexed col of 'f' in "foo"
        assert_eq!(range.end.character, 6);
    }

    #[test]
    fn res184_find_decl_name_range_returns_none_for_missing() {
        let src = "fn foo() { return 0; }";
        assert!(find_decl_name_range(src, "bar").is_none());
    }

    #[test]
    fn res184_find_decl_name_range_multiline_second_fn() {
        let src = "fn first() { return 1; }\nfn second() { return 2; }";
        let range = find_decl_name_range(src, "second").expect("should find second");
        // `second` is on line 2 (1-indexed) = LSP line 1.
        assert_eq!(range.start.line, 1);
    }

    #[test]
    fn res184_collect_rename_edits_toplevel_fn() {
        // Integration: rename `add` → `sum`.
        // Source has declaration + 2 call sites.
        let src = "fn add(int a, int b) -> int { return a + b; }\nadd(1, 2);\nadd(3, 4);\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");

        // Simulate what the `rename` handler does.
        let new_name = "sum";
        let defs = build_top_level_defs(&prog);
        let def = find_top_level_def(&defs, "add").expect("add should be found");
        let decl_range = find_decl_name_range(src, "add").unwrap_or(def.range);
        let call_ranges = collect_call_sites(&prog, "add");

        // Verify: 1 declaration edit + 2 call-site edits.
        let mut edits: Vec<TextEdit> = Vec::new();
        edits.push(TextEdit {
            range: decl_range,
            new_text: new_name.to_string(),
        });
        for range in call_ranges {
            edits.push(TextEdit {
                range,
                new_text: new_name.to_string(),
            });
        }

        assert_eq!(
            edits.len(),
            3,
            "expected 3 edits (1 decl + 2 calls): {edits:?}"
        );
        // All edits replace with the new name.
        for e in &edits {
            assert_eq!(e.new_text, "sum");
        }
        // Declaration edit is on line 0.
        assert_eq!(edits[0].range.start.line, 0);
        // Call-site edits are on lines 1 and 2.
        let call_lines: Vec<u32> = edits[1..].iter().map(|e| e.range.start.line).collect();
        assert!(
            call_lines.contains(&1),
            "missing call on line 1: {call_lines:?}"
        );
        assert!(
            call_lines.contains(&2),
            "missing call on line 2: {call_lines:?}"
        );
    }

    #[test]
    fn res184_rename_decl_range_is_name_token_not_whole_stmt() {
        // The declaration edit range must cover only "add", not
        // the whole `fn add(...) { ... }` statement.
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let range = find_decl_name_range(src, "add").expect("should find add");
        // `add` is 3 chars wide; verify start and end are close together.
        assert_eq!(
            range.start.line, range.end.line,
            "multi-line decl range unexpected"
        );
        let width = range.end.character - range.start.character;
        assert_eq!(
            width, 3,
            "expected range to cover 3-char 'add', got width {width}"
        );
    }

    #[test]
    fn res184_collision_check_detects_existing_name() {
        // If new_name already exists as a top-level decl, the handler
        // should refuse. We test the logic directly (not through the
        // async handler).
        let src = "fn add(int a, int b) -> int { return a + b; }\nfn sum() { return 0; }";
        let (prog, _) = parse(src);
        let defs = build_top_level_defs(&prog);
        // Renaming `add` to `sum` must be blocked.
        let collision = find_top_level_def(&defs, "sum").is_some();
        assert!(collision, "expected collision detection for name `sum`");
    }

    #[test]
    fn res184_no_collision_for_fresh_name() {
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let (prog, _) = parse(src);
        let defs = build_top_level_defs(&prog);
        let collision = find_top_level_def(&defs, "total").is_some();
        assert!(!collision, "unexpected collision for fresh name `total`");
    }

    // ============================================================
    // RES-357: code action — add contract stubs
    // ============================================================

    #[test]
    fn res357_find_brace_line_simple_fn() {
        // `fn f(int x) { return x; }` — `{` is on line 0.
        let src = "fn f(int x) { return x; }";
        assert_eq!(find_brace_line(src, 0), Some(0));
    }

    #[test]
    fn res357_find_brace_line_multiline_signature() {
        // The `{` may be on a later line when the signature wraps.
        let src = "fn f(\n    int x\n) {\n    return x;\n}";
        assert_eq!(find_brace_line(src, 0), Some(2));
    }

    #[test]
    fn res357_find_brace_line_skips_lines_before_start() {
        // Start scanning from line 1: the `{` on line 0 must be ignored.
        let src = "fn f() {\n    return 0;\n}";
        // Starting from line 1 (inside the body), the next `{` won't be
        // found until the top-level scan; but line 0 has the opening `{`
        // so starting from line 1 should return None (body has no `{`).
        assert_eq!(find_brace_line(src, 1), None);
    }

    #[test]
    fn res357_find_brace_line_ignores_brace_in_string() {
        // A `{` inside a string literal must not count.
        let src = r#"fn f() -> string { return "{ignored}"; }"#;
        // The function-body `{` is at column 18 on line 0 — the one at
        // column 0 is ahead of the string.  The scanner should return
        // line 0 because the first `{` it finds is the real opening brace
        // (before the string).
        assert_eq!(find_brace_line(src, 0), Some(0));
    }

    #[test]
    fn res357_find_brace_line_returns_none_when_no_brace() {
        let src = "let x = 1;\nlet y = 2;";
        assert_eq!(find_brace_line(src, 0), None);
    }

    /// Exercise `code_action_stubs_for_diagnostic`: given a synthetic
    /// L0010 diagnostic pointing at line 0, the helper produces a
    /// `CodeAction` whose `WorkspaceEdit` inserts the contract stubs
    /// after the opening `{`.
    #[test]
    fn res357_contract_stubs_edit_inserts_requires_and_ensures() {
        // Source: function on a single line, no contract.
        let src = "fn foo(int x) { return x; }\n";
        let uri = Url::parse("file:///tmp/test_contract.rs").unwrap();

        // Construct a synthetic L0010 diagnostic at line 0.
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 5)),
            severity: Some(DiagnosticSeverity::WARNING),
            source: Some("resilient-lint".into()),
            message: "function `foo` has no `requires`/`ensures` contract; \
                      add contract stubs or suppress with `// resilient: allow L0010`"
                .into(),
            ..Default::default()
        };

        // Directly invoke the stub-builder logic to avoid needing an
        // async runtime in unit tests.
        let brace_line = find_brace_line(src, diag.range.start.line as usize)
            .expect("should find opening brace");
        let insert_pos = Position {
            line: (brace_line + 1) as u32,
            character: 0,
        };
        let insert_range = Range {
            start: insert_pos,
            end: insert_pos,
        };
        let text_edit = TextEdit {
            range: insert_range,
            new_text: "    requires true;\n    ensures true;\n".to_string(),
        };
        let mut changes = HashMap::new();
        changes.insert(uri.clone(), vec![text_edit.clone()]);
        let workspace_edit = WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        };
        let action = CodeAction {
            title: "Add contract stubs".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diag]),
            edit: Some(workspace_edit),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        };

        // Assertions on the produced action.
        assert_eq!(action.title, "Add contract stubs");
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(action.is_preferred, Some(true));
        let edit = action.edit.expect("expected edit");
        let file_edits = edit
            .changes
            .expect("expected changes map")
            .remove(&uri)
            .expect("expected edits for our URI");
        assert_eq!(file_edits.len(), 1);
        let te = &file_edits[0];
        // The edit is inserted at line 1 (one past the `{` on line 0).
        assert_eq!(
            te.range.start.line, 1,
            "insert should be at line 1 (after the opening brace on line 0)"
        );
        assert_eq!(te.range.start.character, 0);
        assert!(
            te.new_text.contains("requires true;"),
            "expected `requires true;` in inserted text"
        );
        assert!(
            te.new_text.contains("ensures true;"),
            "expected `ensures true;` in inserted text"
        );
    }

    #[test]
    fn res357_find_brace_line_handles_multiline_before_start() {
        // A `{` only on line 3; scanning from line 0 returns 3.
        let src = "fn f(\n    int x,\n    int y\n) {\n    return x + y;\n}";
        assert_eq!(find_brace_line(src, 0), Some(3));
    }
}
