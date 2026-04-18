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
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, InitializeParams,
    InitializeResult, InitializedParams, Location, MessageType, OneOf, Position, Range,
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensParams, SemanticTokensResult, SemanticTokensServerCapabilities,
    ServerCapabilities, SymbolInformation, SymbolKind, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url, WorkDoneProgressOptions, WorkspaceSymbolParams,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::{compute_semantic_tokens, parse, typechecker, Node};

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
            Node::Function { name, .. } => {
                make_symbol(name, SymbolKind::FUNCTION, spanned.span)
            }
            Node::StructDecl { name, .. } => {
                make_symbol(name, SymbolKind::STRUCT, spanned.span)
            }
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
        let mut new_index: HashMap<Url, Vec<WorkspaceSymbolEntry>> =
            HashMap::new();
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
    let Ok(entries) = std::fs::read_dir(root) else { return out };
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
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
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
    SemanticTokensLegend { token_types, token_modifiers }
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
        if let (Ok(mut slot), Some(p)) =
            (self.workspace_root.lock(), root_path)
        {
            *slot = Some(p);
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
    async fn did_save(
        &self,
        params: tower_lsp::lsp_types::DidSaveTextDocumentParams,
    ) {
        let uri = params.text_document.uri;
        // Re-read the file from disk. `did_save` notifications
        // carry the text only when the client opts into the
        // `TextDocumentSyncSaveOptions { include_text: true }`
        // — we registered TextDocumentSyncKind::FULL without
        // that, so walk to disk instead.
        let Some(path) = uri.to_file_path().ok() else { return };
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
        let path = std::env::temp_dir().join(format!(
            "res_186_{}_{}_{}",
            tag,
            std::process::id(),
            n
        ));
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
        write_file(
            &root.join(".cache"),
            "d.rs",
            "fn d() { return 0; }\n",
        );
        let found = walk_resilient_files(&root);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"a.rs".to_string()));
        assert!(names.contains(&"b.rs".to_string()));
        assert!(!names.contains(&"c.rs".to_string()), "target/ must be skipped");
        assert!(!names.contains(&"d.rs".to_string()), "dot-dirs must be skipped");
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
                range: Range::new(
                    Position::new(0, 0),
                    Position::new(0, 0),
                ),
            },
            WorkspaceSymbolEntry {
                name: "AlphaBeta".into(),
                kind: SymbolKind::FUNCTION,
                uri: uri.clone(),
                range: Range::new(
                    Position::new(1, 0),
                    Position::new(1, 0),
                ),
            },
            WorkspaceSymbolEntry {
                name: "gamma".into(),
                kind: SymbolKind::FUNCTION,
                uri: uri.clone(),
                range: Range::new(
                    Position::new(2, 0),
                    Position::new(2, 0),
                ),
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
        assert_eq!(legend.token_types[sem_tok::KEYWORD as usize],
                   SemanticTokenType::KEYWORD);
        assert_eq!(legend.token_types[sem_tok::FUNCTION as usize],
                   SemanticTokenType::FUNCTION);
        assert_eq!(legend.token_types[sem_tok::VARIABLE as usize],
                   SemanticTokenType::VARIABLE);
        assert_eq!(legend.token_types[sem_tok::PARAMETER as usize],
                   SemanticTokenType::PARAMETER);
        assert_eq!(legend.token_types[sem_tok::TYPE as usize],
                   SemanticTokenType::TYPE);
        assert_eq!(legend.token_types[sem_tok::STRING as usize],
                   SemanticTokenType::STRING);
        assert_eq!(legend.token_types[sem_tok::NUMBER as usize],
                   SemanticTokenType::NUMBER);
        assert_eq!(legend.token_types[sem_tok::COMMENT as usize],
                   SemanticTokenType::COMMENT);
        assert_eq!(legend.token_types[sem_tok::OPERATOR as usize],
                   SemanticTokenType::OPERATOR);
        // Modifier bit positions: bit 0 = declaration, bit 1 = readonly.
        assert_eq!(legend.token_modifiers[0], SemanticTokenModifier::DECLARATION);
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

    #[test]
    fn workspace_index_spans_multiple_files() {
        // Ticket AC: pre-seed two files, invoke the query (via the
        // helper path), assert both files' symbols are returned.
        let root = tmp_workspace("multifile");
        write_file(&root, "mod_a.rs", "fn a_fn() { return 0; }\nstruct A_Struct { int x }\n");
        write_file(&root, "mod_b.rs", "fn b_fn() { return 0; }\n");

        // Walk + index the whole scratch dir, reproducing what
        // `rebuild_workspace_index` does when the Backend is
        // invoked via the LSP.
        let files = walk_resilient_files(&root);
        assert_eq!(files.len(), 2);
        let mut index: std::collections::HashMap<Url, Vec<WorkspaceSymbolEntry>> =
            std::collections::HashMap::new();
        for p in files {
            let Ok(uri) = Url::from_file_path(&p) else { continue };
            if let Some(entries) = index_file(&p) {
                index.insert(uri, entries);
            }
        }

        // All-match query: three names across two files.
        let r = filter_workspace_symbols(&index, "", 50);
        let names: std::collections::HashSet<&str> =
            r.iter().map(|s| s.name.as_str()).collect();
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
}
