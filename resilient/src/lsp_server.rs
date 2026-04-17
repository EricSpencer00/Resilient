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

use tower_lsp::jsonrpc::Result as JsonResult;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    InitializeParams, InitializeResult, InitializedParams, MessageType, Position, Range,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::{parse, typechecker};

/// The LSP backend. Holds a `Client` handle for publishing diagnostics.
pub struct Backend {
    client: Client,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// RES-074: analyze `text` for parser + typechecker errors and
    /// publish them as LSP diagnostics against `uri`. Called from
    /// both `did_open` and `did_change`.
    async fn publish_analysis(&self, uri: Url, text: String) {
        let mut diagnostics = Vec::new();

        // Step 1: parse. Each parser error string is published as a
        // diagnostic at line 0 since the hand-rolled parser doesn't
        // expose its error positions as structured data yet. Good
        // enough to prove the pipeline; follow-up can thread them.
        let (program, parser_errors) = parse(&text);
        for err in &parser_errors {
            diagnostics.push(Diagnostic {
                range: point_range(0, 0),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("resilient-parser".into()),
                message: err.clone(),
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

/// Parse the RES-080 `<path>:<line>:<col>: <rest>` prefix out of an
/// error string. Returns a zero-width `Range` at that position plus
/// the remaining message (or a zero-zero range + the original
/// message if parsing fails).
fn extract_range_and_message(err: &str) -> (Range, String) {
    // Find the FIRST colon after the last `/` or whitespace (so we
    // don't split on drive letters on Windows). Cheap heuristic:
    // find the first `:` whose remainder parses as `<uint>:<uint>:`.
    for (i, _) in err.match_indices(':') {
        let rest = &err[i + 1..];
        // rest should start with "LINE:COL: MESSAGE"
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

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> JsonResult<InitializeResult> {
        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
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
    fn extract_handles_path_with_colons() {
        // A Windows-style path wouldn't be common on Unix but let's
        // confirm we don't crash if a path happens to include `:`.
        let err = "C:/tmp/foo.rs:2:3: oops";
        let (range, msg) = extract_range_and_message(err);
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 2);
        assert_eq!(msg, "oops");
    }
}
